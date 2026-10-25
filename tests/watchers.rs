use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

struct Environment(TempDir);
impl Environment {
    fn new(config: &str) -> Self {
        let directory = TempDir::new().unwrap();
        fs::write(directory.path().join("parcela.yaml"), config).unwrap();
        Self(directory)
    }
    fn path(&self) -> &Path {
        self.0.path()
    }
    fn run(&self, args: &[&str]) -> Output {
        self.run_at(self.path(), args)
    }
    fn run_at(&self, path: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_parcela"))
            .args(["--json", "--path"])
            .arg(path)
            .arg("--state-dir")
            .arg(self.path().join("state"))
            .env("PARCELA_NO_TRUST", "1")
            .env("PORT", "12345")
            .env("PARCELA_URL", "https://foreign.localhost")
            .args(args)
            .output()
            .unwrap()
    }
    fn success(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        let _ = self.run(&["down"]);
        let _ = self.run_at(&self.path().join("web"), &["down"]);
        let _ = self.run(&["proxy", "stop"]);
    }
}

const WATCHER: &str = "version: 1\nproject: watcher\nreadiness_timeout_seconds: 3\nservices:\n  build:\n    command: 'echo watching; touch ready; exec sleep 300'\n    healthcheck: test -f ready\ntasks:\n  environment:\n    command: printf '%s' \"${PARCELA_URL-unset}:${PORT-unset}:${PARCELA_HOSTNAME-unset}\"\n";

#[test]
fn watcher_lifecycle_needs_no_proxy_ports_or_url() {
    let env = Environment::new(WATCHER);
    let doctor = env.success(&["doctor", "--caddy", "/nonexistent/caddy"]);
    assert_eq!(doctor["ready"], true);
    assert!(
        !env.path().join("state").exists(),
        "doctor must remain read-only"
    );
    let first = env.success(&["up", "--detach"]);
    assert_eq!(first["live"]["state"], "ready");
    assert_eq!(first["live"]["router_owned"], false);
    assert_eq!(first["live"]["route_published"], false);
    assert!(first["session"]["public_url"].is_null());
    assert!(first["session"]["backend_port"].is_null());
    assert!(first["session"]["router_token"].is_null());
    assert_eq!(first["session"]["endpoints"], serde_json::json!([]));
    assert_eq!(first["session"]["ports"], serde_json::json!({}));
    assert!(!env.path().join("state/proxy").exists());
    let status = env.success(&["status"]);
    assert_eq!(status["state"], "ready");
    assert!(status["url"].is_null());
    assert_eq!(status["processes"][0]["name"], "build");
    let context = env.success(&["context"]);
    assert_eq!(context["planned_urls"], serde_json::json!({}));
    let url = env.run(&["url"]);
    assert!(!url.status.success());
    assert!(String::from_utf8_lossy(&url.stderr).contains("no HTTP services or URL"));
    let task = env.run(&["task", "environment"]);
    assert!(
        task.status.success(),
        "{}",
        String::from_utf8_lossy(&task.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&task.stdout), "unset:unset:unset");
    let logs = env.run(&["logs", "build"]);
    assert!(logs.status.success());
    assert!(String::from_utf8_lossy(&logs.stdout).contains("watching"));
    let restarted = env.success(&["restart", "--detach"]);
    assert!(restarted["session"]["generation"].as_i64() > first["session"]["generation"].as_i64());
    assert_eq!(env.success(&["up", "--detach"])["already_running"], true);
    assert_eq!(env.success(&["down"])["state"], "stopped");
    assert_eq!(env.success(&["status"])["state"], "stopped");
    assert!(!env.path().join("state/proxy").exists());
}

#[test]
fn watcher_readiness_waits_for_hooks_and_command_healthchecks() {
    let env = Environment::new(
        "version: 1\nproject: checks\nreadiness_timeout_seconds: 1\nhooks:\n  prestart:\n    command: sleep 0.2; touch prepared\nservices:\n  build:\n    command: exec sleep 300\n    healthcheck: test -f missing\n",
    );
    let output = env.run(&["up", "--detach"]);
    assert!(!output.status.success());
    assert!(env.path().join("prepared").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("healthcheck"));
    assert!(!env.path().join("state/proxy").exists());
}

#[test]
fn watcher_only_environments_reject_http_flags_and_literal_run() {
    for args in [
        vec!["up", "--detach", "--backend-port", "31010"],
        vec!["up", "--detach", "--lan"],
        vec!["up", "--detach", "--lan-ip", "192.0.2.1"],
        vec!["up", "--detach", "--tailscale"],
        vec!["run", "--", "sleep", "1"],
    ] {
        let env = Environment::new(WATCHER);
        let output = env.run(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("HTTP service"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!env.path().join("state/proxy").exists());
    }
}

#[test]
fn watcher_named_ports_are_explicit_and_do_not_create_http_routes() {
    let env = Environment::new(&WATCHER.replace("services:", "ports: [reload]\nservices:"));
    let started = env.success(&["up", "--detach"]);
    assert!(started["session"]["ports"]["reload"].as_u64().unwrap() >= 20000);
    assert!(started["session"]["backend_port"].is_null());
    assert!(!env.path().join("state/proxy").exists());
}

#[test]
fn watcher_owner_recovery_adopts_the_same_process_without_a_proxy() {
    let env = Environment::new(WATCHER);
    let started = env.success(&["up", "--detach"]);
    let owner = started["live"]["owner_pid"].as_u64().unwrap() as u32;
    let process = env.success(&["status"])["processes"][0]["pid"]
        .as_u64()
        .unwrap();
    let identity = parcela::process::Identity::read(owner).unwrap().unwrap();
    parcela::process::signal_verified(&identity, libc::SIGKILL, false).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while identity.alive() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let recovered = env.success(&["up", "--detach"]);
    assert_eq!(recovered["session"]["token"], started["session"]["token"]);
    assert_eq!(env.success(&["status"])["processes"][0]["pid"], process);
    assert!(!env.path().join("state/proxy").exists());
}

#[test]
fn foreground_watcher_exit_propagates_without_starting_the_proxy() {
    let env =
        Environment::new("version: 1\nproject: exit\nservices:\n  build: {command: 'exit 7'}\n");
    let output = env.run(&["up"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("7"));
    assert!(!env.path().join("state/proxy").exists());
}

#[test]
fn watcher_sessions_coexist_with_http_and_refuse_an_older_running_proxy() {
    if parcela::process::on_path("caddy").is_none() {
        return;
    }
    let env = Environment::new(WATCHER);
    let web = env.path().join("web");
    fs::create_dir(&web).unwrap();
    fs::write(web.join("parcela.yaml"), "version: 1\nproject: web\nservices:\n  web:\n    command: python3 -m http.server $PORT --bind 127.0.0.1\n    http: {healthcheck: /}\n").unwrap();
    let ports: Vec<_> = (30000..32767)
        .filter_map(|port| std::net::TcpListener::bind(("127.0.0.1", port)).ok())
        .take(2)
        .collect();
    let proxy_port = ports[0].local_addr().unwrap().port().to_string();
    let web_port = ports[1].local_addr().unwrap().port().to_string();
    drop(ports);
    env.success(&["proxy", "start", "--port", &proxy_port]);
    let started = env.run_at(&web, &["up", "--detach", "--backend-port", &web_port]);
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let metadata = env.path().join("state/proxy/router.json");
    let original = fs::read(&metadata).unwrap();
    let mut older: Value = serde_json::from_slice(&original).unwrap();
    older["protocol"] = serde_json::json!(parcela::router::PROTOCOL - 1);
    fs::write(&metadata, serde_json::to_vec(&older).unwrap()).unwrap();
    let refused = env.run(&["up", "--detach"]);
    fs::write(&metadata, &original).unwrap();
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("running proxy cannot read watcher-only sessions")
    );
    older["token"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
    fs::write(&metadata, serde_json::to_vec(&older).unwrap()).unwrap();
    let unverified = env.run(&["up", "--detach"]);
    fs::write(&metadata, &original).unwrap();
    assert!(
        !unverified.status.success(),
        "a live proxy with unverifiable ownership must not be treated as absent"
    );
    assert!(env.success(&["status"])["recorded_session"].is_null());
    env.success(&["up", "--detach"]);
    let status = env.run_at(&web, &["status"]);
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["state"], "ready");
    assert_eq!(status["live"]["route_published"], true);
    env.success(&["down"]);
    let status: Value = serde_json::from_slice(&env.run_at(&web, &["status"]).stdout).unwrap();
    assert_eq!(status["state"], "ready");
}

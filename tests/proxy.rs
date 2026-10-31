//! The background proxy, driven through the CLI against real Caddy on explicit high ports.
use serde_json::{Value, json};
use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

fn caddy() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join("caddy"))
        .find(|candidate| candidate.is_file())
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn cli(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(state)
        .env_remove("PARCELA_HTTPS_PORT")
        .args(args)
        .output()
        .unwrap()
}

fn json_of(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn alive(pid: u64) -> bool {
    // SAFETY: signal 0 only checks that the PID exists.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Stops whatever proxy the test started, even when an assertion fails.
struct Proxy {
    _t: TempDir,
    state: PathBuf,
}
impl Proxy {
    fn new() -> Self {
        let t = TempDir::new().unwrap();
        let state = t.path().join("state");
        Self { _t: t, state }
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "fixture proxy log: {}",
                fs::read_to_string(self.state.join("proxy/proxy.log")).unwrap_or_default()
            );
        }
        cli(&self.state, &["proxy", "stop"]);
    }
}

#[test]
fn the_saved_profile_beats_the_default_and_parcela_https_port_only_replaces_the_default() {
    let t = TempDir::new().unwrap();
    let state = t.path().join("state");
    let status = cli(&state, &["--json", "proxy", "status"]);
    assert_eq!(status.status.code(), Some(1));
    assert_eq!(json_of(&status)["front_door"], json!({"https": 1443}));
    let overridden = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(&state)
        .env("PARCELA_HTTPS_PORT", "4433")
        .args(["--json", "proxy", "status"])
        .output()
        .unwrap();
    assert_eq!(json_of(&overridden)["url"], "https://<name>.localhost:4433");
    assert!(!state.exists(), "status creates no state");
    let print = cli(&state, &["trust", "--print"]);
    assert_eq!(print.status.code(), Some(1));
    assert!(stderr(&print).ends_with("start the proxy with `parcela proxy start`\n"));
    assert!(!state.exists(), "trust --print creates no state");

    parcela::frontdoor::FrontDoor::on(2000)
        .save(&state)
        .unwrap();
    let saved = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(&state)
        .env("PARCELA_HTTPS_PORT", "4433")
        .args(["--json", "proxy", "status"])
        .output()
        .unwrap();
    assert_eq!(json_of(&saved)["url"], "https://<name>.localhost:2000");
}

#[test]
fn nonterminal_trust_refuses_before_creating_state_or_starting_a_backend() {
    let proxy = Proxy::new();
    let output = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(&proxy.state)
        .args(["trust", "--caddy", "/not-a-trust-backend"])
        .env("PATH", "")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("requires a terminal"),
        "{}",
        stderr(&output)
    );
    assert!(!proxy.state.exists(), "refused trust must not create state");
}

#[test]
fn proxy_start_runs_in_the_background_until_proxy_stop() {
    let Some(caddy) = caddy() else {
        eprintln!("skipped: caddy is not on PATH");
        return;
    };
    let proxy = Proxy::new();
    let port = free_port();
    let caddy = caddy.to_str().unwrap();
    let started = cli(
        &proxy.state,
        &[
            "--json",
            "proxy",
            "start",
            "--port",
            &port.to_string(),
            "--caddy",
            caddy,
        ],
    );
    assert!(started.status.success(), "{}", stderr(&started));
    let started = json_of(&started);
    assert_eq!(started["running"], true);
    assert_eq!(started["already_running"], false);
    assert_eq!(started["front_door"], json!({"https": port}));
    assert_eq!(started["url"], format!("https://<name>.localhost:{port}"));
    assert_eq!(started["version"], env!("PARCELA_VERSION"));
    let pid = started["pid"].as_u64().unwrap();
    let caddy_pid = started["caddy_pid"].as_u64().unwrap();
    assert!(alive(pid) && alive(caddy_pid));
    assert_eq!(
        fs::read_to_string(proxy.state.join("proxy/frontdoor.json"))
            .map(|text| serde_json::from_str::<Value>(&text).unwrap())
            .unwrap(),
        json!({"https": port})
    );
    assert!(
        TcpListener::bind(("127.0.0.1", port)).is_err(),
        "Caddy listens on the chosen port"
    );

    let status = cli(&proxy.state, &["--json", "proxy", "status"]);
    assert!(status.status.success());
    assert_eq!(json_of(&status)["pid"], pid);

    let print = cli(&proxy.state, &["trust", "--print"]);
    assert!(print.status.success(), "{}", stderr(&print));
    let certificate = proxy
        .state
        .canonicalize()
        .unwrap()
        .join("proxy/data/pki/authorities/local/root.crt");
    assert_eq!(
        String::from_utf8_lossy(&print.stdout),
        format!(
            "{}\n",
            proxy
                .state
                .join("proxy/data/pki/authorities/local/root.crt")
                .display()
        )
    );
    assert!(certificate.is_file());

    let again = json_of(&cli(&proxy.state, &["--json", "proxy", "start"]));
    assert_eq!(again["already_running"], true);
    assert_eq!(again["pid"], pid);

    let other = free_port();
    let conflict = cli(
        &proxy.state,
        &["proxy", "start", "--port", &other.to_string()],
    );
    assert!(!conflict.status.success());
    assert_eq!(
        stderr(&conflict),
        format!(
            "parcela: the proxy is already running with HTTPS on :{port}; switch with `parcela proxy restart --port {other}`\n"
        )
    );

    let stopped = json_of(&cli(&proxy.state, &["--json", "proxy", "stop"]));
    assert_eq!(stopped["stopped"], true);
    assert!(!alive(pid) && !alive(caddy_pid));
    assert_eq!(
        cli(&proxy.state, &["proxy", "status"]).status.code(),
        Some(1)
    );
    assert_eq!(
        json_of(&cli(&proxy.state, &["--json", "proxy", "stop"]))["stopped"],
        false
    );
}

#[test]
fn a_busy_port_fails_naming_its_holder_and_keeps_the_saved_profile() {
    let Some(caddy) = caddy() else {
        eprintln!("skipped: caddy is not on PATH");
        return;
    };
    let proxy = Proxy::new();
    let foreign = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = foreign.local_addr().unwrap().port();
    let failed = cli(
        &proxy.state,
        &[
            "proxy",
            "start",
            "--port",
            &port.to_string(),
            "--caddy",
            caddy.to_str().unwrap(),
        ],
    );
    assert_eq!(failed.status.code(), Some(1));
    let message = stderr(&failed);
    assert!(
        message.starts_with(&format!(
            "parcela: the proxy did not start: port {port} is already in use by "
        )) && message.ends_with("; choose another port with `parcela proxy start --port <n>`\n"),
        "{message}"
    );
    assert!(
        message.contains(&format!("(pid {})", std::process::id())),
        "{message}"
    );
    assert!(!proxy.state.join("proxy/frontdoor.json").exists());
    assert_eq!(foreign.local_addr().unwrap().port(), port);
}

#[test]
fn only_a_proxy_on_another_protocol_warns() {
    let Some(caddy) = caddy() else {
        eprintln!("skipped: caddy is not on PATH");
        return;
    };
    let proxy = Proxy::new();
    let port = free_port().to_string();
    let caddy = caddy.to_str().unwrap();
    let started = cli(
        &proxy.state,
        &["proxy", "start", "--port", &port, "--caddy", caddy],
    );
    assert!(started.status.success(), "{}", stderr(&started));
    let record = proxy.state.join("proxy/router.json");
    let rewrite = |version: &str, protocol: Option<u32>| {
        let mut router: Value =
            serde_json::from_str(&fs::read_to_string(&record).unwrap()).unwrap();
        router["version"] = json!(version);
        match protocol {
            Some(protocol) => router["protocol"] = json!(protocol),
            None => drop(router.as_object_mut().unwrap().remove("protocol")),
        }
        parcela::private::write_json(&record, &router).unwrap();
    };
    let status = || json_of(&cli(&proxy.state, &["--json", "proxy", "status"]));

    // Another release on the same protocol shares the proxy quietly.
    rewrite("0.0.2", Some(parcela::router::PROTOCOL));
    assert_eq!(status()["version_warning"], Value::Null);

    rewrite("0.0.3", Some(parcela::router::PROTOCOL + 1));
    assert_eq!(
        status()["version_warning"],
        format!(
            "the running proxy is from Parcela 0.0.3, newer than this {}; run `parcela upgrade`",
            env!("PARCELA_VERSION")
        )
    );

    rewrite("0.0.1", Some(parcela::router::PROTOCOL - 1));
    let warning = format!(
        "the running proxy is from Parcela 0.0.1, older than this {}; run `parcela proxy restart` to update it",
        env!("PARCELA_VERSION")
    );
    let before = status();
    assert_eq!(before["version_warning"], warning);
    let start = cli(&proxy.state, &["--json", "proxy", "start"]);
    assert!(start.status.success());
    assert_eq!(stderr(&start), format!("parcela: {warning}\n"));

    // A record this Parcela cannot read still restarts on the same port.
    rewrite("0.0.0", None);
    assert!(
        status()["error"]
            .as_str()
            .unwrap()
            .ends_with("is from Parcela \"0.0.0\"; run `parcela proxy restart`")
    );

    let restarted = cli(
        &proxy.state,
        &["--json", "proxy", "restart", "--caddy", caddy],
    );
    assert!(restarted.status.success(), "{}", stderr(&restarted));
    let restarted = json_of(&restarted);
    assert_eq!(restarted["version"], env!("PARCELA_VERSION"));
    assert_eq!(restarted["version_warning"], Value::Null);
    assert_ne!(restarted["pid"], before["pid"]);
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(program).is_file())
    })
}

/// Stops the environments before the proxy guard runs.
struct Environments<'a> {
    state: &'a Path,
    paths: Vec<PathBuf>,
}
impl Drop for Environments<'_> {
    fn drop(&mut self) {
        for path in &self.paths {
            Command::new(env!("CARGO_BIN_EXE_parcela"))
                .arg("--path")
                .arg(path)
                .arg("--state-dir")
                .arg(self.state)
                .arg("down")
                .output()
                .unwrap();
        }
    }
}

#[test]
fn parallel_ups_in_two_checkouts_start_one_proxy_and_both_urls_answer() {
    if !["caddy", "curl", "python3"]
        .iter()
        .all(|program| on_path(program))
    {
        eprintln!("skipped: needs caddy, curl and python3 on PATH");
        return;
    }
    let proxy = Proxy::new();
    let root = proxy._t.path().canonicalize().unwrap();
    let port = free_port();
    let manifest = "version: 1\nproject: notes\nservices: {web: {command: python3 -m http.server $PORT --bind 127.0.0.1, http: {healthcheck: /}}}\n";
    let primary = root.join("notes");
    let preview = root.join("notes-preview");
    for path in [&primary, &preview] {
        fs::create_dir(path).unwrap();
        fs::write(path.join("parcela.yaml"), manifest).unwrap();
    }
    let environments = Environments {
        state: &proxy.state,
        paths: vec![primary.clone(), preview.clone()],
    };
    let up = |path: &Path, extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_parcela"))
            .arg("--path")
            .arg(path)
            .arg("--state-dir")
            .arg(&proxy.state)
            .env("PARCELA_HTTPS_PORT", port.to_string())
            .args(["--json", "up", "--detach"])
            .args(extra)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let first = up(&primary, &[]);
    let second = up(&preview, &["--name", "preview"]);
    let outputs = [
        first.wait_with_output().unwrap(),
        second.wait_with_output().unwrap(),
    ];
    for output in &outputs {
        assert!(output.status.success(), "{}", stderr(output));
    }
    let hint = format!("started the proxy on :{port}; run `parcela trust` to trust https://");
    assert_eq!(
        outputs
            .iter()
            .filter(|output| stderr(output).contains(&hint))
            .count(),
        1,
        "exactly one up starts the proxy"
    );
    let urls: Vec<String> = outputs
        .iter()
        .map(|output| {
            json_of(output)["session"]["public_url"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        urls,
        [
            format!("https://notes.localhost:{port}"),
            format!("https://preview-notes.localhost:{port}")
        ]
    );
    let daemons = Command::new("pgrep")
        .args([
            "-f",
            &format!(
                "{} proxy run",
                proxy.state.canonicalize().unwrap().display()
            ),
        ])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&daemons.stdout).lines().count(), 1);
    for url in &urls {
        let host = url
            .trim_start_matches("https://")
            .split(':')
            .next()
            .unwrap();
        let curl = Command::new("curl")
            .args([
                "--silent",
                "--insecure",
                "--max-time",
                "5",
                "--resolve",
                &format!("{host}:{port}:127.0.0.1"),
                "--output",
                "/dev/null",
                "--write-out",
                "%{http_code}",
                url,
            ])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&curl.stdout), "200", "{url}");
    }
    drop(environments);
}

#[test]
fn domain_names_answer_on_loopback_http_for_a_front_proxy_until_down() {
    if !["caddy", "curl", "python3"]
        .iter()
        .all(|program| on_path(program))
    {
        eprintln!("skipped: needs caddy, curl and python3 on PATH");
        return;
    }
    let proxy = Proxy::new();
    let checkout = proxy._t.path().canonicalize().unwrap().join("notes");
    fs::create_dir(&checkout).unwrap();
    // Answers with the forwarded headers and the hosts Parcela told it to allow.
    fs::write(
        checkout.join("echo.py"),
        "import http.server, os, sys\n\
         class H(http.server.BaseHTTPRequestHandler):\n\
         \x20   def do_GET(self):\n\
         \x20       body = '|'.join([self.headers.get('X-Forwarded-Proto', ''), self.headers.get('X-Forwarded-Host', ''), os.environ['PARCELA_SHARE_HOSTS']]).encode()\n\
         \x20       self.send_response(200); self.end_headers(); self.wfile.write(body)\n\
         http.server.HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()\n",
    )
    .unwrap();
    fs::write(
        checkout.join("parcela.yaml"),
        "version: 1\nproject: notes\nservices: {web: {command: python3 echo.py $PORT, http: {healthcheck: /}}}\n",
    )
    .unwrap();
    let environments = Environments {
        state: &proxy.state,
        paths: vec![checkout.clone()],
    };
    let https = free_port();
    let domain_port = free_port();
    let up = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--path")
        .arg(&checkout)
        .arg("--state-dir")
        .arg(&proxy.state)
        .env("PARCELA_HTTPS_PORT", https.to_string())
        .args(["--json", "up", "--detach", "--domain", "Dev.Example.com"])
        .args(["--domain-port", &domain_port.to_string()])
        .output()
        .unwrap();
    assert!(up.status.success(), "{}", stderr(&up));
    let status = json_of(&cli(
        &proxy.state,
        &["--path", checkout.to_str().unwrap(), "status", "--current"],
    ));
    assert_eq!(
        status["live"]["endpoints"]["web"]["shared"],
        json!(["https://notes.dev.example.com"])
    );
    let get = |host: &str| {
        let curl = Command::new("curl")
            .args([
                "--silent",
                "--max-time",
                "5",
                "--write-out",
                "|%{http_code}",
            ])
            .args(["--header", &format!("Host: {host}")])
            .args(["--header", "X-Forwarded-Proto: https"])
            .arg(format!("http://127.0.0.1:{domain_port}/"))
            .output()
            .unwrap();
        String::from_utf8_lossy(&curl.stdout).into_owned()
    };
    assert_eq!(
        get("notes.dev.example.com"),
        "https|notes.dev.example.com|notes.dev.example.com|200"
    );
    assert!(get("other.dev.example.com").ends_with("|404"));
    drop(environments);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::net::TcpStream::connect(("127.0.0.1", domain_port)).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "domain listener outlived down"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[test]
fn urls_a_process_hands_to_browser_are_offered_on_loopback_names_and_pages_act_unshared() {
    if !["caddy", "curl", "python3"]
        .iter()
        .all(|program| on_path(program))
    {
        eprintln!("skipped: needs caddy, curl and python3 on PATH");
        return;
    }
    let proxy = Proxy::new();
    let checkout = proxy._t.path().canonicalize().unwrap().join("notes");
    fs::create_dir(&checkout).unwrap();
    let https = free_port();
    let domain_port = free_port();
    fs::write(
        checkout.join("mailer.py"),
        "import http.server, os, subprocess, sys\n\
         class H(http.server.BaseHTTPRequestHandler):\n\
         \x20   def do_GET(self):\n\
         \x20       if self.path == '/send':\n\
         \x20           os.makedirs('mail', exist_ok=True)\n\
         \x20           open('mail/rich.html', 'w').write('<title>\\n Your sign-in link </title><a href=\"plain.html\">plain</a>')\n\
         \x20           open('mail/plain.html', 'w').write('plain text')\n\
         \x20           subprocess.run([os.environ['BROWSER'], 'file://' + os.path.abspath('mail/rich.html'), os.environ['PARCELA_URL'] + '/welcome?x=1'], check=True)\n\
         \x20       self.send_response(200); self.end_headers(); self.wfile.write(b'sent')\n\
         http.server.HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()\n",
    )
    .unwrap();
    fs::write(checkout.join("secret"), "secret").unwrap();
    fs::write(
        checkout.join("parcela.yaml"),
        "version: 1\nproject: notes\nservices: {web: {command: python3 mailer.py $PORT, http: {healthcheck: /}}}\n",
    )
    .unwrap();
    let environments = Environments {
        state: &proxy.state,
        paths: vec![checkout.clone()],
    };
    // Sharing through the environment, so the proxy this run starts is spawned with it too.
    let up = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--path")
        .arg(&checkout)
        .arg("--state-dir")
        .arg(&proxy.state)
        .env("PARCELA_HTTPS_PORT", https.to_string())
        .env("PARCELA_DOMAIN", "dev.example.com")
        .env("PARCELA_DOMAIN_PORT", domain_port.to_string())
        .args(["--json", "up", "--detach"])
        .output()
        .unwrap();
    assert!(up.status.success(), "{}", stderr(&up));
    let local = |method: &str, path: &str, headers: &[&str]| {
        let mut curl = Command::new("curl");
        curl.args(["--silent", "--insecure", "--max-time", "5", "--include"])
            .args(["--request", method])
            .args(["--resolve", &format!("notes.localhost:{https}:127.0.0.1")])
            .args([
                "--resolve",
                &format!("parcela-ui.localhost:{https}:127.0.0.1"),
            ]);
        for header in headers {
            curl.args(["--header", header]);
        }
        String::from_utf8_lossy(&curl.arg(path).output().unwrap().stdout).into_owned()
    };
    let shared = |method: &str, path: &str, headers: &[&str]| {
        let mut curl = Command::new("curl");
        curl.args(["--silent", "--max-time", "5", "--include"])
            .args(["--request", method])
            .args(["--header", "Host: notes.dev.example.com"]);
        for header in headers {
            curl.args(["--header", header]);
        }
        let output = curl
            .arg(format!("http://127.0.0.1:{domain_port}{path}"))
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let body = |response: String| {
        response
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .to_owned()
    };
    let app = format!("https://notes.localhost:{https}");
    let get = |path: &str| local("GET", &format!("{app}{path}"), &[]);
    assert!(get("/send").ends_with("sent"));

    let opens: Value = serde_json::from_str(&body(get("/_parcela/opens"))).unwrap();
    let opens = opens["opens"].as_array().unwrap();
    assert_eq!(opens.len(), 2);
    let file = opens[0]["href"].as_str().unwrap().to_owned();
    assert_eq!(opens[0]["label"], "Your sign-in link");
    assert_eq!(
        file,
        format!(
            "/_parcela/files/{}/rich.html",
            opens[0]["id"].as_str().unwrap()
        )
    );
    assert_eq!(
        opens[1]["href"], "/welcome?x=1",
        "a URL on its own host becomes a path"
    );
    let page = get(&file);
    assert!(page.starts_with("HTTP/2 200"), "{page}");
    assert!(
        page.contains("content-security-policy: sandbox "),
        "opened files never run scripts on the app's origin: {page}"
    );
    assert!(page.ends_with("<a href=\"plain.html\">plain</a>"));
    let directory = file.trim_end_matches("rich.html");
    fs::write(checkout.join("mail/.env"), "TOKEN=1").unwrap();
    fs::write(checkout.join("mail/notes.yaml"), "token: 1").unwrap();
    assert!(get(&format!("{directory}plain.html")).ends_with("plain text"));
    for refused in ["..%2Fsecret", ".env", "notes.yaml"] {
        let response = get(&format!("{directory}{refused}"));
        assert!(response.starts_with("HTTP/2 404"), "{refused}: {response}");
    }
    let menu = get("/_parcela/");
    assert!(
        menu.starts_with("HTTP/2 200")
            && menu.contains("<script src=\"/_parcela/widget.js\" data-page>"),
        "{menu}"
    );

    // LAN and domain names never reach Parcela's paths: the app answers them, here with `sent`.
    assert!(
        shared("GET", "/", &[]).ends_with("sent"),
        "the run is shared"
    );
    let environment = json_of(&cli(
        &proxy.state,
        &["--path", checkout.to_str().unwrap(), "status", "--current"],
    ))["workspace"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let restart = format!("/_parcela/environments/{environment}/restart");
    for path in [
        "/_parcela/opens".to_owned(),
        file.clone(),
        "/_parcela/environments".to_owned(),
        format!("/_parcela/logs?environment={environment}"),
    ] {
        let response = shared("GET", &path, &["X-Parcela-Access: local"]);
        assert!(response.ends_with("sent"), "{path}: {response}");
    }
    let refused = shared(
        "POST",
        &restart,
        &[
            "X-Parcela-Action: 1",
            "Origin: https://notes.dev.example.com",
            "X-Parcela-Access: local",
        ],
    );
    assert!(!refused.contains("accepted"), "{refused}");

    let spoofed = local(
        "GET",
        &format!("https://parcela-ui.localhost:{https}/_parcela/opens"),
        &[
            &format!("X-Parcela-Environment: {environment}"),
            "X-Parcela-Hostname: notes.localhost",
            "X-Parcela-Access: local",
        ],
    );
    assert!(
        spoofed.starts_with("HTTP/2 404"),
        "the dashboard name never answers for an environment: {spoofed}"
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let listed = loop {
        let listed: Value = serde_json::from_str(&body(get("/_parcela/environments"))).unwrap();
        if listed["environments"][0]["url"].is_string() {
            break listed;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the local page never listed the environment: {listed}"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let this = &listed["environments"][0];
    assert_eq!(this["current"], true);
    assert_eq!(this["key"], environment.as_str());
    assert_eq!(this["url"], app.as_str());
    assert!(
        this.get("source").is_none(),
        "pages never see checkout paths"
    );
    let logs: Value = serde_json::from_str(&body(get(&format!(
        "/_parcela/logs?environment={environment}"
    ))))
    .unwrap();
    assert_eq!(logs["process"], "web");

    let act = |action: &str, headers: &[&str]| {
        local(
            "POST",
            &format!("{app}/_parcela/environments/{environment}/{action}"),
            headers,
        )
    };
    let origin = format!("Origin: {app}");
    assert!(act("restart", &[]).starts_with("HTTP/2 403"));
    assert!(
        act(
            "restart",
            &["X-Parcela-Action: 1", "Origin: https://evil.example"]
        )
        .starts_with("HTTP/2 403")
    );
    let unregistered = local(
        "POST",
        &format!(
            "{app}/_parcela/environments/{}/start",
            checkout.to_str().unwrap().replace('/', "%2F")
        ),
        &["X-Parcela-Action: 1", &origin],
    );
    assert!(
        unregistered.starts_with("HTTP/2 404"),
        "pages never start a checkout by path: {unregistered}"
    );
    let accepted = act("restart", &["X-Parcela-Action: 1", &origin]);
    assert!(accepted.starts_with("HTTP/2 202"), "{accepted}");
    // The page is down while its own environment restarts, so wait on the action through it but
    // explain a timeout with the CLI's view and the owner log.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    loop {
        let response = get("/_parcela/environments");
        let listed: Value = serde_json::from_str(&body(response.clone())).unwrap_or_default();
        let action = &listed["environments"][0]["action"];
        if action["name"] == "restart" && action["state"] == "done" {
            break;
        }
        assert_ne!(action["state"], "failed", "{listed}");
        if std::time::Instant::now() >= deadline {
            let status = cli(
                &proxy.state,
                &["--path", checkout.to_str().unwrap(), "status", "--current"],
            );
            let runtime = proxy.state.join("environments").join(&environment);
            panic!(
                "restart from the page never finished\npage: {response}\nstatus: {}{}\nowner log: {}",
                String::from_utf8_lossy(&status.stdout),
                stderr(&status),
                fs::read_to_string(runtime.join("log/owner.log")).unwrap_or_default()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    assert!(
        get("/").ends_with("sent"),
        "the restarted run answers locally"
    );
    assert!(
        !shared("GET", "/", &[]).ends_with("sent"),
        "a page never shares what it starts, whatever the proxy was started with"
    );

    let accepted = act("stop", &["X-Parcela-Action: 1", &origin]);
    assert!(accepted.starts_with("HTTP/2 202"), "{accepted}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while json_of(&cli(
        &proxy.state,
        &["--path", checkout.to_str().unwrap(), "status", "--current"],
    ))["state"]
        != "stopped"
    {
        assert!(
            std::time::Instant::now() < deadline,
            "stop from the page never stopped the environment"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    drop(environments);
}

#[test]
fn a_port_only_root_may_use_is_refused_with_the_reason() {
    // macOS lets only root listen on loopback ports below 1024.
    if !cfg!(target_os = "macos") || unsafe { libc::getuid() } == 0 {
        return;
    }
    let Some(caddy) = caddy() else {
        eprintln!("skipped: caddy is not on PATH");
        return;
    };
    let proxy = Proxy::new();
    let started = cli(
        &proxy.state,
        &[
            "proxy",
            "start",
            "--port",
            "443",
            "--caddy",
            caddy.to_str().unwrap(),
        ],
    );
    assert_eq!(started.status.code(), Some(1));
    assert!(
        stderr(&started).contains(
            "this system only lets root listen on port 443; Parcela runs without root, so pick a port of 1024 or higher (the default is 1443)"
        ),
        "{}",
        stderr(&started)
    );
}

#[test]
fn open_hands_the_running_url_to_the_browser_and_refuses_a_stopped_one() {
    if !["caddy", "python3"].iter().all(|program| on_path(program)) {
        eprintln!("skipped: needs caddy and python3 on PATH");
        return;
    }
    let proxy = Proxy::new();
    let root = proxy._t.path().canonicalize().unwrap();
    let port = free_port().to_string();
    let app = root.join("notes");
    fs::create_dir(&app).unwrap();
    fs::write(
        app.join("parcela.yaml"),
        "version: 1\nproject: notes\nservices: {web: {command: python3 -m http.server $PORT --bind 127.0.0.1, http: {healthcheck: /}}}\n",
    )
    .unwrap();
    let _environments = Environments {
        state: &proxy.state,
        paths: vec![app.clone()],
    };
    let opened = root.join("opened");
    let browser = root.join("browser");
    fs::write(
        &browser,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n",
            opened.display()
        ),
    )
    .unwrap();
    fs::set_permissions(
        &browser,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let parcela = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_parcela"))
            .arg("--path")
            .arg(&app)
            .arg("--state-dir")
            .arg(&proxy.state)
            .env("PARCELA_HTTPS_PORT", &port)
            .env("BROWSER", format!("{} --new-window", browser.display()))
            .args(args)
            .output()
            .unwrap()
    };
    let up = parcela(&["up", "--detach"]);
    assert!(up.status.success(), "{}", stderr(&up));
    let url = format!("https://notes.localhost:{port}");

    let open = parcela(&["--json", "open"]);
    assert!(open.status.success(), "{}", stderr(&open));
    assert_eq!(json_of(&open), json!({ "url": url }));
    assert_eq!(
        fs::read_to_string(&opened).unwrap(),
        format!("--new-window {url}\n")
    );

    assert!(!parcela(&["open", "nope"]).status.success());
    assert!(parcela(&["down"]).status.success());
    let stopped = parcela(&["open"]);
    assert_eq!(
        stderr(&stopped),
        "parcela: notes is not running; start it with `parcela up`\n"
    );
    assert_eq!(
        fs::read_to_string(&opened).unwrap(),
        format!("--new-window {url}\n"),
        "nothing opened while stopped"
    );
}

#[test]
fn proxy_start_waits_for_a_transient_status_probe_to_release_the_owner_lock() {
    use std::{
        os::unix::fs::PermissionsExt,
        process::Stdio,
        time::{Duration, Instant},
    };
    let Some(caddy) = caddy() else {
        return;
    };
    let proxy = Proxy::new();
    let root = proxy._t.path().canonicalize().unwrap();
    let directory = proxy.state.join("proxy");
    parcela::private::directory(&directory).unwrap();
    let probe = parcela::private::Lock::acquire(&directory.join("owner.lock")).unwrap();
    let wrapper = root.join("caddy-probe");
    let reached = root.join("version-reached");
    fs::write(&wrapper, format!("#!/bin/sh\nif [ \"$1\" = version ]; then\n  '{}' version\n  touch '{}'\nelse\n  exec '{}' \"$@\"\nfi\n", caddy.display(), reached.display(), caddy.display())).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let port = free_port().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(&proxy.state)
        .args(["proxy", "run", "--port", &port, "--caddy"])
        .arg(&wrapper)
        .env("PARCELA_NO_TRUST", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !reached.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(150));
    let exited = child.try_wait().unwrap();
    drop(probe);
    if exited.is_some() {
        let output = child.wait_with_output().unwrap();
        panic!(
            "a transient status probe aborted startup: {}",
            stderr(&output)
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cli(&proxy.state, &["proxy", "status"]).status.success() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(cli(&proxy.state, &["proxy", "stop"]).status.success());
    assert!(child.wait().unwrap().success());
}

#[test]
fn proxy_does_not_retain_an_unrelated_listener_inherited_from_its_caller() {
    use std::{os::fd::AsRawFd, process::Stdio};
    let Some(caddy) = caddy() else {
        return;
    };
    let proxy = Proxy::new();
    let foreign = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let foreign_port = foreign.local_addr().unwrap().port();
    let port = free_port().to_string();
    let fd = foreign.as_raw_fd();
    // SAFETY: the test owns fd. This reproduces a descriptor inherited across a concurrent spawn.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) },
        0
    );
    let child = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(&proxy.state)
        .args(["proxy", "start", "--port", &port, "--caddy"])
        .arg(caddy)
        .env("PARCELA_NO_TRUST", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFD, flags) }, 0);
    drop(foreign);
    let started = child.unwrap().wait_with_output().unwrap();
    assert!(started.status.success(), "{}", stderr(&started));
    assert!(
        TcpListener::bind(("127.0.0.1", foreign_port)).is_ok(),
        "Parcela or Caddy retained the caller's unrelated listener on {foreign_port}"
    );
    assert!(cli(&proxy.state, &["proxy", "status"]).status.success());
}

#[test]
fn trust_requires_both_input_and_error_terminals_before_any_state_effects() {
    use std::{fs::File, os::fd::FromRawFd, process::Stdio};
    for (input_terminal, error_terminal) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let proxy = Proxy::new();
        let (mut primary, mut secondary) = (0, 0);
        // SAFETY: openpty initializes two owned descriptors; no terminal settings are changed.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut primary,
                    &mut secondary,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let (_reader, writer) =
            unsafe { (File::from_raw_fd(primary), File::from_raw_fd(secondary)) };
        let output = Command::new(env!("CARGO_BIN_EXE_parcela"))
            .arg("--state-dir")
            .arg(&proxy.state)
            .args(["trust", "--caddy", "/nonexistent/trust-backend"])
            .env("PARCELA_NO_TRUST", "1")
            .env("PATH", "")
            .env("PARCELA_HTTPS_PORT", free_port().to_string())
            .stdin(if input_terminal {
                Stdio::from(writer.try_clone().unwrap())
            } else {
                Stdio::null()
            })
            .stderr(if error_terminal {
                Stdio::from(writer.try_clone().unwrap())
            } else {
                Stdio::piped()
            })
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            proxy.state.exists(),
            input_terminal && error_terminal,
            "stdin terminal={input_terminal}, stderr terminal={error_terminal}"
        );
        assert!(!proxy.state.join("proxy/data/pki").exists());
    }
}

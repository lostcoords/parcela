use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn cli(config: &Path, state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(state)
        .env("PARCELA_CONFIG", config)
        .env("DO_NOT_TRACK", "1")
        .env_remove("PARCELA_DOMAIN")
        .env_remove("PARCELA_DOMAIN_PORT")
        .env_remove("PARCELA_HTTPS_PORT")
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    assert_eq!(output.status.code(), Some(1));
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn config_sets_reads_and_clears_host_settings() {
    let temp = TempDir::new().unwrap();
    let config = temp.path().join("home/parcela/config.yaml");
    let state = temp.path().join("state");
    let run = |args: &[&str]| cli(&config, &state, args);

    assert_eq!(
        stdout(&run(&["config", "path"])).trim(),
        format!("{:?}", config.display().to_string())
    );
    assert!(!run(&["config", "get", "sharing.domains"]).status.success());
    stdout(&run(&[
        "config",
        "add",
        "sharing.domains",
        "Tower.Example.com",
    ]));
    stdout(&run(&[
        "config",
        "add",
        "sharing.domains",
        "mb.example.org",
    ]));
    stdout(&run(&["config", "set", "sharing.domain_port", "1481"]));
    assert_eq!(
        stdout(&run(&["config", "get", "sharing.domains"])).trim(),
        "[\n  \"tower.example.com\",\n  \"mb.example.org\"\n]"
    );
    let list: serde_json::Value = serde_json::from_str(&stdout(&run(&["config"]))).unwrap();
    assert_eq!(
        list["sharing.domains"],
        serde_json::json!(["tower.example.com", "mb.example.org"])
    );
    assert_eq!(list["sharing.domain_port"], 1481);
    assert_eq!(list["sharing.lan"], serde_json::Value::Null);
    assert_eq!(list["proxy.port"], 1443);
    assert!(fs::read_to_string(&config).unwrap().contains(
        "sharing:\n  domains: [\"tower.example.com\",\"mb.example.org\"]\n  domain_port: 1481\n"
    ));

    assert!(stderr(&run(&["config", "set", "sharing.funnel", "true"])).contains("unknown key"));
    assert!(stderr(&run(&["config", "set", "proxy.port", "8443"])).contains("read-only"));
    assert!(
        stderr(&run(&["config", "add", "sharing.domains", "a.localhost"])).contains(".localhost")
    );
    stdout(&run(&[
        "config",
        "remove",
        "sharing.domains",
        "tower.example.com",
    ]));
    stdout(&run(&["config", "unset", "sharing.domains"]));
    let list: serde_json::Value = serde_json::from_str(&stdout(&run(&["config"]))).unwrap();
    let set: Vec<_> = list
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, value)| {
            !value.is_null() && value.as_array().is_none_or(|items| !items.is_empty())
        })
        .map(|(key, _)| key.as_str())
        .collect();
    assert_eq!(set, ["proxy.port"]);
}

#[test]
fn up_takes_the_host_domain_unless_sharing_is_off() {
    let temp = TempDir::new().unwrap();
    let config = temp.path().join("config.yaml");
    let state = temp.path().join("state");
    let checkout = temp.path().join("notes");
    fs::create_dir(&checkout).unwrap();
    fs::write(
        checkout.join("parcela.yaml"),
        "version: 1\nproject: notes\nservices: {web: {command: sleep 60, http: {healthcheck: /}}}\n",
    )
    .unwrap();
    // Preflight refuses the proxy's own port, so nothing starts.
    let up = |flags: &[&str]| {
        let mut args = vec![
            "--path",
            checkout.to_str().unwrap(),
            "up",
            "--backend-port",
            "1443",
        ];
        args.extend_from_slice(flags);
        stderr(&cli(&config, &state, &args))
    };

    assert!(up(&["--domain-port", "1481"]).contains("--domain-port requires --domain"));
    fs::write(&config, "sharing:\n  domains: [tower.example.com]\n").unwrap();
    let error = up(&["--domain-port", "1481"]);
    assert!(!error.contains("--domain-port requires"), "{error}");

    fs::write(&config, "sharing:\n  funnel: true\n").unwrap();
    let error = up(&[]);
    assert!(
        error.contains("invalid") && error.contains("config.yaml"),
        "{error}"
    );
    let error = up(&["--no-share"]);
    assert!(
        !error.contains("config.yaml"),
        "--no-share never reads it: {error}"
    );
}

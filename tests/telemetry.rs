use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use tempfile::TempDir;

/// A directory with a fake curl that records any call, so a test can prove nothing was sent.
fn sandbox() -> TempDir {
    let t = TempDir::new().unwrap();
    let bin = t.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let curl = bin.join("curl");
    let called = t.path().join("curl-called");
    fs::write(&curl, format!("#!/bin/sh\ntouch '{}'\n", called.display())).unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
    t
}

fn parcela(t: &Path, args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(t.join("state"))
        .args(args)
        .env("PATH", format!("{}:/usr/bin:/bin", t.join("bin").display()))
        .env_remove("DO_NOT_TRACK")
        .env_remove("PARCELA_TELEMETRY_DISABLED")
        .env_remove("PARCELA_TELEMETRY_DEBUG")
        .env("PARCELA_CONFIG", t.join("config.yaml"))
        .envs(env.iter().copied())
        .output()
        .unwrap()
}

fn status(t: &Path, args: &[&str], env: &[(&str, &str)]) -> Value {
    let output = parcela(t, &[&["telemetry"], args].concat(), env);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn telemetry_command_turns_it_off_and_on() {
    let t = sandbox();
    let t = t.path();
    assert_eq!(status(t, &[], &[])["enabled"], true);
    let off = status(t, &["disable"], &[]);
    assert_eq!(off["enabled"], false);
    assert_eq!(off["disabled_by"], "parcela telemetry disable");
    assert!(
        fs::read_to_string(t.join("config.yaml"))
            .unwrap()
            .contains("telemetry: false\n")
    );
    assert_eq!(status(t, &["status"], &[])["enabled"], false);
    assert_eq!(status(t, &["enable"], &[])["enabled"], true);
    // A choice saved by an older release still counts until the host config makes one.
    fs::write(t.join("config.yaml"), "").unwrap();
    fs::write(
        t.join("state/telemetry.json"),
        "{\"noticed\":true,\"enabled\":false}",
    )
    .unwrap();
    fs::set_permissions(
        t.join("state/telemetry.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert_eq!(status(t, &["status"], &[])["enabled"], false);
    let set = parcela(t, &["config", "set", "telemetry", "true"], &[]);
    assert!(set.status.success());
    assert_eq!(status(t, &["status"], &[])["enabled"], true);
    let tracked = status(t, &["status"], &[("DO_NOT_TRACK", "1")]);
    assert_eq!(tracked["disabled_by"], "DO_NOT_TRACK");
    let env = status(t, &["status"], &[("PARCELA_TELEMETRY_DISABLED", "1")]);
    assert_eq!(env["disabled_by"], "PARCELA_TELEMETRY_DISABLED");
}

#[test]
fn debug_prints_the_payload_and_sends_nothing() {
    let t = sandbox();
    let checkout = t.path().join("acme-secret");
    fs::create_dir(&checkout).unwrap();
    let path = checkout.to_str().unwrap();
    let output = parcela(
        t.path(),
        &["--path", path, "ls", "--json"],
        &[("PARCELA_TELEMETRY_DEBUG", "1")],
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    let start = stderr.find('{').expect("payload on stderr");
    let event: Value = serde_json::from_str(&stderr[start..]).unwrap();
    assert_eq!(event["command"], "ls");
    assert_eq!(
        event["flags"],
        serde_json::json!(["--json", "--path", "--state-dir"])
    );
    assert_eq!(event["schema"], 1);
    assert!(!stderr.contains("acme-secret"), "{stderr}");
    assert!(!t.path().join("curl-called").exists());
}

#[test]
fn test_builds_never_notice_write_or_send() {
    let t = sandbox();
    for _ in 0..3 {
        let output = parcela(t.path(), &["ls"], &[]);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("anonymous usage"));
    }
    assert!(!t.path().join("curl-called").exists());
    assert!(!t.path().join("state/telemetry.json").exists());
}

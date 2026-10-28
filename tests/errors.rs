use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

mod common;

fn cli(path: &Path, state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--path")
        .arg(path)
        .arg("--state-dir")
        .arg(state)
        .args(args)
        .output()
        .unwrap()
}
fn up(path: &Path, state: &Path, flags: &[&str]) -> String {
    let mut args = vec![
        "up",
        // Registers the checkout, then preflight refuses the proxy's port before anything starts.
        "--backend-port",
        "1443",
    ];
    args.extend_from_slice(flags);
    let output = cli(path, state, &args);
    assert_eq!(output.status.code(), Some(1));
    String::from_utf8(output.stderr).unwrap()
}
fn checkout(root: &Path, name: &str, manifest: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("parcela.yaml"), manifest).unwrap();
    path
}

#[test]
fn unknown_config_fields_explain_how_to_check_and_upgrade() {
    let t = TempDir::new().unwrap();
    let root = t.path().canonicalize().unwrap();
    let state = root.join("state");
    for (name, manifest, field) in [
        (
            "future",
            "version: 1\nproject: demo\nservices: {}\nfuture_feature: true\n",
            "future_feature",
        ),
        (
            "typo",
            "version: 1\nproject: demo\nservices: {web: {commmand: bin/web}}\n",
            "commmand",
        ),
    ] {
        let path = checkout(&root, name, manifest);
        let output = cli(&path, &state, &["up"]);
        assert_eq!(output.status.code(), Some(1));
        let message = String::from_utf8(output.stderr).unwrap();
        assert!(
            message.contains(&format!("unknown field `{field}`")),
            "{message}"
        );
        assert!(message.contains("check the field spelling"), "{message}");
        assert!(message.contains("newer release"), "{message}");
        assert!(message.contains("parcela upgrade"), "{message}");
        assert!(!state.exists(), "invalid config must not create state");
    }
}

#[test]
fn registration_conflicts_name_the_holder_and_the_fix() {
    let t = TempDir::new().unwrap();
    let root = t.path().canonicalize().unwrap();
    let state = root.join("state");
    let manifest = "version: 1\nproject: demo\nservices:\n  web: {command: bin/web, http: {healthcheck: /}}\n  api: {command: bin/api, http: {subdomain: api, healthcheck: /}}\n";
    let first = checkout(&root, "first", manifest);
    let second = checkout(&root, "second", manifest);
    let third = checkout(&root, "third", manifest);
    up(&first, &state, &[]);
    assert_eq!(
        up(&second, &state, &[]),
        format!(
            "parcela: demo already has a primary checkout at {}; start this one as a preview with --name <name>\n",
            first.display()
        )
    );
    up(&second, &state, &["--name", "review"]);
    assert_eq!(
        up(&third, &state, &["--name", "review"]),
        format!(
            "parcela: preview name review is already used by {}; choose another with --name <name>\n",
            second.display()
        )
    );
    assert_eq!(
        up(&third, &state, &["--name", "api"]),
        format!(
            "parcela: api-demo.localhost is already used by {}; choose another preview name with --name <name>\n",
            first.display()
        )
    );
}

#[test]
fn missing_manifest_and_unstarted_checkout_point_to_the_next_command() {
    let t = TempDir::new().unwrap();
    let root = t.path().canonicalize().unwrap();
    let state = root.join("state");
    let no_manifest = format!(
        "parcela: no parcela.yaml in {} or above; create one with `parcela init`, or pass --path to a configured checkout\n",
        root.display()
    );
    for command in ["status", "up", "url", "logs"] {
        let output = cli(
            &root,
            &state,
            &if command == "status" {
                vec![command, "--current"]
            } else {
                vec![command]
            },
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            no_manifest,
            "{command}"
        );
    }
    let named = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(&state)
        .args(["-w", "shop", "status"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&named.stderr),
        "parcela: no environment named shop; list environments with parcela ls\n"
    );
    let id = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(&state)
        .args(["-w", "6b0f2c3e-1111-4222-8333-444455556666", "status"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&id.stderr),
        "parcela: unknown environment ID; list environments with parcela ls\n"
    );
    let mut bare = Command::new(env!("CARGO_BIN_EXE_parcela"));
    bare.arg("--path").arg(&root).arg("--state-dir").arg(&state);
    assert_eq!(
        common::on_terminal(bare).1,
        "No parcela.yaml here. Create one with `parcela init`, then run `parcela up`.\nSee `parcela --help` for every command.\n"
    );
    let app = checkout(
        &root,
        "app",
        "version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n",
    );
    let bare = cli(&app, &state, &[]);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bare.stdout).unwrap(),
        serde_json::json!({"hint": "Not started yet. Run `parcela up`."})
    );
    for command in ["status", "url"] {
        let output = cli(
            &app,
            &state,
            &if command == "status" {
                vec![command, "--current"]
            } else {
                vec![command]
            },
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "parcela: this checkout has not been started; start it with `parcela up`\n",
            "{command}"
        );
    }
    up(&app, &state, &[]);
    let bare = cli(&app, &state, &[]);
    let status: serde_json::Value = serde_json::from_slice(&bare.stdout).unwrap();
    assert_eq!(status["state"], "stopped");
    for command in ["status", "url"] {
        let output = cli(
            &app,
            &state,
            &if command == "status" {
                vec![command, "--current"]
            } else {
                vec![command]
            },
        );
        assert!(output.status.success(), "{command}");
    }

    // Renaming the project strands the registration; the message names the way out, and it works.
    fs::write(
        app.join("parcela.yaml"),
        "version: 1\nproject: shop\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n",
    )
    .unwrap();
    let id = status["workspace"]["id"].as_str().unwrap();
    let renamed = cli(&app, &state, &["status", "--current"]);
    assert_eq!(
        String::from_utf8_lossy(&renamed.stderr),
        format!(
            "parcela: parcela.yaml now gives this checkout shop.localhost, but it was started as app.localhost; forget the old one with `parcela -w {id} rm`, then run `parcela up`\n"
        )
    );
    let forget = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(&state)
        .args(["-w", id, "rm"])
        .output()
        .unwrap();
    assert!(
        forget.status.success(),
        "{}",
        String::from_utf8_lossy(&forget.stderr)
    );
    assert!(up(&app, &state, &[]).contains("1443"));
}

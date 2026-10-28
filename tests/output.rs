use parcela::{
    config::Manifest,
    registry::{Registry, Role},
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

mod common;

const MANIFEST: &str =
    "version: 1\nproject: demo\nservices: {web: {command: echo unused, http: {healthcheck: /}}}\n";

fn command(path: &Path, state: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_parcela"));
    command
        .arg("--path")
        .arg(path)
        .arg("--state-dir")
        .arg(state)
        .args(args)
        .env_remove("PARCELA_HTTPS_PORT");
    command
}
fn piped(path: &Path, state: &Path, args: &[&str]) -> Output {
    command(path, state, args).output().unwrap()
}
fn terminal(path: &Path, state: &Path, args: &[&str]) -> String {
    let (code, text) = common::on_terminal(command(path, state, args));
    assert_eq!(code, Some(0), "{args:?}: {text}");
    text
}
fn project() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let t = TempDir::new().unwrap();
    let path = t.path().canonicalize().unwrap().join("demo");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("parcela.yaml"), MANIFEST).unwrap();
    let state = t.path().join("state");
    (t, path, state)
}

#[test]
fn piped_stdout_is_json_and_json_flag_forces_it_on_a_terminal() {
    let (_t, path, state) = project();
    let empty =
        serde_json::json!({"runtime_verified": false, "workspaces": [], "unregistered": []});
    let outside = path.parent().unwrap();
    for args in [&["ls"][..], &["--json", "ls"]] {
        let output = piped(outside, &state, args);
        assert!(output.status.success());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            empty
        );
    }
    let forced = terminal(outside, &state, &["--json", "ls"]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&forced).unwrap(),
        empty
    );
}

#[test]
fn the_human_flag_is_gone() {
    let (_t, path, state) = project();
    assert_eq!(
        piped(&path, &state, &["--human", "status"]).status.code(),
        Some(2)
    );
}

#[test]
fn a_terminal_gets_aligned_text() {
    let (_t, path, state) = project();
    assert_eq!(
        terminal(path.parent().unwrap(), &state, &["ls"]),
        "No environments registered. Start one with `parcela up`.\n"
    );
    assert_eq!(
        terminal(&path, &state, &["ls"]),
        format!(
            "PROJECT  NAME     STATE        URL                          PATH\ndemo     primary  not started  https://demo.localhost:1443  {}\n",
            path.display()
        )
    );
    let manifest = Manifest::load(&path.join("parcela.yaml")).unwrap();
    Registry::open(&state, true)
        .unwrap()
        .unwrap()
        .adopt(&manifest, &path, Role::Primary, "primary")
        .unwrap();
    assert_eq!(
        terminal(&path, &state, &["ls"]),
        format!(
            "PROJECT  NAME     STATE    URL                          PATH\ndemo     primary  stopped  https://demo.localhost:1443  {}\n",
            path.display()
        )
    );
    let status: serde_json::Value =
        serde_json::from_slice(&piped(&path, &state, &["status", "--current"]).stdout).unwrap();
    assert_eq!(
        (&status["state"], &status["processes"]),
        (&serde_json::json!("stopped"), &serde_json::json!([]))
    );
    assert_eq!(
        terminal(&path, &state, &["down"]),
        "✓ demo is not running\n"
    );
}

use parcela::registry::{Registry, Role};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

const MANIFEST: &str =
    "version: 1\nproject: demo\nservices: {web: {command: echo unused, http: {healthcheck: /}}}\n";

fn source(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("parcela.yaml"), MANIFEST).unwrap();
}
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
fn attempt(path: &Path, state: &Path, flags: &[&str]) {
    let mut args = vec![
        "up",
        // Registers the checkout, then preflight refuses the proxy's port before anything starts.
        "--backend-port",
        "1443",
    ];
    args.extend_from_slice(flags);
    let result = cli(path, state, &args);
    assert!(!result.status.success());
}
fn registered(state: &Path) -> Vec<parcela::registry::Workspace> {
    Registry::open(state, false)
        .unwrap()
        .map_or_else(Vec::new, |r| r.list(None).unwrap())
}
fn jj(path: &Path, args: &[&str]) {
    let result = Command::new("jj")
        .current_dir(path)
        .args(["--config", "signing.behavior=\"drop\""])
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn plain_up_registers_selected_project_without_an_adopt_command() {
    let t = TempDir::new().unwrap();
    let path = t.path().join("project");
    source(&path);
    let state = t.path().join("state");
    attempt(&path, &state, &[]);
    let rows = registered(&state);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].role, Role::Primary);
    assert_eq!(rows[0].hostname, "demo.localhost");
    let id = rows[0].id.clone();
    attempt(&path, &state, &[]);
    assert_eq!(registered(&state)[0].id, id);
    assert_eq!(
        fs::read_to_string(path.join("parcela.yaml")).unwrap(),
        MANIFEST
    );
}
#[test]
fn named_preview_is_persisted_and_cannot_be_implicitly_renamed() {
    let t = TempDir::new().unwrap();
    let path = t.path().join("project");
    source(&path);
    let state = t.path().join("state");
    attempt(&path, &state, &["--name", "review"]);
    let rows = registered(&state);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].role, Role::Preview);
    assert_eq!(rows[0].hostname, "review-demo.localhost");
    let id = rows[0].id.clone();
    attempt(&path, &state, &[]);
    attempt(&path, &state, &["--name", "other"]);
    let rows = registered(&state);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].label, "review");
    attempt(&path, &state, &["--primary"]);
    assert_eq!(registered(&state)[0].role, Role::Preview);
}
#[test]
fn invalid_explicit_name_does_not_create_registry_state() {
    let t = TempDir::new().unwrap();
    let path = t.path().join("project");
    source(&path);
    for name in ["../escape", "Upper", "", "a.b"] {
        let state = t.path().join("state");
        attempt(&path, &state, &["--name", name]);
        assert!(!state.exists());
    }
}
#[test]
fn preview_name_collision_preserves_both_sources_and_existing_identity() {
    let t = TempDir::new().unwrap();
    let a = t.path().join("a");
    let b = t.path().join("b");
    source(&a);
    source(&b);
    let state = t.path().join("state");
    attempt(&a, &state, &["--name", "review"]);
    let rows = registered(&state);
    assert_eq!(rows.len(), 1);
    let id = rows[0].id.clone();
    attempt(&b, &state, &["--name", "review"]);
    let rows = registered(&state);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].source, a.canonicalize().unwrap());
}
#[test]
fn first_jj_linked_start_is_preview_and_main_stays_available() {
    let t = TempDir::new().unwrap();
    let main = t.path().join("main");
    source(&main);
    jj(&main, &["git", "init"]);
    let preview = t.path().join("linked");
    jj(
        &main,
        &[
            "workspace",
            "add",
            "--name",
            "Review_Test",
            "-r",
            "@",
            preview.to_str().unwrap(),
        ],
    );
    let state = t.path().join("state");
    attempt(&preview, &state, &[]);
    let rows = registered(&state);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].role, Role::Preview);
    assert_eq!(rows[0].hostname, "review-test-demo.localhost");
    let id = rows[0].id.clone();
    jj(&preview, &["workspace", "rename", "renamed"]);
    attempt(&preview, &state, &[]);
    assert_eq!(registered(&state)[0].id, id);
    attempt(&main, &state, &[]);
    let rows = registered(&state);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter(|w| w.role == Role::Primary).count(), 1);
}
#[test]
fn run_uses_the_same_named_registration_path() {
    let t = TempDir::new().unwrap();
    let path = t.path().join("project");
    source(&path);
    let state = t.path().join("state");
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    // A literal command is only found at launch, after `run` has started the shared proxy.
    let result = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--path")
        .arg(&path)
        .arg("--state-dir")
        .arg(&state)
        .env("PARCELA_HTTPS_PORT", port.to_string())
        .args([
            "run",
            "--name",
            "command-review",
            "--",
            "/nonexistent/parcela-test-command",
        ])
        .output()
        .unwrap();
    cli(&path, &state, &["proxy", "stop"]);
    assert!(!result.status.success());
    let rows = registered(&state);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].hostname, "command-review-demo.localhost");
}

fn git(path: &Path, args: &[&str]) {
    assert!(!path.join(".jj").exists());
    let result = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn first_detached_git_worktree_start_is_not_promoted_to_primary() {
    let t = TempDir::new().unwrap();
    let main = t.path().join("main");
    source(&main);
    git(&main, &["init", "--initial-branch", "main"]);
    git(
        &main,
        &[
            "-c",
            "user.name=Parcela Tests",
            "-c",
            "user.email=tests@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "fixture",
        ],
    );
    let preview = t.path().join("review copy");
    git(
        &main,
        &["worktree", "add", "--detach", preview.to_str().unwrap()],
    );
    source(&preview);
    let state = t.path().join("state");
    attempt(&preview, &state, &[]);
    let rows = registered(&state);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].role, Role::Preview);
    assert_eq!(rows[0].hostname, "review-copy-demo.localhost");
    attempt(&main, &state, &[]);
    assert_eq!(registered(&state).len(), 2);
}
#[test]
fn conflicting_primary_never_replaces_the_existing_owner() {
    let t = TempDir::new().unwrap();
    let a = t.path().join("a");
    let b = t.path().join("b");
    source(&a);
    source(&b);
    let state = t.path().join("state");
    attempt(&a, &state, &[]);
    let id = registered(&state)[0].id.clone();
    attempt(&b, &state, &[]);
    assert_eq!(registered(&state).len(), 1);
    assert_eq!(registered(&state)[0].id, id);
    attempt(&b, &state, &["--name", "other"]);
    assert_eq!(registered(&state).len(), 2);
}
#[test]
fn primary_override_is_explicit_and_inspection_never_registers() {
    let t = TempDir::new().unwrap();
    let main = t.path().join("main");
    source(&main);
    jj(&main, &["git", "init"]);
    let linked = t.path().join("linked");
    jj(
        &main,
        &[
            "workspace",
            "add",
            "--name",
            "review",
            "-r",
            "@",
            linked.to_str().unwrap(),
        ],
    );
    let state = t.path().join("state");
    for args in [
        vec!["ls"],
        vec!["status", "--current"],
        vec!["context"],
        vec!["url"],
    ] {
        cli(&linked, &state, &args);
        assert!(!state.exists());
    }
    attempt(&linked, &state, &["--primary"]);
    assert_eq!(registered(&state)[0].role, Role::Primary);
}

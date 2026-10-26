use parcela::{
    config::Manifest,
    registry::{Registry, Role},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

const MANIFEST: &str =
    "version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n";
fn output(command: &mut Command) -> Output {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}
fn jj(source: &Path, args: &[&str]) -> String {
    String::from_utf8(
        output(
            Command::new("jj")
                .arg("--repository")
                .arg(source)
                .args(["--config", "signing.behavior=\"drop\""])
                .args(args),
        )
        .stdout,
    )
    .unwrap()
}
fn git(source: &Path, args: &[&str]) -> String {
    assert!(
        !source.join(".jj").exists(),
        "never use Git in a colocated fixture"
    );
    String::from_utf8(output(Command::new("git").arg("-C").arg(source).args(args)).stdout).unwrap()
}
fn ls(path: &Path, state: &Path) -> Value {
    serde_json::from_slice(
        &output(
            Command::new(env!("CARGO_BIN_EXE_parcela"))
                .arg("--path")
                .arg(path)
                .arg("--state-dir")
                .arg(state)
                .env("PARCELA_HTTPS_PORT", "47443")
                .arg("ls"),
        )
        .stdout,
    )
    .unwrap()
}
/// `(label, role, url, source)` of each unregistered checkout.
fn unregistered(listing: &Value) -> Vec<(String, String, String, String)> {
    listing["unregistered"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            assert_eq!(row["state"], "not started");
            let workspace = &row["workspace"];
            (
                workspace["label"].as_str().unwrap().to_owned(),
                workspace["role"].as_str().unwrap().to_owned(),
                row["url"].as_str().unwrap().to_owned(),
                workspace["source"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}
fn operation(source: &Path) -> String {
    jj(
        source,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "op",
            "log",
            "--limit",
            "1",
            "--template",
            "id",
        ],
    )
}

#[test]
fn ls_shows_unregistered_jj_workspaces_without_snapshotting_or_registering() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("source");
    output(
        Command::new("jj")
            .args(["git", "init", "--colocate"])
            .arg(&source),
    );
    fs::write(source.join("parcela.yaml"), MANIFEST).unwrap();
    jj(&source, &["bookmark", "set", "main"]);
    jj(&source, &["new", "main"]);
    let sibling = root.join("sibling with spaces");
    jj(
        &source,
        &[
            "workspace",
            "add",
            "--name",
            "preview",
            "-r",
            "main",
            sibling.to_str().unwrap(),
        ],
    );
    fs::write(
        source.join("unsnapshotted.txt"),
        "keep this edit outside jj's operation log",
    )
    .unwrap();
    let before = operation(&source);
    let state = root.join("state");
    assert_eq!(
        unregistered(&ls(&source, &state)),
        [
            (
                "primary".into(),
                "primary".into(),
                "https://app.localhost:47443".into(),
                source.display().to_string()
            ),
            (
                "preview".into(),
                "preview".into(),
                "https://preview-app.localhost:47443".into(),
                sibling.display().to_string()
            ),
        ]
    );
    assert!(!state.exists());

    let manifest = Manifest::load(&source.join("parcela.yaml")).unwrap();
    let adopted = Registry::open(&state, true)
        .unwrap()
        .unwrap()
        .adopt(&manifest, &source, Role::Primary, "primary")
        .unwrap();
    let listing = ls(&sibling, &state);
    assert_eq!(
        listing["workspaces"][0]["workspace"]["id"],
        json!(adopted.id)
    );
    assert_eq!(
        unregistered(&listing)
            .into_iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        ["preview"]
    );
    let registry = Registry::open(&state, false).unwrap().unwrap();
    assert_eq!(registry.list(None).unwrap().len(), 1);
    assert_eq!(before, operation(&source));
}

#[test]
fn ls_names_an_unregistered_git_worktree_after_its_directory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    git(&source, &["init", "--initial-branch", "main"]);
    fs::write(source.join("parcela.yaml"), MANIFEST).unwrap();
    git(&source, &["add", "parcela.yaml"]);
    git(
        &source,
        &[
            "-c",
            "user.name=Parcela Tests",
            "-c",
            "user.email=tests@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "fixture",
        ],
    );
    let sibling = root.join("detached");
    git(
        &source,
        &["worktree", "add", "--detach", sibling.to_str().unwrap()],
    );
    let head = git(&source, &["rev-parse", "HEAD"]);
    let state = root.join("state");
    let rows = unregistered(&ls(&sibling, &state));
    assert_eq!(
        rows.iter()
            .map(|row| (row.0.as_str(), row.2.as_str()))
            .collect::<Vec<_>>(),
        [
            ("primary", "https://app.localhost:47443"),
            ("detached", "https://detached-app.localhost:47443")
        ]
    );
    assert_eq!(head, git(&source, &["rev-parse", "HEAD"]));
    assert!(!state.exists());
}

#[test]
fn ls_outside_a_project_shows_only_registered_environments() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let state = root.join("absent-state");
    let listing = ls(&root, &state);
    assert_eq!(
        listing,
        json!({"runtime_verified": false, "workspaces": [], "unregistered": []})
    );
    assert!(!state.exists());
}

#[test]
fn a_missing_jj_workspace_does_not_block_creating_or_listing_live_checkouts() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("source");
    output(
        Command::new("jj")
            .args(["git", "init", "--colocate"])
            .arg(&source),
    );
    fs::write(source.join("parcela.yaml"), MANIFEST).unwrap();
    let missing = root.join("missing");
    jj(
        &source,
        &[
            "workspace",
            "add",
            "--name",
            "missing",
            "-r",
            "@",
            missing.to_str().unwrap(),
        ],
    );
    fs::remove_dir_all(&missing).unwrap();
    let listing = ls(&source, &root.join("state"));
    assert_eq!(unregistered(&listing).len(), 1, "{listing}");
    output(
        Command::new(env!("CARGO_BIN_EXE_parcela"))
            .arg("--path")
            .arg(&source)
            .arg("--state-dir")
            .arg(root.join("state"))
            .args(["create", "usable"]),
    );
    assert!(root.join("source-usable/parcela.yaml").exists());
}

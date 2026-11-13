use parcela::{
    project::config::{Manifest, dns_label, locate},
    state::context::ResolvedContext,
    state::registry::{Registry, Role},
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::{Command, Output},
    sync::{Arc, Barrier},
    thread,
};
use tempfile::TempDir;

const MANIFEST: &str = "version: 1\nproject: demo\nservices:\n  web: {command: echo hi, http: {healthcheck: /}}\n  api: {command: echo api, http: {subdomain: api, healthcheck: /}}\ntasks: {test: {command: python3 -m unittest, timeout_seconds: 60}}\n";
fn manifest() -> Manifest {
    Manifest::parse(MANIFEST).unwrap()
}
fn checkout(root: &Path, name: &str) -> std::path::PathBuf {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    fs::write(path.join("parcela.yaml"), MANIFEST).unwrap();
    path.canonicalize().unwrap()
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
/// Register through a first `up` that stops at preflight.
fn register(path: &Path, state: &Path, flags: &[&str]) -> Output {
    let mut args = vec![
        "up",
        // Registers the checkout, then preflight refuses the proxy's port before anything starts.
        "--backend-port",
        "1443",
    ];
    args.extend_from_slice(flags);
    let output = cli(path, state, &args);
    assert_eq!(output.status.code(), Some(1));
    output
}
fn value(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn strict_schema_with_extension_keys() {
    manifest();
    Manifest::parse(&format!("{MANIFEST}x-shared: &shared {{a: 1}}\n")).unwrap();
    for text in [
        format!("{MANIFEST}typo: true\n"),
        MANIFEST.replace(
            "http: {healthcheck: /}",
            "http: {healthcheck: /, typo: true}",
        ),
        MANIFEST.replace("timeout_seconds: 60", "typo: 60"),
        MANIFEST.replace("version: 1", "version: 2"),
        MANIFEST.replace("version: 1\n", ""),
        MANIFEST.replace("project: demo\n", ""),
    ] {
        assert!(Manifest::parse(&text).is_err(), "{text}");
    }
}
#[test]
fn validation_rejects_bad_contracts() {
    for text in [
        MANIFEST.replace("subdomain: api, ", ""),
        MANIFEST.replace("http: {healthcheck: /}", "healthcheck: true"),
        MANIFEST.replace("project: demo", "project: ../escape"),
        MANIFEST.replace("command: python3 -m unittest", "command: ''"),
        MANIFEST.replace("timeout_seconds: 60", "timeout_seconds: 0"),
        MANIFEST.replace("command: echo hi,", "command: echo hi, depends_on: [db],"),
        MANIFEST
            .replace("command: echo hi,", "command: echo hi, depends_on: [api],")
            .replace(
                "command: echo api,",
                "command: echo api, depends_on: [web],",
            ),
        MANIFEST.replace("subdomain: api", "subdomain: API"),
        format!("{MANIFEST}hooks: {{setup: {{command: bin/setup, depends_on: [db]}}}}\n"),
        MANIFEST.replace("  api:", "  setup:"),
    ] {
        assert!(Manifest::parse(&text).is_err(), "{text}");
    }
    for label in ["", "Upper", "a.b", "-a", "a-", "../a", "á", &"a".repeat(64)] {
        assert!(dns_label(label).is_err());
    }
    dns_label("auth-2").unwrap();
    assert!(manifest().hostname(false, &"a".repeat(60)).is_err());
}
#[test]
fn flat_planned_urls_and_explicit_no_runtime_claims() {
    let temp = TempDir::new().unwrap();
    let path = checkout(temp.path(), "app");
    let mut registry = Registry::open(&temp.path().join("state"), true)
        .unwrap()
        .unwrap();
    let m = manifest();
    let primary = registry.adopt(&m, &path, Role::Primary, "main").unwrap();
    assert_eq!(primary.hostname, "demo.localhost");
    let preview_path = checkout(temp.path(), "preview");
    let preview = registry
        .adopt(&m, &preview_path, Role::Preview, "auth")
        .unwrap();
    let context = ResolvedContext::new(&registry, &m, preview.clone()).unwrap();
    assert_eq!(
        context.planned_urls["web"],
        "https://auth-demo.localhost:1443"
    );
    assert_eq!(
        context.planned_urls["api"],
        "https://api-auth-demo.localhost:1443"
    );
    parcela::proxy::frontdoor::FrontDoor::on(443)
        .save(registry.state_path())
        .unwrap();
    let standard = ResolvedContext::new(&registry, &m, preview).unwrap();
    assert_eq!(standard.planned_urls["web"], "https://auth-demo.localhost");
    assert!(!context.capabilities.routing && !context.capabilities.data_isolation_verified);
    assert!(!context.runtime_dir.exists());
    assert!(context.source_exists);
}
#[test]
fn identities_survive_reopen_and_metadata_changes() {
    let temp = TempDir::new().unwrap();
    let path = checkout(temp.path(), "app");
    fs::create_dir(path.join(".jj")).unwrap();
    fs::create_dir(path.join(".git")).unwrap();
    let state = temp.path().join("state");
    let m = manifest();
    let id = {
        let mut registry = Registry::open(&state, true).unwrap().unwrap();
        let workspace = registry.adopt(&m, &path, Role::Preview, "auth").unwrap();
        assert_eq!(workspace.vcs, "jj");
        workspace.id
    };
    fs::write(path.join(".git/HEAD"), "detached-revision").unwrap();
    let mut registry = Registry::open(&state, true).unwrap().unwrap();
    assert_eq!(
        registry.adopt(&m, &path, Role::Preview, "auth").unwrap().id,
        id
    );
    assert!(registry.adopt(&m, &path, Role::Primary, "auth").is_err());
    assert!(registry.adopt(&m, &path, Role::Preview, "new").is_err());
    let mut changed = manifest();
    changed.project.name = "changed".into();
    assert!(registry.current(&path, &changed).is_err());
    assert_eq!(registry.current(&path, &m).unwrap().id, id);
}
#[test]
fn conflicts_do_not_steal_ownership_or_leave_projects() {
    let temp = TempDir::new().unwrap();
    let a = checkout(temp.path(), "a");
    let b = checkout(temp.path(), "b");
    let state = temp.path().join("state");
    let mut registry = Registry::open(&state, true).unwrap().unwrap();
    let m = manifest();
    registry.adopt(&m, &a, Role::Preview, "auth").unwrap();
    assert!(registry.adopt(&m, &b, Role::Preview, "auth").is_err());
    let mut other = manifest();
    other.project.id = "different".into();
    other.project.name = "auth-demo".into();
    assert!(registry.adopt(&other, &b, Role::Primary, "main").is_err());
    let connection = rusqlite::Connection::open(state.join("registry.sqlite3")).unwrap();
    let count: i64 = connection
        .query_row("SELECT count(*) FROM projects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(registry.list(None).unwrap().len(), 1);
}
#[test]
fn only_one_primary_per_project_and_explicit_scoping() {
    let temp = TempDir::new().unwrap();
    let a = checkout(temp.path(), "a");
    let b = checkout(temp.path(), "b");
    let mut registry = Registry::open(&temp.path().join("state"), true)
        .unwrap()
        .unwrap();
    let m = manifest();
    registry.adopt(&m, &a, Role::Primary, "main").unwrap();
    assert!(registry.adopt(&m, &b, Role::Primary, "other").is_err());
    let mut other = manifest();
    other.project.id = "different".into();
    other.project.name = "other".into();
    registry.adopt(&other, &b, Role::Primary, "main").unwrap();
    assert_eq!(registry.list(Some("demo")).unwrap().len(), 1);
    assert_eq!(registry.list(None).unwrap().len(), 2);
}
#[test]
fn symlinked_source_is_same_identity_and_nested_path_finds_checkout() {
    let temp = TempDir::new().unwrap();
    let path = checkout(temp.path(), "app");
    fs::create_dir(path.join("src")).unwrap();
    assert_eq!(locate(&path.join("src")).unwrap(), path);
    let link = temp.path().join("link");
    symlink(&path, &link).unwrap();
    let mut registry = Registry::open(&temp.path().join("state"), true)
        .unwrap()
        .unwrap();
    let a = registry
        .adopt(&manifest(), &path, Role::Preview, "auth")
        .unwrap();
    assert_eq!(
        registry
            .adopt(&manifest(), &link, Role::Preview, "auth")
            .unwrap()
            .id,
        a.id
    );
}
#[test]
fn inspection_and_invalid_names_do_not_create_state() {
    let temp = TempDir::new().unwrap();
    let path = checkout(temp.path(), "app");
    let state = temp.path().join("state");
    assert_eq!(
        value(&cli(&path, &state, &["ls"]))["workspaces"],
        serde_json::json!([])
    );
    value(&cli(temp.path(), &state, &["ls"]));
    assert!(!cli(&path, &state, &["context"]).status.success());
    let invalid = cli(&path, &state, &["up", "--name", "../bad"]);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(!state.exists());
}
#[test]
fn cli_json_errors_and_source_preservation() {
    let temp = TempDir::new().unwrap();
    let path = checkout(temp.path(), "app");
    let state = temp.path().join("state");
    let before = fs::read(path.join("parcela.yaml")).unwrap();
    let first = register(&path, &state, &["--name", "auth"]);
    assert!(first.stdout.is_empty());
    let context = value(&cli(&path, &state, &["context"]));
    assert_eq!(
        (
            &context["workspace"]["role"],
            &context["workspace"]["label"]
        ),
        (&serde_json::json!("preview"), &serde_json::json!("auth"))
    );
    assert_eq!(before, fs::read(path.join("parcela.yaml")).unwrap());
    assert_eq!(fs::read_dir(&path).unwrap().count(), 1);
    let error = register(&path, &state, &["--primary"]);
    assert!(error.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&error.stderr),
        "parcela: checkout already uses preview identity \"auth\"; omit the naming flags to reuse it or select another checkout; no identity was changed\n"
    );
}
#[test]
fn private_state_and_database_permissions_and_symlinks() {
    let temp = TempDir::new().unwrap();
    let state = temp.path().join("state");
    drop(Registry::open(&state, true).unwrap());
    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(state.join("registry.sqlite3"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let link = temp.path().join("link");
    symlink(&state, &link).unwrap();
    assert!(Registry::open(&link, false).is_err());
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Registry::open(&state, true).is_err());
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let foreign = temp.path().join("foreign");
    fs::write(&foreign, "unrelated").unwrap();
    fs::remove_file(state.join("registry.sqlite3")).unwrap();
    symlink(&foreign, state.join("registry.sqlite3")).unwrap();
    assert!(Registry::open(&state, true).is_err());
    assert_eq!(fs::read_to_string(&foreign).unwrap(), "unrelated");
}
#[test]
fn malformed_and_future_state_fail_closed() {
    let temp = TempDir::new().unwrap();
    let state = temp.path().join("state");
    drop(Registry::open(&state, true).unwrap());
    let db = state.join("registry.sqlite3");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.pragma_update(None, "user_version", 999).unwrap();
    assert!(Registry::open(&state, true).is_err());
    assert!(Registry::open(&state, false).is_err());
    drop(connection);
    fs::write(&db, "not a sqlite database").unwrap();
    assert!(Registry::open(&state, true).is_err());
    assert_eq!(fs::read_to_string(db).unwrap(), "not a sqlite database");
}
#[test]
fn corrupted_identity_cannot_become_runtime_path() {
    let temp = TempDir::new().unwrap();
    let state = temp.path().join("state");
    let path = checkout(temp.path(), "app");
    let mut registry = Registry::open(&state, true).unwrap().unwrap();
    registry
        .adopt(&manifest(), &path, Role::Preview, "auth")
        .unwrap();
    let connection = rusqlite::Connection::open(state.join("registry.sqlite3")).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    connection
        .execute("UPDATE workspaces SET id = '../../foreign'", [])
        .unwrap();
    assert!(registry.list(None).is_err());
    assert!(registry.current(&path, &manifest()).is_err());
}
#[test]
fn missing_source_is_retained_not_deleted() {
    let temp = TempDir::new().unwrap();
    let path = checkout(temp.path(), "app");
    let mut registry = Registry::open(&temp.path().join("state"), true)
        .unwrap()
        .unwrap();
    registry
        .adopt(&manifest(), &path, Role::Preview, "auth")
        .unwrap();
    fs::remove_dir_all(path).unwrap();
    assert_eq!(registry.list(None).unwrap().len(), 1);
    let output = value(&cli(temp.path(), &temp.path().join("state"), &["ls"]));
    assert_eq!(output["workspaces"][0]["source_exists"], false);
}
#[test]
fn concurrent_cli_adoption_is_idempotent_with_one_owner() {
    let temp = TempDir::new().unwrap();
    let path = checkout(temp.path(), "app");
    let state = temp.path().join("state");
    let barrier = Arc::new(Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            let state = state.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                String::from_utf8(register(&path, &state, &["--name", "auth"]).stderr).unwrap()
            })
        })
        .collect();
    for handle in handles {
        let stderr = handle.join().unwrap();
        assert!(stderr.contains("is the proxy's port"), "{stderr}");
    }
    let registry = Registry::open(&state, false).unwrap().unwrap();
    assert_eq!(registry.list(None).unwrap().len(), 1);
}
#[test]
fn concurrent_conflicting_adoption_has_exactly_one_winner() {
    let temp = TempDir::new().unwrap();
    let a = checkout(temp.path(), "a");
    let b = checkout(temp.path(), "b");
    let state = temp.path().join("state");
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [a, b]
        .into_iter()
        .map(|path| {
            let state = state.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                String::from_utf8(register(&path, &state, &["--primary"]).stderr)
                    .unwrap()
                    .contains("is the proxy's port")
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum::<usize>(),
        1
    );
    let registry = Registry::open(&state, false).unwrap().unwrap();
    assert_eq!(registry.list(None).unwrap().len(), 1);
}

#[test]
fn dangling_state_and_database_symlinks_fail_closed() {
    let temp = TempDir::new().unwrap();
    let state = temp.path().join("state");
    symlink(temp.path().join("absent"), &state).unwrap();
    assert!(Registry::open(&state, false).is_err());
    assert!(Registry::open(&state, true).is_err());
    fs::remove_file(&state).unwrap();
    drop(Registry::open(&state, true).unwrap());
    fs::remove_file(state.join("registry.sqlite3")).unwrap();
    symlink(
        temp.path().join("absent-db"),
        state.join("registry.sqlite3"),
    )
    .unwrap();
    assert!(Registry::open(&state, false).is_err());
    assert!(Registry::open(&state, true).is_err());
}

#[test]
fn foreign_database_is_not_initialized_or_modified() {
    let temp = TempDir::new().unwrap();
    let state = temp.path().join("state");
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let db = state.join("registry.sqlite3");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE unrelated (value TEXT); INSERT INTO unrelated VALUES ('keep');",
        )
        .unwrap();
    drop(connection);
    fs::set_permissions(&db, fs::Permissions::from_mode(0o600)).unwrap();
    let before = fs::read(&db).unwrap();
    assert!(Registry::open(&state, true).is_err());
    assert_eq!(fs::read(&db).unwrap(), before);
}

#[test]
fn url_takes_the_service_like_logs_does() {
    let temp = TempDir::new().unwrap();
    let state = temp.path().join("state");
    let path = checkout(temp.path(), "demo");
    register(&path, &state, &["--name", "auth"]);
    let url = |args: &[&str]| {
        let output = cli(&path, &state, args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    assert_eq!(url(&["url"]), "https://auth-demo.localhost:1443\n");
    assert_eq!(
        url(&["url", "api"]),
        "https://api-auth-demo.localhost:1443\n"
    );
    let unknown = cli(&path, &state, &["url", "nope"]);
    assert!(!unknown.status.success());
}

#[test]
fn workspace_selects_an_environment_by_name_url_or_host() {
    let temp = TempDir::new().unwrap();
    let state = temp.path().join("state");
    let demo = checkout(temp.path(), "demo");
    register(&demo, &state, &["--name", "auth"]);
    let other = checkout(temp.path(), "other");
    fs::write(
        other.join("parcela.yaml"),
        MANIFEST.replace("project: demo", "project: other"),
    )
    .unwrap();
    register(&other, &state, &["--name", "auth"]);
    let id = |path: &Path| value(&cli(path, &state, &["context"]))["workspace"]["id"].clone();
    let select = |selector: &str| {
        Command::new(env!("CARGO_BIN_EXE_parcela"))
            .arg("--state-dir")
            .arg(&state)
            .args(["-w", selector, "context"])
            .output()
            .unwrap()
    };
    for (selector, path) in [
        ("auth-demo", &demo),
        ("auth-demo.localhost", &demo),
        ("https://api-auth-demo.localhost:1443/cart", &demo),
        ("auth-other.local", &other),
        ("HTTPS://AUTH-DEMO.LOCALHOST", &demo),
    ] {
        assert_eq!(
            value(&select(selector))["workspace"]["id"],
            id(path),
            "{selector}"
        );
    }
    let ambiguous = select("auth");
    assert!(!ambiguous.status.success());
    assert!(
        String::from_utf8_lossy(&ambiguous.stderr)
            .contains("auth names 2 environments; use one of: auth-demo, auth-other"),
        "{}",
        String::from_utf8_lossy(&ambiguous.stderr)
    );
    let short = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(&state)
        .arg("-C")
        .arg(&demo)
        .arg("context")
        .output()
        .unwrap();
    assert_eq!(value(&short)["workspace"]["id"], id(&demo));
}

#[test]
fn uuid_target_inspection_and_stop_survive_missing_source() {
    let temp = TempDir::new().unwrap();
    let path = checkout(temp.path(), "app");
    let state = temp.path().join("state");
    register(&path, &state, &["--name", "auth"]);
    let context = value(&cli(&path, &state, &["context"]));
    let id = context["workspace"]["id"].as_str().unwrap();
    fs::remove_file(path.join("parcela.yaml")).unwrap();
    fs::remove_dir(&path).unwrap();
    for command in ["status", "down", "url"] {
        let result = Command::new(env!("CARGO_BIN_EXE_parcela"))
            .arg("--state-dir")
            .arg(&state)
            .args(["--workspace", id, command])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if command == "status" {
            assert_eq!(value(&result)["source_exists"], false);
            assert_eq!(value(&result)["state"], "stopped");
        }
    }
    assert!(!state.join("environments").exists());
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_parcela"))
            .arg("--state-dir")
            .arg(&state)
            .args([
                "--workspace",
                id,
                "--path",
                temp.path().to_str().unwrap(),
                "status"
            ])
            .output()
            .unwrap()
            .status
            .success()
    );
}

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

const CONFIG: &str = "version: 1\nproject: sandbox\nservices:\n  worker: {command: 'exec sleep 300'}\ndata:\n  files: [uploads]\n  sqlite: [app.sqlite3]\ntasks:\n  seed: {command: python3 seed.py}\n";

struct Fixture {
    _temp: TempDir,
    source: PathBuf,
    state: PathBuf,
}
impl Fixture {
    fn new(jj: bool) -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("source");
        let state = root.join("state");
        if jj {
            success(
                Command::new("jj")
                    .args(["git", "init", "--colocate"])
                    .arg(&source),
            );
        } else {
            success(Command::new("git").arg("init").arg(&source));
        }
        fs::write(source.join("parcela.yaml"), CONFIG).unwrap();
        fs::write(source.join("seed.py"), "import os, sqlite3\nfrom pathlib import Path\np=Path(os.environ['PARCELA_DATA_DIR'])\n(p/'uploads').mkdir(exist_ok=True)\n(p/'uploads/fixture.txt').write_text('fixture')\nwith sqlite3.connect(p/'app.sqlite3') as db:\n db.execute('CREATE TABLE IF NOT EXISTS items(value TEXT)')\n db.execute(\"INSERT INTO items VALUES ('seed')\")\n").unwrap();
        if jj {
            success(Command::new("jj").arg("-R").arg(&source).args([
                "--config",
                "signing.behavior='drop'",
                "describe",
                "-m",
                "fixture",
            ]));
        } else {
            success(
                Command::new("git")
                    .arg("-C")
                    .arg(&source)
                    .args(["add", "."]),
            );
            success(Command::new("git").arg("-C").arg(&source).args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-m",
                "fixture",
            ]));
        }
        Self {
            _temp: temp,
            source,
            state,
        }
    }
    fn run(&self, args: &[&str]) -> Output {
        self.at(&self.source, args)
    }
    fn at(&self, path: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_parcela"))
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--path")
            .arg(path)
            .arg("--json")
            .env("PARCELA_NO_TRUST", "1")
            .args(args)
            .output()
            .unwrap()
    }
    fn value(&self, args: &[&str]) -> Value {
        value(self.run(args))
    }
    fn target(&self, id: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_parcela"))
            .arg("--state-dir")
            .arg(&self.state)
            .args(["--json", "--workspace", id])
            .env("PARCELA_NO_TRUST", "1")
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.run(&["down", "--all-projects"]);
        let _ = self.run(&["proxy", "stop"]);
    }
}
fn success(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn value(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn data(f: &Fixture, id: &str) -> PathBuf {
    PathBuf::from(
        value(f.target(id, &["context"]))["runtime_dir"]
            .as_str()
            .unwrap(),
    )
    .join("data")
}

#[test]
fn create_makes_registered_git_and_jj_checkouts_without_starting_them() {
    for jj in [false, true] {
        let f = Fixture::new(jj);
        let made = f.value(&["create", "trial"]);
        let source = PathBuf::from(made["workspace"]["source"].as_str().unwrap());
        let id = made["workspace"]["id"].as_str().unwrap();
        assert_ne!(source, f.source);
        assert_eq!(
            fs::read_to_string(source.join("parcela.yaml")).unwrap(),
            CONFIG
        );
        assert_eq!(made["checkout"]["vcs"], if jj { "jj" } else { "git" });
        assert_eq!(made["workspace"]["label"], "trial");
        assert_eq!(value(f.target(id, &["status"]))["state"], "stopped");
        assert!(!data(&f, id).exists());
        assert!(!f.state.join("proxy").exists());
        let output = f.run(&["create", "trial"]);
        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(source.join("parcela.yaml")).unwrap(),
            CONFIG
        );
        value(f.target(id, &["doctor", "--caddy", "/nonexistent/caddy"]));
        let first = value(f.target(id, &["up", "--detach"]));
        assert_eq!(first["live"]["state"], "ready");
        value(f.target(id, &["down"]));
    }
}

#[test]
fn snapshots_clone_declared_data_and_sqlite_wal_without_copying_secrets_or_overwriting() {
    let f = Fixture::new(true);
    let first = f.value(&["create", "first"]);
    let id = first["workspace"]["id"].as_str().unwrap();
    let seeded = f.target(id, &["task", "seed"]);
    assert!(
        seeded.status.success(),
        "{}",
        String::from_utf8_lossy(&seeded.stderr)
    );
    let path = data(&f, id);
    fs::write(path.join(".env"), "SECRET=do-not-copy").unwrap();
    let db = rusqlite::Connection::open(path.join("app.sqlite3")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO items VALUES ('in WAL');").unwrap();
    assert!(path.join("app.sqlite3-wal").exists());
    let saved = value(f.target(id, &["snapshot", "baseline"]));
    assert_eq!(saved["snapshot"]["name"], "baseline");
    let second = f.value(&["create", "second", "--from", "baseline"]);
    let second_id = second["workspace"]["id"].as_str().unwrap();
    let restored = data(&f, second_id);
    assert_eq!(
        fs::read_to_string(restored.join("uploads/fixture.txt")).unwrap(),
        "fixture"
    );
    assert!(!restored.join(".env").exists());
    let copied = rusqlite::Connection::open(restored.join("app.sqlite3")).unwrap();
    assert_eq!(
        copied
            .query_row("SELECT count(*) FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(
        !f.target(second_id, &["restore", "baseline"])
            .status
            .success()
    );
    assert!(!f.target(id, &["snapshot", "baseline"]).status.success());
    value(f.target(id, &["up", "--detach"]));
    assert!(
        !f.target(id, &["snapshot", "while-running"])
            .status
            .success()
    );
    assert_eq!(value(f.target(id, &["status"]))["state"], "ready");
}

#[test]
fn invalid_creation_inputs_have_no_checkout_or_process_effects() {
    let f = Fixture::new(false);
    for args in [
        vec!["create", "../escape"],
        vec!["create", "trial", "--revision", "missing-revision"],
        vec!["create", "trial", "--from", "missing-snapshot"],
    ] {
        let output = f.run(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(!f.source.parent().unwrap().join("source-trial").exists());
    }
    assert!(!f.state.join("proxy").exists());
}

#[test]
fn restore_detects_corruption_and_never_publishes_partial_data() {
    let f = Fixture::new(false);
    let first = f.value(&["create", "first"]);
    let id = first["workspace"]["id"].as_str().unwrap();
    assert!(f.target(id, &["task", "seed"]).status.success());
    let saved = value(f.target(id, &["snapshot", "baseline"]));
    let snapshot = PathBuf::from(saved["snapshot"]["path"].as_str().unwrap());
    fs::write(snapshot.join("data/uploads/fixture.txt"), "corrupted").unwrap();
    let second = f.value(&["create", "second"]);
    let second_id = second["workspace"]["id"].as_str().unwrap();
    let output = f.target(second_id, &["restore", "baseline"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"));
    assert!(!data(&f, second_id).exists());
    let manifest_path = snapshot.join("manifest.json");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    metadata["entries"][0]["path"] = "../../outside".into();
    parcela::private::write_json(&manifest_path, &metadata).unwrap();
    assert!(
        !f.target(second_id, &["restore", "baseline"])
            .status
            .success()
    );
    assert!(!data(&f, second_id).exists());
    assert!(!f.state.join("outside").exists());
}

#[test]
fn snapshot_refuses_linked_files_secrets_and_raw_sqlite_in_file_directories() {
    use std::os::unix::fs::symlink;
    for kind in ["symlink", "hardlink", "secret", "sqlite"] {
        let f = Fixture::new(false);
        let made = f.value(&["create", "trial"]);
        let id = made["workspace"]["id"].as_str().unwrap();
        assert!(f.target(id, &["task", "seed"]).status.success());
        let path = data(&f, id);
        match kind {
            "symlink" => symlink(path.join("app.sqlite3"), path.join("uploads/link")).unwrap(),
            "hardlink" => fs::hard_link(
                path.join("uploads/fixture.txt"),
                path.join("uploads/second.txt"),
            )
            .unwrap(),
            "secret" => fs::write(path.join("uploads/.env.local"), "secret").unwrap(),
            "sqlite" => {
                fs::copy(path.join("app.sqlite3"), path.join("uploads/raw.db")).unwrap();
            }
            _ => unreachable!(),
        }
        let refused = f.target(id, &["snapshot", "unsafe"]);
        assert!(!refused.status.success(), "{kind}");
        assert!(!f.state.join("snapshots/sandbox/unsafe").exists());
        assert_eq!(
            fs::read_to_string(path.join("uploads/fixture.txt")).unwrap(),
            "fixture"
        );
    }
}

#[test]
fn data_policy_rejects_unsafe_or_overlapping_paths() {
    for path in [
        "../outside",
        "/absolute",
        ".",
        ".env",
        "uploads/.env.local",
        "master.key",
        "key.pem",
        "db.sqlite3-wal",
        "uploads//file",
    ] {
        let config = CONFIG.replace("files: [uploads]", &format!("files: [{path:?}]"));
        assert!(parcela::config::Manifest::parse(&config).is_err(), "{path}");
    }
    for config in [
        CONFIG.replace("sqlite: [app.sqlite3]", "sqlite: [uploads/db.sqlite3]"),
        CONFIG.replace("files: [uploads]", "files: [uploads, uploads/nested]"),
    ] {
        assert!(parcela::config::Manifest::parse(&config).is_err());
    }
}

use parcela::{
    project::config::Manifest,
    state::registry::{Registry, Role},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

mod common;

fn command(path: &Path, state: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_parcela"));
    command
        .arg("--path")
        .arg(path)
        .arg("--state-dir")
        .arg(state)
        .env_remove("PARCELA_HTTPS_PORT")
        .args(args);
    command
}
fn cli(path: &Path, state: &Path, args: &[&str]) -> Output {
    command(path, state, args).output().unwrap()
}
/// Human output, as printed on a terminal.
fn human(path: &Path, state: &Path, args: &[&str]) -> String {
    common::on_terminal(command(path, state, args)).1
}
fn json_of(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

struct Fixture {
    _t: TempDir,
    root: PathBuf,
    state: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let t = TempDir::new().unwrap();
        let root = t.path().canonicalize().unwrap();
        Self {
            state: root.join("state"),
            root,
            _t: t,
        }
    }
    /// Register a checkout with a short-form manifest; returns its source and environment ID.
    fn checkout(
        &self,
        project: &str,
        directory: &str,
        role: Role,
        label: &str,
    ) -> (PathBuf, String) {
        let path = self.root.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("parcela.yaml"),
            format!("version: 1\nproject: {project}\nservices:\n  web: {{command: bin/web, http: {{healthcheck: /}}}}\n  api: {{command: bin/api, http: {{subdomain: api, healthcheck: /}}}}\n"),
        )
        .unwrap();
        let manifest = Manifest::load(&path.join("parcela.yaml")).unwrap();
        let mut registry = Registry::open(&self.state, true).unwrap().unwrap();
        let id = registry.adopt(&manifest, &path, role, label).unwrap().id;
        (path, id)
    }
    fn running(&self, id: &str, port: u16) {
        let mut registry = Registry::open(&self.state, true).unwrap().unwrap();
        let workspace = registry
            .list(None)
            .unwrap()
            .into_iter()
            .find(|workspace| workspace.id == id)
            .unwrap();
        let url = format!("https://{}", workspace.hostname);
        registry
            .begin_session(
                &workspace,
                &uuid::Uuid::new_v4().to_string(),
                port,
                "up",
                &url,
            )
            .unwrap();
    }
    /// The runtime directory a run would have left, holding `bytes` of data.
    fn runtime(&self, id: &str, bytes: usize) -> PathBuf {
        let runtime = self.state.join("environments").join(id);
        parcela::state::private::write_bytes(&runtime.join("data/db.sqlite3"), &vec![1; bytes])
            .unwrap();
        runtime
    }
    fn ids(&self) -> Vec<String> {
        Registry::open(&self.state, false)
            .unwrap()
            .unwrap()
            .list(None)
            .unwrap()
            .into_iter()
            .map(|workspace| workspace.id)
            .collect()
    }
}

#[test]
fn ls_lists_every_project_in_one_table() {
    let f = Fixture::new();
    let (notes, _) = f.checkout("notes", "notes", Role::Primary, "primary");
    let (review, _) = f.checkout("notes", "notes-review", Role::Preview, "review");
    let (shop, _) = f.checkout("shop", "shop", Role::Primary, "primary");
    assert_eq!(
        human(&f.root, &f.state, &["ls"]),
        format!(
            "PROJECT  NAME     STATE    URL                                  PATH\n\
             notes    review   stopped  https://review-notes.localhost:1443  {}\n\
             notes    primary  stopped  https://notes.localhost:1443         {}\n\
             shop     primary  stopped  https://shop.localhost:1443          {}\n",
            review.display(),
            notes.display(),
            shop.display()
        )
    );
    let listed = json_of(&cli(&f.root, &f.state, &["ls"]));
    assert_eq!(listed["workspaces"].as_array().unwrap().len(), 3);
}

#[test]
fn rm_forgets_a_stopped_environment_releases_its_hosts_and_keeps_files() {
    let f = Fixture::new();
    let (review, review_id) = f.checkout("notes", "notes-review", Role::Preview, "review");
    let (_, running_id) = f.checkout("notes", "notes", Role::Primary, "primary");
    f.running(&running_id, 47001);
    let refused = cli(&f.root.join("notes"), &f.state, &["rm"]);
    assert_eq!(refused.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr),
        "parcela: notes is starting; stop it with `parcela down` before removing it\n"
    );
    let removed = json_of(&cli(&review, &f.state, &["rm"]));
    let runtime = f
        .state
        .canonicalize()
        .unwrap()
        .join("environments")
        .join(&review_id);
    assert_eq!(removed["skipped"], json!([]));
    assert_eq!(
        removed["forgotten"][0]["released_hostnames"],
        json!(["api-review-notes.localhost", "review-notes.localhost"])
    );
    assert_eq!(
        removed["forgotten"][0]["kept"],
        json!({"source": review, "runtime_dir": runtime})
    );
    assert_eq!(f.ids(), [running_id]);
    assert_eq!(
        fs::read_to_string(review.join("parcela.yaml")).unwrap(),
        "version: 1\nproject: notes\nservices:\n  web: {command: bin/web, http: {healthcheck: /}}\n  api: {command: bin/api, http: {subdomain: api, healthcheck: /}}\n"
    );
    // The released hostname can be reserved again by another checkout.
    let (_, again) = f.checkout("notes", "elsewhere", Role::Preview, "review");
    assert_ne!(again, review_id);
}

#[test]
fn rm_prune_forgets_only_stopped_registrations_whose_checkout_is_gone() {
    let f = Fixture::new();
    let (kept, kept_id) = f.checkout("notes", "notes", Role::Primary, "primary");
    let (gone, _) = f.checkout("notes", "gone", Role::Preview, "gone");
    let (live_gone, live_id) = f.checkout("notes", "live-gone", Role::Preview, "live");
    f.running(&live_id, 47002);
    fs::remove_dir_all(&gone).unwrap();
    fs::remove_dir_all(&live_gone).unwrap();
    let text = human(&kept, &f.state, &["rm", "--prune"]);
    let runtime = f.state.canonicalize().unwrap().join("environments");
    assert!(
        text.starts_with("✓ Forgot gone (notes); released api-gone-notes.localhost, gone-notes.localhost\n  data kept in "),
        "{text}"
    );
    assert!(text.contains(&runtime.display().to_string()));
    assert!(text.ends_with(
        "✗ Skipped live (notes): still running; stop it with `parcela down --workspace ID`\n"
    ));
    let mut ids = f.ids();
    ids.sort();
    let mut expected = vec![kept_id, live_id];
    expected.sort();
    assert_eq!(ids, expected);
}

#[test]
fn down_all_selects_running_environments_by_project_and_reports_each() {
    let f = Fixture::new();
    let (notes, _) = f.checkout("notes", "notes", Role::Primary, "primary");
    f.checkout("shop", "shop", Role::Primary, "primary");
    assert_eq!(
        json_of(&cli(&notes, &f.state, &["down", "--all-projects"])),
        json!({"environments": []})
    );
    assert_eq!(
        human(&notes, &f.state, &["down", "--all"]),
        "Nothing is running.\n"
    );
    let rejected = cli(
        &notes,
        &f.state,
        &[
            "down",
            "--all",
            "--workspace",
            &uuid::Uuid::new_v4().to_string(),
        ],
    );
    assert_eq!(rejected.status.code(), Some(2));
}

#[test]
fn status_overview_works_anywhere_and_explicit_selection_keeps_process_details() {
    let f = Fixture::new();
    let (notes, id) = f.checkout("notes", "notes", Role::Primary, "primary");
    let (review, _) = f.checkout("notes", "notes-review", Role::Preview, "review");
    f.checkout("shop", "shop", Role::Primary, "primary");
    let before = f.ids();
    let overview = json_of(&cli(&review, &f.state, &["status"]));
    let rows = overview["workspaces"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["workspace"]["label"], "review");
    assert_eq!(rows[0]["current"], true);
    assert_eq!(rows[1]["workspace"]["project_id"], "notes");
    assert_eq!(rows[2]["workspace"]["project_id"], "shop");
    assert!(human(&review, &f.state, &["status"]).contains("review *"));
    assert_eq!(
        json_of(&cli(&f.root, &f.state, &["status"]))["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let current = json_of(&cli(&notes, &f.state, &["status", "--current"]));
    assert_eq!(current["workspace"]["id"], id);
    assert_eq!(current["processes"], json!([]));
    assert_eq!(current["state"], "stopped");
    let selected = Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--state-dir")
        .arg(&f.state)
        .args(["--workspace", &id, "status"])
        .output()
        .unwrap();
    assert_eq!(json_of(&selected)["workspace"]["id"], id);
    assert_eq!(f.ids(), before);
}

#[test]
fn status_overview_does_not_create_state_or_register_unstarted_checkouts() {
    let f = Fixture::new();
    let empty = json_of(&cli(&f.root, &f.state, &["status"]));
    assert_eq!(empty["workspaces"], json!([]));
    assert!(!f.state.exists());
    let path = f.root.join("unstarted");
    fs::create_dir(&path).unwrap();
    fs::write(
        path.join("parcela.yaml"),
        "version: 1\nproject: fresh\nservices: {web: {command: unused, http: {healthcheck: /}}}\n",
    )
    .unwrap();
    let overview = json_of(&cli(&path, &f.state, &["status"]));
    assert_eq!(overview["unregistered"][0]["state"], "not started");
    assert_eq!(overview["unregistered"][0]["current"], true);
    assert!(!f.state.exists());
}

#[test]
fn dashboard_hostname_cannot_be_adopted_by_an_application() {
    let f = Fixture::new();
    let path = f.root.join("collision");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("parcela.yaml"), "version: 1\nproject: parcela-dashboard\nservices: {web: {command: unused, http: {healthcheck: /}}}\n").unwrap();
    let manifest = Manifest::load(&path.join("parcela.yaml")).unwrap();
    let mut registry = Registry::open(&f.state, true).unwrap().unwrap();
    let error = registry
        .adopt(&manifest, &path, Role::Primary, "primary")
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("reserved for the Parcela dashboard")
    );
    assert!(registry.list(None).unwrap().is_empty());
}

#[test]
fn rm_data_deletes_the_runtime_directory_only_once_the_environment_is_forgotten() {
    let f = Fixture::new();
    let (review, review_id) = f.checkout("notes", "notes-review", Role::Preview, "review");
    let (notes, running_id) = f.checkout("notes", "notes", Role::Primary, "primary");
    let review_data = f.runtime(&review_id, 64 * 1024);
    let running_data = f.runtime(&running_id, 1024);
    f.running(&running_id, 47003);
    let refused = cli(&notes, &f.state, &["rm", "--data"]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(running_data.is_dir());

    let removed = json_of(&cli(&review, &f.state, &["rm", "--data"]));
    let row = &removed["forgotten"][0];
    assert_eq!(row["kept"], json!({"source": review}));
    assert_eq!(row["deleted"]["runtime_dir"], json!(review_data));
    assert!(
        row["deleted"]["bytes"].as_u64().unwrap() >= 64 * 1024,
        "{row}"
    );
    assert!(!review_data.exists());
    assert!(review.join("parcela.yaml").is_file());
    assert_eq!(f.ids(), [running_id]);
    // Created but never started: there is no directory to delete.
    let (shop, _) = f.checkout("shop", "shop", Role::Primary, "primary");
    let removed = json_of(&cli(&shop, &f.state, &["rm", "--data"]));
    assert_eq!(removed["forgotten"][0]["deleted"], Value::Null);
    assert_eq!(removed["forgotten"][0]["kept"], json!({"source": shop}));
    assert!(human(&f.root, &f.state, &["rm", "--orphans"]).starts_with("No orphaned data in "));
}

#[test]
fn rm_prune_data_deletes_the_data_of_each_forgotten_environment() {
    let f = Fixture::new();
    let (kept, kept_id) = f.checkout("notes", "notes", Role::Primary, "primary");
    let (gone, gone_id) = f.checkout("notes", "gone", Role::Preview, "gone");
    let kept_data = f.runtime(&kept_id, 1024);
    let gone_data = f.runtime(&gone_id, 1024);
    fs::remove_dir_all(&gone).unwrap();
    let text = human(&kept, &f.state, &["rm", "--prune", "--data"]);
    assert!(
        text.starts_with("✓ Forgot gone (notes); released api-gone-notes.localhost, gone-notes.localhost\n  deleted "),
        "{text}"
    );
    assert!(
        text.contains(&format!("of data in {}", gone_data.display())),
        "{text}"
    );
    assert!(!gone_data.exists());
    assert!(kept_data.is_dir());
    assert_eq!(f.ids(), [kept_id]);
}

#[test]
fn rm_orphans_lists_unowned_data_and_deletes_it_only_with_data() {
    let f = Fixture::new();
    let (notes, owned_id) = f.checkout("notes", "notes", Role::Primary, "primary");
    let (review, review_id) = f.checkout("notes", "notes-review", Role::Preview, "review");
    let owned = f.runtime(&owned_id, 1024);
    let orphan = f.runtime(&review_id, 64 * 1024);
    parcela::state::private::write_json(
        &orphan.join("context.json"),
        &json!({"source": review, "environment": {"PARCELA_PROJECT_ID": "notes", "PARCELA_ROLE": "preview", "PARCELA_ENVIRONMENT_NAME": "review"}}),
    )
    .unwrap();
    json_of(&cli(&review, &f.state, &["rm"]));
    let unknown = f.runtime(&uuid::Uuid::new_v4().to_string(), 1024);

    let listed = json_of(&cli(&notes, &f.state, &["rm", "--orphans"]));
    let rows = listed["orphans"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{listed}");
    assert_eq!(rows[0]["id"], review_id);
    assert_eq!(rows[0]["path"], json!(orphan));
    assert_eq!(rows[0]["name"], "review");
    assert_eq!(rows[0]["source"], json!(review));
    assert_eq!(rows[1]["path"], json!(unknown));
    assert_eq!(rows[1]["project"], Value::Null);
    let text = human(&notes, &f.state, &["rm", "--orphans"]);
    assert!(
        text.starts_with(&format!(
            "2 data directories in {} no environment owns, ",
            f.state.join("environments").display()
        )),
        "{text}"
    );
    assert!(text.contains(&format!("  {review_id}  ")), "{text}");
    assert!(text.contains("just now  review (notes)  "), "{text}");
    assert!(
        text.ends_with("Delete them with parcela rm --orphans --data\n"),
        "{text}"
    );
    assert!(orphan.is_dir() && unknown.is_dir());

    let doctor = cli(
        &notes,
        &f.state,
        &["doctor", "--caddy", "/nonexistent/caddy"],
    );
    let report: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(report["orphaned_data"]["directories"], 2);
    assert!(
        report["notes"].as_array().unwrap().iter().any(|note| note
            .as_str()
            .unwrap()
            .starts_with("2 data directories no environment owns hold ")),
        "{report}"
    );

    let deleted = json_of(&cli(&notes, &f.state, &["rm", "--orphans", "--data"]));
    assert_eq!(deleted["deleted"].as_array().unwrap().len(), 2);
    assert_eq!(deleted["failed"], json!([]));
    assert!(!orphan.exists() && !unknown.exists());
    assert!(owned.join("data/db.sqlite3").is_file());
    assert_eq!(
        human(&notes, &f.state, &["rm", "--orphans"]),
        format!(
            "No orphaned data in {}.\n",
            f.state.join("environments").display()
        )
    );
    let report: Value = serde_json::from_slice(
        &cli(
            &notes,
            &f.state,
            &["doctor", "--caddy", "/nonexistent/caddy"],
        )
        .stdout,
    )
    .unwrap();
    assert_eq!(
        report["orphaned_data"],
        json!({"directories": 0, "bytes": 0})
    );
}

#[test]
fn rm_orphans_refuses_to_judge_ownership_without_a_registry() {
    let f = Fixture::new();
    let (_, id) = f.checkout("notes", "notes", Role::Primary, "primary");
    let runtime = f.runtime(&id, 1024);
    fs::remove_file(f.state.join("registry.sqlite3")).unwrap();
    let refused = cli(&f.root, &f.state, &["rm", "--orphans", "--data"]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("registry is missing"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(runtime.is_dir());
}

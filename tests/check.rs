use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn parcela(path: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--path")
        .arg(path)
        .arg("--state-dir")
        .arg(path.join("state"))
        .args(args)
        .output()
        .unwrap()
}

/// The doctor's config check as `{ok, detail}`.
fn doctor_config(path: &Path) -> (bool, String) {
    let report: Value =
        serde_json::from_slice(&parcela(path, &["--json", "doctor"]).stdout).unwrap();
    let check = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "config")
        .unwrap();
    (
        check["ok"].as_bool().unwrap(),
        check["detail"].as_str().unwrap().to_owned(),
    )
}

#[test]
fn up_and_doctor_report_the_same_configuration_error() {
    let head = "version: 1\nproject: app\nservices:\n";
    for (services, expected) in [
        (
            "  web: {command: bin/web, http: {subdomain: www, healthcheck: /}}\n",
            "invalid {path}/parcela.yaml: one service needs http without a subdomain, like `http: {healthcheck: /}`; it gets the environment's URL",
        ),
        (
            "  web: {command: bin/web, http: {healthcheck: /}, depends_on: [db]}\n",
            "invalid {path}/parcela.yaml: service web depends on db, which is not a service",
        ),
    ] {
        let t = TempDir::new().unwrap();
        let path = t.path().canonicalize().unwrap();
        fs::write(path.join("parcela.yaml"), format!("{head}{services}")).unwrap();
        let expected = expected.replace("{path}", &path.display().to_string());
        let up = parcela(&path, &["up", "--backend-port", "1443"]);
        assert_eq!(up.status.code(), Some(1), "{services}");
        assert_eq!(
            String::from_utf8(up.stderr).unwrap(),
            format!("parcela: {expected}\n"),
            "{services}"
        );
        assert_eq!(doctor_config(&path), (false, expected), "{services}");
        assert!(!path.join("state").exists(), "{services}");
    }
}

#[test]
fn every_bundled_example_passes_the_doctor_config_check() {
    // The repository's own parcela.yaml runs the docs site.
    for example in ["examples/docs-demo", "examples/rails-postgres", "."] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(example);
        let report: Value = serde_json::from_slice(
            &Command::new(env!("CARGO_BIN_EXE_parcela"))
                .arg("--path")
                .arg(&path)
                .args([
                    "--state-dir",
                    "/nonexistent/parcela-state",
                    "--json",
                    "doctor",
                ])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let config = report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == "config")
            .unwrap();
        assert_eq!(config["ok"], true, "{example}: {config}");
    }
}

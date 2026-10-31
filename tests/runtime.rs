use parcela::{
    config::Manifest,
    frontdoor::FrontDoor,
    private,
    registry::{Registry, Role, SessionStatus},
    router::{self, Router},
    runtime,
};
use std::{collections::BTreeMap, fs, os::unix::fs::symlink};
use tempfile::TempDir;
use uuid::Uuid;
fn fixture() -> (TempDir, Registry, Manifest, parcela::registry::Workspace) {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let manifest: Manifest = Manifest::parse("version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /up}}}\nenv: {DATA_DIR: '${PARCELA_RUNTIME_DIR}/data'}\n").unwrap();
    let mut registry = Registry::open(&temp.path().join("state"), true)
        .unwrap()
        .unwrap();
    let workspace = registry
        .adopt(&manifest, &source, Role::Preview, "auth")
        .unwrap();
    (temp, registry, manifest, workspace)
}
#[test]
fn lease_token_generation_and_interrupted_ownership() {
    let (_temp, mut registry, _manifest, workspace) = fixture();
    let token = Uuid::new_v4().to_string();
    let session = registry
        .begin_session(
            &workspace,
            &token,
            32001,
            "run",
            "https://auth-app.localhost",
        )
        .unwrap();
    assert_eq!(session.generation, 2);
    assert!(
        registry
            .begin_session(
                &workspace,
                &token,
                32002,
                "run",
                "https://auth-app.localhost"
            )
            .is_err()
    );
    let mut wrong = session.clone();
    wrong.token = Uuid::new_v4().to_string();
    assert!(
        registry
            .session_status(&wrong, SessionStatus::Stopped)
            .is_err()
    );
    registry
        .session_status(&session, SessionStatus::Ready)
        .unwrap();
    registry
        .session_status(&session, SessionStatus::Stopped)
        .unwrap();
    let next = registry
        .begin_session(
            &workspace,
            &token,
            32002,
            "run",
            "https://auth-app.localhost",
        )
        .unwrap();
    assert_eq!(next.generation, 3);
    assert!(
        registry
            .session_status(&session, SessionStatus::Ready)
            .is_err()
    );
}
#[test]
fn port_conflict_rolls_back_generation() {
    let (temp, mut registry, manifest, workspace) = fixture();
    let other = temp.path().join("other");
    fs::create_dir(&other).unwrap();
    let other = registry
        .adopt(&manifest, &other, Role::Preview, "other")
        .unwrap();
    let token = Uuid::new_v4().to_string();
    registry
        .begin_session(
            &workspace,
            &token,
            32001,
            "run",
            "https://auth-app.localhost",
        )
        .unwrap();
    assert!(
        registry
            .begin_session(&other, &token, 32001, "run", "https://other-app.localhost")
            .is_err()
    );
    assert_eq!(
        registry
            .current(&other.source, &manifest)
            .unwrap()
            .generation,
        1
    );
    assert!(registry.session(&other.id).unwrap().is_none());
}
#[test]
fn caddy_config_is_exact_owned_loopback_no_trust_no_persist() {
    let (_temp, mut registry, _manifest, workspace) = fixture();
    let router = Router {
        token: Uuid::new_v4().to_string(),
        front_door: FrontDoor::on(34443),
        version: router::VERSION.into(),
        protocol: router::PROTOCOL,
    };
    let session = registry
        .begin_session(
            &workspace,
            &router.token,
            32001,
            "run",
            &router.url(&workspace.hostname),
        )
        .unwrap();
    let before = router::configuration(&registry, &router).unwrap();
    assert_eq!(
        before["apps"]["http"]["servers"]["https"]["routes"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    registry
        .session_status(&session, SessionStatus::Ready)
        .unwrap();
    let config = router::configuration(&registry, &router).unwrap();
    assert_eq!(
        session.public_url.as_deref(),
        Some("https://auth-app.localhost:34443")
    );
    assert_eq!(
        config["apps"]["http"]["servers"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["https"],
        "port mode serves no plain-HTTP listener"
    );
    assert!(config["apps"]["http"].get("http_port").is_none());
    assert_eq!(config["admin"]["config"]["persist"], false);
    assert_eq!(
        config["apps"]["pki"]["certificate_authorities"]["local"]["install_trust"],
        false
    );
    assert_eq!(
        config["apps"]["http"]["servers"]["https"]["listen"][0],
        "127.0.0.1:34443"
    );
    assert_eq!(
        config["apps"]["http"]["servers"]["https"]["routes"][0]["match"][0]["host"][0],
        parcela::dashboard::HOST
    );
    assert_eq!(
        config["apps"]["tls"]["automation"]["policies"][0]["issuers"][0]["module"],
        "internal"
    );
    registry
        .session_status(&session, SessionStatus::Stopping)
        .unwrap();
    assert_eq!(router::configuration(&registry,&router).unwrap()["apps"]["http"]["servers"]["https"]["routes"].as_array().unwrap().len(),3);
    let replacement = Router {
        token: Uuid::new_v4().to_string(),
        ..router
    };
    registry
        .session_status(&session, SessionStatus::Ready)
        .unwrap();
    assert_eq!(
        router::configuration(&registry, &replacement).unwrap()["apps"]["http"]["servers"]["https"]
            ["routes"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
#[test]
fn context_placeholders_reserved_names_and_readiness_validation() {
    let (_temp, registry, mut manifest, workspace) = fixture();
    let env = runtime::environment(
        &manifest,
        &workspace,
        &registry.runtime_path(&workspace),
        "https://auth-app.localhost",
        Some(32001),
        &[],
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    assert!(env["DATA_DIR"].ends_with("/data"));
    assert_eq!(env["PORT"], "32001");
    assert_eq!(env["HOST"], "127.0.0.1");
    assert_eq!(env["PARCELA_ROLE"], "preview");
    manifest.environment.insert("PORT".into(), "1234".into());
    assert!(manifest.validate().is_err());
    manifest.environment.remove("PORT");
    manifest
        .environment
        .insert("MISSING".into(), "${WRONG}".into());
    assert!(
        runtime::environment(
            &manifest,
            &workspace,
            &registry.runtime_path(&workspace),
            "https://auth-app.localhost",
            None,
            &[],
            &Default::default(),
            &Default::default(),
        )
        .is_err()
    );
    manifest.services.get_mut("web").unwrap().readiness_path = "/up\r\nBad: true".into();
    assert!(manifest.validate().is_err());
}
#[test]
fn kernel_locks_and_private_files_reject_symlinks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("private");
    private::directory(&root).unwrap();
    let path = root.join("lock");
    let lock = private::Lock::acquire(&path).unwrap();
    assert!(private::Lock::acquire(&path).is_err());
    assert!(private::Lock::try_acquire(&path).unwrap().is_none());
    drop(lock);
    assert!(private::Lock::try_acquire(&path).unwrap().is_some());
    let link = root.join("link");
    symlink(&path, &link).unwrap();
    assert!(private::Lock::acquire(&link).is_err());
    assert!(private::Lock::try_acquire(&link).is_err());
    let metadata = root.join("metadata.json");
    private::write_json(&metadata, &serde_json::json!({"id":"owned"})).unwrap();
    let value: serde_json::Value = private::read_json(&metadata).unwrap();
    assert_eq!(value["id"], "owned");
    let linked = root.join("linked.json");
    symlink(&metadata, &linked).unwrap();
    assert!(private::read_json::<serde_json::Value>(&linked).is_err());
    assert!(private::write_json(&linked, &serde_json::json!({"id":"wrong"})).is_err());
}
#[test]
fn port_443_urls_carry_no_port() {
    let (_temp, registry, _manifest, _workspace) = fixture();
    let router = Router {
        token: Uuid::new_v4().to_string(),
        front_door: FrontDoor::on(443),
        version: router::VERSION.into(),
        protocol: router::PROTOCOL,
    };
    assert_eq!(
        router.url("auth-app.localhost"),
        "https://auth-app.localhost"
    );
    let config = router::configuration(&registry, &router).unwrap();
    assert_eq!(
        config["apps"]["http"]["servers"]["https"]["listen"][0],
        "127.0.0.1:443"
    );
    assert!(config["apps"]["http"]["servers"].get("redirect").is_none());
}

#[test]
fn service_hosts_and_port_leases_are_atomic_for_the_whole_set() {
    use parcela::registry::Endpoint;
    let (temp, mut registry, mut manifest, workspace) = fixture();
    manifest.services.insert(
        "backend".into(),
        parcela::config::Service {
            subdomain: Some("api".into()),
            readiness_path: "/ready".into(),
            wildcard: false,
        },
    );
    registry
        .adopt(&manifest, &workspace.source, Role::Preview, "auth")
        .unwrap();
    let hosts = manifest.service_hosts(&workspace.hostname).unwrap();
    assert_eq!(hosts["backend"], "api-auth-app.localhost");
    let token = Uuid::new_v4().to_string();
    let endpoints = vec![
        Endpoint {
            service: "web".into(),
            hostname: workspace.hostname.clone(),
            backend_port: 32010,
            public_url: "https://auth-app.localhost".into(),
        },
        Endpoint {
            service: "backend".into(),
            hostname: hosts["backend"].clone(),
            backend_port: 32011,
            public_url: "https://api-auth-app.localhost".into(),
        },
    ];
    let session = registry
        .begin_endpoints(
            &workspace,
            &token,
            32010,
            "up",
            "https://auth-app.localhost",
            &endpoints,
            &BTreeMap::from([("db".into(), 32013)]),
        )
        .unwrap();
    let recorded = registry.session(&workspace.id).unwrap().unwrap();
    assert_eq!(recorded.endpoints, endpoints);
    assert_eq!(recorded.ports, BTreeMap::from([("db".into(), 32013)]));
    let other = temp.path().join("other");
    fs::create_dir(&other).unwrap();
    let other = registry
        .adopt(&manifest, &other, Role::Preview, "other")
        .unwrap();
    assert!(
        registry
            .begin_session(&other, &token, 32011, "run", "https://other-app.localhost")
            .is_err()
    );
    assert!(
        registry
            .begin_session(&other, &token, 32013, "run", "https://other-app.localhost")
            .is_err(),
        "a named port is leased like an endpoint"
    );
    let other_endpoints = vec![
        Endpoint {
            service: "web".into(),
            hostname: other.hostname.clone(),
            backend_port: 32012,
            public_url: "https://other-app.localhost".into(),
        },
        Endpoint {
            service: "backend".into(),
            hostname: "api-other-app.localhost".into(),
            backend_port: 32011,
            public_url: "https://api-other-app.localhost".into(),
        },
    ];
    assert!(
        registry
            .begin_endpoints(
                &other,
                &token,
                32012,
                "up",
                "https://other-app.localhost",
                &other_endpoints,
                &BTreeMap::new()
            )
            .is_err()
    );
    assert!(registry.session(&other.id).unwrap().is_none());
    let database =
        rusqlite::Connection::open(registry.state_path().join("registry.sqlite3")).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT count(*) FROM active_ports WHERE port=32012",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );

    assert_eq!(
        registry
            .current(&other.source, &manifest)
            .unwrap()
            .generation,
        1
    );
    assert!(
        registry
            .adopt(&manifest, &workspace.source, Role::Preview, "auth")
            .is_ok()
    );
    manifest.services.remove("backend");
    assert!(
        registry
            .adopt(&manifest, &workspace.source, Role::Preview, "auth")
            .is_err()
    );
    registry
        .session_status(&session, SessionStatus::Stopped)
        .unwrap();
    let next = registry
        .begin_session(&other, &token, 32011, "run", "https://other-app.localhost")
        .unwrap();
    assert_eq!(next.backend_port, Some(32011));
    assert!(
        registry
            .session_status(&session, SessionStatus::Ready)
            .is_err()
    );
    registry
        .session_status(&session, SessionStatus::Stopped)
        .unwrap();
    assert!(
        registry
            .begin_session(
                &workspace,
                &token,
                32011,
                "run",
                "https://auth-app.localhost"
            )
            .is_err()
    );
}

#[test]
fn secondary_hostname_conflicts_with_preview_base_without_partial_adoption() {
    let (temp, mut registry, mut manifest, workspace) = fixture();
    manifest.services.insert(
        "api".into(),
        parcela::config::Service {
            subdomain: None,
            readiness_path: "/".into(),
            wildcard: false,
        },
    );
    registry
        .adopt(&manifest, &workspace.source, Role::Preview, "auth")
        .unwrap();
    let other = temp.path().join("collision");
    fs::create_dir(&other).unwrap();
    assert!(
        registry
            .adopt(&manifest, &other, Role::Preview, "api-auth")
            .is_err()
    );
    assert_eq!(registry.list(None).unwrap().len(), 1);
    assert_eq!(
        registry.hosts(&workspace.id).unwrap()["api"],
        "api-auth-app.localhost"
    );
    assert!(
        manifest
            .service_hosts(&format!("{}.localhost", "a".repeat(63)))
            .is_err()
    );
}

#[test]
fn task_runtime_has_only_generic_dirs_and_strips_only_parcela_variables() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let yaml = "version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\ntasks:\n  probe:\n    command: 'cd \"$PARCELA_RUNTIME_DIR\" && ls -p && find tmp -mindepth 1 && echo \"${PORTLESS_PROBE-unset} ${PARCELA_PROBE-unset}\"'\n    timeout_seconds: 10\n";
    fs::write(source.join("parcela.yaml"), yaml).unwrap();
    let manifest: Manifest = Manifest::parse(yaml).unwrap();
    let state = temp.path().join("state");
    Registry::open(&state, true)
        .unwrap()
        .unwrap()
        .adopt(&manifest, &source, Role::Preview, "probe")
        .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_parcela"))
        .arg("--path")
        .arg(&source)
        .arg("--state-dir")
        .arg(&state)
        .args(["task", "probe"])
        .env("PORTLESS_PROBE", "inherited")
        .env("PARCELA_PROBE", "inherited")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "bin/\ncache/\ndata/\nlog/\ntask.lock\ntmp/\ninherited unset\n"
    );
}

#[test]
fn an_older_registry_schema_is_refused_unchanged() {
    let (temp, registry, _manifest, _workspace) = fixture();
    drop(registry);
    let state = temp.path().join("state");
    let connection = rusqlite::Connection::open(state.join("registry.sqlite3")).unwrap();
    connection.execute_batch("PRAGMA user_version=3;").unwrap();
    drop(connection);
    for create in [false, true] {
        let error = Registry::open(&state, create).err().unwrap();
        assert!(
            error
                .to_string()
                .starts_with("unsupported registry schema 3"),
            "{error}"
        );
    }
    let connection = rusqlite::Connection::open(state.join("registry.sqlite3")).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 3);
}

use parcela::{
    config::Manifest,
    registry::Endpoint,
    supervisor::{Condition, Kind, Probe, Program, plan},
};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path, process::Command};
use tempfile::TempDir;

const SHOP: &str = "\
version: 1
project: shop
env:
  DATABASE_URL: postgres://127.0.0.1:${PARCELA_PORT_DB}/shop
ports: [db]
services:
  db:
    command: postgres -D \"$PARCELA_DATA_DIR/postgres\" -p \"$PARCELA_PORT_DB\"
    healthcheck: pg_isready -h 127.0.0.1 -p \"$PARCELA_PORT_DB\"
  web:
    command: bin/rails server -p $PORT
    http:
      healthcheck: /up
    depends_on: [db]
  api:
    command: node api.js
    working_dir: api
    env:
      API_NAME: api-${PARCELA_ENVIRONMENT_NAME}
    http:
      subdomain: api
      healthcheck: /health
  jobs:
    command: bin/jobs
    depends_on: [db]
hooks:
  setup:
    command: bin/rails db:setup
    depends_on: [db]
  prestart:
    command: bin/rails db:migrate
    depends_on: [db]
tasks:
  test:
    command: bin/rails test
    timeout_seconds: 600
x-notes: anything
";

fn error(text: &str) -> String {
    format!("{:#}", Manifest::parse(text).unwrap_err())
}

#[test]
fn a_file_with_every_field_becomes_the_runtime_shape() {
    let manifest = Manifest::parse(SHOP).unwrap();
    let value = serde_json::to_value(&manifest).unwrap();
    assert_eq!(value["project"], json!({"id": "shop", "name": "shop"}));
    assert_eq!(value["default_service"], "web");
    assert_eq!(
        value["services"],
        json!({
            "api": {"subdomain": "api", "readiness_path": "/health"},
            "web": {"subdomain": null, "readiness_path": "/up"},
        })
    );
    assert_eq!(
        value["processes"]["db"]["healthcheck"],
        "pg_isready -h 127.0.0.1 -p \"$PARCELA_PORT_DB\""
    );
    assert_eq!(value["processes"]["api"]["working_dir"], "api");
    assert_eq!(value["hooks"]["setup"]["depends_on"], json!(["db"]));
    assert_eq!(value["tasks"]["test"]["command"], "bin/rails test");
    assert_eq!(value["ports"], json!(["db"]));
}

#[test]
fn hooks_run_after_their_dependencies_and_before_everything_else() {
    let manifest = Manifest::parse(SHOP).unwrap();
    let endpoints = [("web", 21000), ("api", 21001)].map(|(service, port)| Endpoint {
        service: service.into(),
        hostname: format!("{service}.localhost"),
        backend_port: port,
        public_url: format!("https://{service}.localhost:1443"),
    });
    let env = BTreeMap::from([
        ("PARCELA_ENVIRONMENT_NAME".to_owned(), "signup".to_owned()),
        ("PORT".to_owned(), "21000".to_owned()),
    ]);
    let source = Path::new("/src/shop");
    let graph = plan(
        &manifest,
        &endpoints,
        &env,
        source,
        Path::new("/state/env"),
        true,
        &Default::default(),
    )
    .unwrap();
    let node = |name: &str| graph.iter().find(|node| node.name == name).unwrap();
    let names: Vec<&str> = graph.iter().map(|node| node.name.as_str()).collect();
    assert_eq!(names, ["setup", "prestart", "api", "db", "jobs", "web"]);
    assert_eq!(node("setup").kind, Kind::Hook);
    assert_eq!(
        node("setup").program,
        Program::Shell("(\nbin/rails db:setup\n)\nstatus=$?\nprintf %s \"$status\" > '/state/env/hooks/setup.exit'\nexit \"$status\"".into())
    );
    assert_eq!(
        node("setup").depends_on,
        [("db".into(), Condition::Healthy)]
    );
    assert_eq!(
        node("prestart").depends_on,
        [
            ("db".into(), Condition::Healthy),
            ("setup".into(), Condition::Completed)
        ]
    );
    assert!(node("db").depends_on.is_empty());
    assert_eq!(
        node("web").depends_on,
        [
            ("db".into(), Condition::Healthy),
            ("prestart".into(), Condition::Completed)
        ]
    );
    assert_eq!(node("web").env["PORT"], "21000");
    assert!(!node("db").env.contains_key("PORT"));
    assert_eq!(
        node("web").probe,
        Some(Probe::Http {
            port: 21000,
            host: "web.localhost".into(),
            path: "/up".into()
        })
    );
    assert_eq!(
        node("db").probe,
        Some(Probe::Exec(
            "pg_isready -h 127.0.0.1 -p \"$PARCELA_PORT_DB\"".into()
        ))
    );
    assert_eq!(node("api").env["PORT"], "21001");
    assert_eq!(node("api").env["API_NAME"], "api-signup");
    assert_eq!(node("api").working_dir, source.join("api"));
    assert_eq!(node("web").working_dir, source);

    let graph = plan(
        &manifest,
        &endpoints,
        &env,
        source,
        Path::new("/state/env"),
        false,
        &Default::default(),
    )
    .unwrap();
    assert!(graph.iter().all(|node| node.name != "setup"));
    assert_eq!(graph[0].depends_on, [("db".to_owned(), Condition::Healthy)]);
}

#[test]
fn every_rule_has_one_error() {
    let minimal =
        "version: 1\nproject: app\nservices:\n  web: {command: bin/web, http: {healthcheck: /}}\n";
    Manifest::parse(minimal).unwrap();
    for (text, expected) in [
        (
            "project: app\nservices: {}\n".to_owned(),
            "add `version: 1` at the top of parcela.yaml",
        ),
        (
            minimal.replace("version: 1", "version: 2"),
            "unsupported version 2; this Parcela reads version 1",
        ),
        (
            minimal.replace("project: app\n", ""),
            "missing field `project`",
        ),
        (
            minimal.replace(
                "http: {healthcheck: /}",
                "http: {subdomain: www, healthcheck: /}",
            ),
            "one service needs http without a subdomain, like `http: {healthcheck: /}`; it gets the environment's URL",
        ),
        (
            format!("{minimal}  admin: {{command: bin/admin, http: {{healthcheck: /}}}}\n"),
            "only one http service can leave out subdomain; it gets the environment's own URL",
        ),
        (
            minimal.replace(
                "http: {healthcheck: /}",
                "http: {healthcheck: /}, healthcheck: 'true'",
            ),
            "service web has http, so its readiness is http.healthcheck; remove healthcheck",
        ),
        (
            minimal.replace("command: bin/web,", "command: bin/web, depends_on: [db],"),
            "service web depends on db, which is not a service",
        ),
        (
            format!(
                "{minimal}  jobs: {{command: bin/jobs, depends_on: [worker]}}\n  worker: {{command: bin/worker, depends_on: [jobs]}}\n"
            ),
            "depends_on forms a cycle: jobs -> worker -> jobs",
        ),
        (
            format!("{minimal}hooks:\n  setup: {{command: bin/setup, depends_on: [db]}}\n"),
            "hooks.setup depends on db, which is not a service",
        ),
        (
            format!("{minimal}hooks:\n  teardown: {{command: bin/drop, depends_on: [web]}}\n"),
            "hooks.teardown runs after every service stopped, so it cannot use depends_on",
        ),
        (
            format!("{minimal}  setup: {{command: bin/setup}}\n"),
            "setup is reserved for hooks.setup; rename the service",
        ),
        (
            format!("{minimal}env: {{PORT: '1'}}\n"),
            "invalid env: PORT is set by Parcela",
        ),
        (
            minimal.replace("healthcheck: /", "healthcheck: up"),
            "service web: http.healthcheck must be a path like /up",
        ),
        (
            format!("{minimal}  db: {{}}\n"),
            "service db needs a command",
        ),
        (
            format!(
                "{minimal}  db: {{command: postgres, compose: {{file: compose.yml, services: [db]}}}}\n"
            ),
            "service db has both command and compose; keep one",
        ),
        (
            format!(
                "{minimal}  db: {{compose: {{file: compose.yml, services: [db]}}, stop: 'true'}}\n"
            ),
            "service db has compose, so Parcela runs, checks and stops it; remove http, healthcheck, stop and working_dir",
        ),
        (
            format!("{minimal}  db: {{compose: {{file: compose.yml, services: []}}}}\n"),
            "service db: compose needs at least one Compose service in services or http",
        ),
        (
            format!("{minimal}  db: {{compose: {{file: /srv/compose.yml, services: [db]}}}}\n"),
            "service db: compose.file must be a path relative to parcela.yaml",
        ),
        (
            format!("{minimal}  db: {{compose: {{file: compose.yml, services: ['db; rm']}}}}\n"),
            "service db: invalid Compose service name \"db; rm\"",
        ),
        (
            format!(
                "{minimal}  db: {{compose: {{file: a.yml, services: [db]}}}}\n  search: {{compose: {{file: b.yml, services: [search]}}}}\n"
            ),
            "only one service can use compose; list every Compose service in its services",
        ),
        (
            format!(
                "{minimal}  db: {{compose: {{file: compose.yml, http: {{web: {{port: 80, subdomain: mail, healthcheck: /}}}}}}}}\n"
            ),
            "two services are named web",
        ),
        (
            format!(
                "{minimal}  db: {{compose: {{file: compose.yml, http: {{mail: {{port: 80, healthcheck: /}}}}}}}}\n"
            ),
            "only one http service can leave out subdomain",
        ),
        (
            format!(
                "{minimal}  db: {{compose: {{file: compose.yml, services: [db]}}}}\nenv: {{COMPOSE_PROJECT_NAME: shop}}\n"
            ),
            "COMPOSE_PROJECT_NAME is set by Parcela for the compose service",
        ),
    ] {
        let message = error(&text);
        assert!(message.contains(expected), "{text}\n{message}");
    }
    assert!(error(&format!("{minimal}workers: {{}}\n")).contains("unknown field `workers`"));
    assert!(
        error(&minimal.replace("command: bin/web,", "cmd: bin/web,"))
            .contains("unknown field `cmd`")
    );
}

#[test]
fn a_compose_service_runs_checks_and_stops_its_containers() {
    let manifest = Manifest::parse(
        "version: 1\nproject: shop\nservices:\n  deps:\n    compose:\n      file: docker-compose.yml\n      services: [postgres, search]\n  web: {command: bin/web, http: {healthcheck: /}, depends_on: [deps]}\n",
    )
    .unwrap();
    let deps = &manifest.processes["deps"];
    assert_eq!(deps.command, "exec docker compose up postgres search");
    assert_eq!(deps.stop.as_deref(), Some("docker compose stop"));
    assert!(deps.healthcheck.as_deref().unwrap().ends_with("= 2"));
    assert_eq!(manifest.compose().unwrap().file, "docker-compose.yml");
}

#[test]
fn a_container_can_serve_the_environments_url() {
    let manifest = Manifest::parse(
        "version: 1\nproject: blog\nservices:\n  stack:\n    compose:\n      file: compose.yaml\n      services: [db]\n      http:\n        wordpress: {port: 80, healthcheck: /}\n        mail: {service: mailpit, port: 8025, subdomain: mail, healthcheck: /}\n",
    )
    .unwrap();
    assert_eq!(manifest.default_service.as_deref(), Some("wordpress"));
    assert_eq!(manifest.services["mail"].subdomain.as_deref(), Some("mail"));
    assert_eq!(
        manifest.compose().unwrap().services,
        ["db", "mailpit", "wordpress"]
    );
    let mail = &manifest.processes["mail"];
    assert!(
        mail.command.ends_with(" compose-forward mailpit 8025"),
        "{}",
        mail.command
    );
    assert_eq!(mail.depends_on, ["stack"]);
}

#[test]
fn named_ports_reject_invalid_names() {
    let head =
        "version: 1\nproject: app\nservices: {web: {command: bin/dev, http: {healthcheck: /}}}\n";
    assert_eq!(
        Manifest::parse(&format!("{head}ports: [db, vite-hmr]\n"))
            .unwrap()
            .ports,
        ["db", "vite-hmr"]
    );
    assert_eq!(
        parcela::config::port_variable("vite-hmr"),
        "PARCELA_PORT_VITE_HMR"
    );
    for (ports, message) in [
        ("[Db]", "DNS label"),
        ("[db_1]", "DNS label"),
        ("[db, db]", "duplicate port name: db"),
        ("db", "invalid type"),
    ] {
        let message_text = error(&format!("{head}ports: {ports}\n"));
        assert!(message_text.contains(message), "{ports}: {message_text}");
    }
}

fn run(path: &Path, program: &str, args: &[&str]) {
    let output = Command::new(program)
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn linked_checkouts_suggest_the_main_checkout_project_name() {
    let t = TempDir::new().unwrap();
    let main = t.path().join("Main-Repo");
    fs::create_dir(&main).unwrap();
    run(&main, "jj", &["git", "init", "--quiet"]);
    run(
        &main,
        "jj",
        &["workspace", "add", "--quiet", "../linked-jj"],
    );
    run(&main, "git", &["init", "--quiet", "../git-main"]);
    let git = t.path().join("git-main");
    run(
        &git,
        "git",
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    run(
        &git,
        "git",
        &["worktree", "add", "--quiet", "../linked-git"],
    );
    for (directory, expected) in [
        ("Main-Repo", "main-repo"),
        ("linked-jj", "main-repo"),
        ("git-main", "git-main"),
        ("linked-git", "git-main"),
    ] {
        let name = parcela::config::inferred_project_name(&t.path().join(directory)).unwrap();
        assert_eq!(name, expected, "{directory}");
    }
}

#[test]
fn sharing_publishers_join_the_graph_and_retry() {
    let manifest = Manifest::parse(SHOP).unwrap();
    let shared = BTreeMap::from([(
        "parcela-tailscale-web".to_owned(),
        "tailscale serve --https=443 http://127.0.0.1:21000".to_owned(),
    )]);
    let graph = plan(
        &manifest,
        &[],
        &BTreeMap::from([("PARCELA_ENVIRONMENT_NAME".to_owned(), "signup".to_owned())]),
        Path::new("/src/shop"),
        Path::new("/state/env"),
        false,
        &shared,
    )
    .unwrap();
    let publisher = graph
        .iter()
        .find(|node| node.name == "parcela-tailscale-web")
        .unwrap();
    assert_eq!(publisher.kind, Kind::Publisher);
    assert_eq!(
        publisher.program,
        Program::Shell("tailscale serve --https=443 http://127.0.0.1:21000".into())
    );
    assert_eq!(publisher.max_restarts, 10);
}

#[test]
fn long_inferred_labels_fit_every_service_host() {
    let manifest = Manifest::parse(
        "version: 1\nproject: shop\nservices:\n  web: {command: bin/web, http: {healthcheck: /}}\n  admin: {command: bin/admin, http: {healthcheck: /, subdomain: admin}}\n",
    )
    .unwrap();
    let branch = "add-mobile-checkout-flow-with-apple-pay-and-google-pay-v2";
    let label = parcela::config::fit_label(branch, manifest.label_budget());
    let other = parcela::config::fit_label(
        "add-mobile-checkout-flow-with-apple-pay-and-google-pay-v3",
        manifest.label_budget(),
    );
    assert_ne!(label, other);
    assert!(label.starts_with("add-mobile-checkout"));
    let hostname = manifest.hostname(false, &label).unwrap();
    let hosts = manifest.service_hosts(&hostname).unwrap();
    assert!(
        hosts
            .values()
            .all(|host| host.len() - ".localhost".len() <= 63)
    );
    assert_eq!(parcela::config::fit_label("signup", 40), "signup");
    let error = manifest.hostname(false, branch).unwrap_err().to_string();
    assert!(error.contains("choose a shorter --name"), "{error}");
}

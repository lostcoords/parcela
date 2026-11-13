//! A `compose` service runs containers from the project's own Docker Compose file. Each
//! environment gets its own Compose project, so its own containers, network and volumes, and an
//! override file in its runtime directory that publishes the containers' ports on leased loopback
//! ports. The project's Compose file stays as it is.
use crate::project::config::{Compose, ComposePort, ComposeTarget, Manifest};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    thread,
};

/// Set by Parcela for every service, hook and task while a service uses compose.
pub const VARIABLES: [&str; 2] = ["COMPOSE_PROJECT_NAME", "COMPOSE_FILE"];
const OVERRIDE: &str = "compose.yaml";

/// Compose's own rule for service names; these also reach the shell unquoted.
pub fn service_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && name.as_bytes()[0].is_ascii_alphanumeric()
}

/// The service's command, healthcheck and stop command. Compose reads its project and files from
/// [`VARIABLES`]. A service is ready once each Compose service runs and is healthy, or has no
/// healthcheck.
pub fn commands(services: &[String]) -> (String, String, String) {
    let services = services.join(" ");
    let count = services.split(' ').count();
    (
        format!("exec docker compose up {services}"),
        format!(
            "test \"$(docker compose ps -a --format '{{{{.State}}}}/{{{{.Health}}}}' {services} | grep -cxE 'running/(healthy)?')\" = {count}"
        ),
        "docker compose stop".into(),
    )
}

/// The command of an HTTP service that serves a container port.
pub fn forward_command(target: &ComposeTarget) -> Result<String> {
    let exe = std::env::current_exe().context("find the parcela executable")?;
    let exe = exe.to_str().context("non-UTF-8 parcela path")?;
    Ok(format!(
        "exec '{}' compose-forward {} {}",
        exe.replace('\'', "'\\''"),
        target.service,
        target.port
    ))
}

/// `parcela compose-forward SERVICE PORT` serves the container port on the HTTP service's own
/// `$PORT`. Docker, not the environment, holds a published port, and Parcela routes only
/// listeners its own processes hold.
pub fn forward(service: &str, port: u16) -> Result<()> {
    let listen: u16 = std::env::var("PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .context("compose-forward needs the service's PORT")?;
    let output = Command::new("docker")
        .args(["compose", "port", service, &port.to_string()])
        .output()
        .context("run `docker compose port`")?;
    ensure!(
        output.status.success(),
        "docker compose port {service} {port} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let target: SocketAddr = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .with_context(|| format!("{service} does not publish port {port}"))?;
    ensure!(
        target.ip().is_loopback(),
        "{service} publishes port {port} on {target}, not on loopback"
    );
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, listen))
        .with_context(|| format!("listen on 127.0.0.1:{listen}"))?;
    for client in listener.incoming().flatten() {
        thread::spawn(move || {
            if let Ok(server) = TcpStream::connect(target) {
                relay(client, server);
            }
        });
    }
    Ok(())
}

fn relay(client: TcpStream, server: TcpStream) {
    let (Ok(mut from_client), Ok(mut to_server)) = (client.try_clone(), server.try_clone()) else {
        return;
    };
    let upstream = thread::spawn(move || {
        let _ = io::copy(&mut from_client, &mut to_server);
        let _ = to_server.shutdown(Shutdown::Write);
    });
    let (mut from_server, mut to_client) = (server, client);
    let _ = io::copy(&mut from_server, &mut to_client);
    let _ = to_client.shutdown(Shutdown::Write);
    let _ = upstream.join();
}

/// The Compose file and, as plain `docker compose` would load it next to a default-named file,
/// its override file.
fn files(source: &Path, compose: &Compose) -> Vec<PathBuf> {
    let file = source.join(&compose.file);
    let stem = file.file_stem().and_then(|stem| stem.to_str());
    let overrides = match stem {
        Some(stem @ ("compose" | "docker-compose")) => ["yaml", "yml"]
            .iter()
            .map(|extension| file.with_file_name(format!("{stem}.override.{extension}")))
            .find(|path| path.is_file()),
        _ => None,
    };
    std::iter::once(file).chain(overrides).collect()
}

/// Fills in what the compose service starts and the ports it publishes, from the Compose file as
/// `docker compose config` reads it now, and leases each port with the manifest's named ports.
pub fn resolve(manifest: &mut Manifest, source: &Path) -> Result<()> {
    let Some((compose_env, compose)) = manifest
        .processes
        .values_mut()
        .find_map(|process| Some((process.env.clone(), process.compose.as_mut()?)))
    else {
        return Ok(());
    };
    let files = files(source, compose);
    ensure!(
        files[0].is_file(),
        "compose.file {} does not exist",
        files[0].display()
    );
    supported(&compose_version()?)?;
    let mut command = Command::new("docker");
    command.current_dir(source).arg("compose");
    for file in &files {
        command.arg("-f").arg(file);
    }
    command.args(["config", "--format", "json"]);
    for key in VARIABLES {
        command.env_remove(key);
    }
    command.envs(known_environment(
        &manifest.environment,
        &compose_env,
        source,
    )?);
    let output = command
        .output()
        .context("run `docker compose config`; is Docker installed?")?;
    ensure!(
        output.status.success(),
        "docker compose config failed for {}: {}",
        files[0].display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let config: Value =
        serde_json::from_slice(&output.stdout).context("read `docker compose config`")?;
    (compose.started, compose.ports) =
        published(&config, &compose.services, compose.routes.values())?;
    compose.volumes = own(&config["volumes"]);
    compose.networks = own(&config["networks"]);
    for port in &compose.ports {
        ensure!(
            !manifest.ports.contains(&port.name),
            "port {} is published by the Compose service {}; rename it in ports",
            port.name,
            port.service
        );
        manifest.ports.push(port.name.clone());
    }
    Ok(())
}

/// The oldest Compose that applies the override's `!override` tag. Older releases ignore it
/// without an error and keep the project's own published ports, so two environments would publish
/// the same host port.
const MINIMUM: semver::Version = semver::Version::new(2, 24, 4);

fn compose_version() -> Result<String> {
    let output = Command::new("docker")
        .args(["compose", "version", "--short"])
        .output()
        .context("run `docker compose version`; is Docker installed?")?;
    ensure!(
        output.status.success(),
        "docker compose version failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Refuses a Compose older than `MINIMUM`. Accepts `v2.24.4`, `2.29.1-desktop.1` and the like.
fn supported(version: &str) -> Result<()> {
    let core = version
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or_default();
    let parsed = semver::Version::parse(core)
        .with_context(|| format!("cannot read the Docker Compose version {version:?}"))?;
    ensure!(
        parsed >= MINIMUM,
        "Docker Compose {version} ignores the port overrides Parcela writes; update to {MINIMUM} or newer"
    );
    Ok(())
}

/// The `env` values Compose can read before the environment exists: those that need no port or
/// environment name, like `COMPOSE_ENV_FILES: ${PARCELA_PRIMARY_DIR}/.env`.
fn known_environment(
    environment: &BTreeMap<String, String>,
    service: &BTreeMap<String, String>,
    source: &Path,
) -> Result<BTreeMap<String, String>> {
    let primary =
        crate::project::config::main_checkout(source).unwrap_or_else(|| source.to_owned());
    let mut known = BTreeMap::from([
        (
            "PARCELA_PRIMARY_DIR".to_owned(),
            primary
                .to_str()
                .context("non-UTF-8 checkout path")?
                .to_owned(),
        ),
        ("HOST".to_owned(), "127.0.0.1".to_owned()),
    ]);
    for (key, value) in environment.iter().chain(service) {
        if let Ok(value) = crate::environment::variables::resolve(key, value, &known) {
            known.insert(key.clone(), value);
        }
    }
    Ok(known)
}

/// The volumes or networks the Compose file defines itself, rather than as `external`.
fn own(definitions: &Value) -> Vec<String> {
    definitions
        .as_object()
        .map_or_else(Vec::new, |definitions| {
            definitions
                .iter()
                .filter(|(_, definition)| definition["external"].as_bool() != Some(true))
                .map(|(name, _)| name.clone())
                .collect()
        })
}

/// The services `docker compose up services` starts, and the ports they publish, with each route's
/// port added. A service's first port is named after it, and every other one adds its container
/// port.
fn published<'a>(
    config: &Value,
    services: &[String],
    routes: impl Iterator<Item = &'a ComposeTarget> + Clone,
) -> Result<(Vec<String>, Vec<ComposePort>)> {
    let definitions = config["services"]
        .as_object()
        .context("docker compose config has no services")?;
    let mut started = BTreeSet::new();
    let mut pending: Vec<&str> = services.iter().map(String::as_str).collect();
    while let Some(name) = pending.pop() {
        let Some(definition) = definitions.get(name) else {
            bail!("the Compose file has no service {name}");
        };
        if started.insert(name.to_owned())
            && let Some(dependencies) = definition["depends_on"].as_object()
        {
            pending.extend(dependencies.keys().map(String::as_str));
        }
    }
    let mut ports = Vec::new();
    for service in &started {
        let label = crate::project::config::sanitize_label(service)
            .with_context(|| format!("cannot name a port after the Compose service {service}"))?;
        let mut targets = Vec::new();
        for port in definitions[service]["ports"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
        {
            let target = port["target"]
                .as_u64()
                .and_then(|target| u16::try_from(target).ok())
                .with_context(|| format!("Compose service {service}: unreadable port {port}"))?;
            targets.push((
                target,
                port["protocol"].as_str().unwrap_or("tcp").to_owned(),
            ));
        }
        for route in routes.clone().filter(|route| &route.service == service) {
            if !targets.iter().any(|(target, _)| *target == route.port) {
                targets.push((route.port, "tcp".into()));
            }
        }
        for (index, (target, protocol)) in targets.into_iter().enumerate() {
            ports.push(ComposePort {
                name: if index == 0 {
                    label.clone()
                } else {
                    format!("{label}-{target}")
                },
                service: service.clone(),
                target,
                protocol,
            });
        }
    }
    let mut names = BTreeSet::new();
    for port in &ports {
        crate::project::config::port_name(&port.name)?;
        ensure!(
            names.insert(&port.name),
            "two Compose ports are both named {}",
            port.name
        );
    }
    Ok((started.into_iter().collect(), ports))
}

/// [`VARIABLES`] for an environment, so every `docker compose` it runs finds its own containers.
pub fn environment(
    manifest: &Manifest,
    source: &Path,
    runtime: &Path,
    label: &str,
) -> Result<Vec<(String, String)>> {
    let Some(compose) = manifest.compose() else {
        return Ok(Vec::new());
    };
    let files = files(source, compose)
        .into_iter()
        .chain([runtime.join(OVERRIDE)])
        .map(|path| {
            let path = path.to_str().context("non-UTF-8 Compose path")?;
            ensure!(!path.contains(':'), "Compose path {path} contains `:`");
            Ok(path.to_owned())
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(vec![
        (VARIABLES[0].into(), project(manifest, label)),
        (VARIABLES[1].into(), files.join(":")),
    ])
}

fn project(manifest: &Manifest, label: &str) -> String {
    format!("{}-{label}", manifest.project.name)
}

/// Removes an environment's containers and network, which need no Compose file, and returns the
/// volumes it keeps.
pub fn remove(manifest: &Manifest, label: &str) -> Result<Vec<String>> {
    if manifest.compose().is_none() {
        return Ok(Vec::new());
    }
    let project = project(manifest, label);
    let docker = |args: &[&str]| -> Result<String> {
        let mut command = Command::new("docker");
        command.args(args).current_dir(std::env::temp_dir());
        for key in VARIABLES {
            command.env_remove(key);
        }
        let output = command
            .output()
            .context("run docker; is Docker installed?")?;
        ensure!(
            output.status.success(),
            "docker {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    docker(&["compose", "-p", &project, "down"])?;
    let label = format!("label=com.docker.compose.project={project}");
    Ok(docker(&["volume", "ls", "-q", "--filter", &label])?
        .lines()
        .map(str::to_owned)
        .collect())
}

/// Writes the override file the environment's [`VARIABLES`] name.
pub fn write_override(
    runtime: &Path,
    compose: &Compose,
    ports: &BTreeMap<String, u16>,
) -> Result<()> {
    crate::state::private::write_bytes(
        &runtime.join(OVERRIDE),
        override_text(compose, ports)?.as_bytes(),
    )
}

/// Each started service loses any `container_name`, and each volume and network any fixed `name`,
/// which two environments cannot share. Each service publishes its ports on leased loopback ports
/// instead of the file's own.
fn override_text(compose: &Compose, ports: &BTreeMap<String, u16>) -> Result<String> {
    let mut text = String::from("# Written by Parcela for this environment.\nservices:\n");
    for service in &compose.started {
        text.push_str(&format!("  {service}:\n    container_name: !reset null\n"));
        let published = compose
            .ports
            .iter()
            .filter(|port| &port.service == service)
            .map(|port| {
                let host = ports
                    .get(&port.name)
                    .with_context(|| format!("port {} was not leased", port.name))?;
                Ok(format!(
                    "\"127.0.0.1:{host}:{}/{}\"",
                    port.target, port.protocol
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        if !published.is_empty() {
            text.push_str(&format!(
                "    ports: !override [{}]\n",
                published.join(", ")
            ));
        }
    }
    for (section, names) in [
        ("volumes", &compose.volumes),
        ("networks", &compose.networks),
    ] {
        if !names.is_empty() {
            text.push_str(&format!("{section}:\n"));
            for name in names {
                text.push_str(&format!("  {name}:\n    name: !reset null\n"));
            }
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    #[test]
    fn compose_before_2_24_4_is_refused() {
        for old in ["2.24.3", "v2.20.0", "1.29.2"] {
            assert!(super::supported(old).is_err(), "{old}");
        }
        for new in ["2.24.4", "v2.24.5", "2.29.1-desktop.1", "5.5.0"] {
            super::supported(new).unwrap();
        }
        assert!(super::supported("unknown").is_err());
    }

    use super::*;
    use serde_json::json;

    fn config() -> Value {
        json!({"services": {
            "postgres": {"ports": [{"target": 5432, "published": "5432", "protocol": "tcp"}]},
            "search": {
                "depends_on": {"postgres": {"condition": "service_healthy"}},
                "ports": [
                    {"target": 9200, "published": "9200", "protocol": "tcp"},
                    {"target": 9300, "published": "9300", "protocol": "tcp"}
                ]
            },
            "web": {"ports": [{"target": 3000, "published": "3000", "protocol": "tcp"}]}
        }})
    }

    #[test]
    fn published_follows_dependencies_and_names_ports() {
        let (started, ports) = published(&config(), &["search".into()], [].iter()).unwrap();
        assert_eq!(started, ["postgres", "search"]);
        let names: Vec<_> = ports.iter().map(|port| port.name.as_str()).collect();
        assert_eq!(names, ["postgres", "search", "search-9300"]);
    }

    #[test]
    fn a_route_publishes_its_port_after_the_file_ports() {
        let routes = [
            ComposeTarget {
                service: "web".into(),
                port: 3000,
            },
            ComposeTarget {
                service: "web".into(),
                port: 9229,
            },
        ];
        let (_, ports) = published(&config(), &["web".into()], routes.iter()).unwrap();
        let names: Vec<_> = ports
            .iter()
            .map(|port| (port.name.as_str(), port.target))
            .collect();
        assert_eq!(names, [("web", 3000), ("web-9229", 9229)]);
    }

    #[test]
    fn published_rejects_an_unknown_service() {
        let error = published(&config(), &["redis".into()], [].iter()).unwrap_err();
        assert_eq!(error.to_string(), "the Compose file has no service redis");
    }

    #[test]
    fn override_rebinds_ports_to_loopback() {
        let (started, ports) = published(&config(), &["postgres".into()], [].iter()).unwrap();
        let compose = Compose {
            file: "docker-compose.yml".into(),
            services: vec!["postgres".into()],
            routes: BTreeMap::new(),
            started,
            ports,
            volumes: vec!["data".into()],
            networks: vec!["default".into()],
        };
        let text = override_text(&compose, &BTreeMap::from([("postgres".into(), 24001)])).unwrap();
        assert_eq!(
            text,
            "# Written by Parcela for this environment.\nservices:\n  postgres:\n    container_name: !reset null\n    ports: !override [\"127.0.0.1:24001:5432/tcp\"]\nvolumes:\n  data:\n    name: !reset null\nnetworks:\n  default:\n    name: !reset null\n"
        );
    }

    #[test]
    fn external_volumes_and_networks_stay_shared() {
        let definitions =
            json!({"data": {"name": "shop_data"}, "cache": {"name": "cache", "external": true}});
        assert_eq!(own(&definitions), ["data"]);
        assert!(own(&Value::Null).is_empty());
    }

    #[test]
    fn compose_reads_the_env_that_needs_no_environment() {
        let environment = BTreeMap::from([
            (
                "COMPOSE_ENV_FILES".into(),
                "${PARCELA_PRIMARY_DIR}/.env".into(),
            ),
            ("DATABASE_PORT".into(), "${PARCELA_PORT_POSTGRES}".into()),
        ]);
        let service = BTreeMap::from([("IMAGE_TAG".into(), "17".into())]);
        let known = known_environment(&environment, &service, Path::new("/nowhere")).unwrap();
        assert_eq!(known["COMPOSE_ENV_FILES"], "/nowhere/.env");
        assert_eq!(known["IMAGE_TAG"], "17");
        assert!(!known.contains_key("DATABASE_PORT"));
    }

    #[test]
    fn healthcheck_counts_every_service() {
        let (command, healthcheck, _) = commands(&["postgres".into(), "search".into()]);
        assert_eq!(command, "exec docker compose up postgres search");
        assert!(healthcheck.ends_with("grep -cxE 'running/(healthy)?')\" = 2"));
        assert!(healthcheck.contains("--format '{{.State}}/{{.Health}}' postgres search"));
    }
}

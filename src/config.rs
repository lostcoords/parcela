use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// The validated project configuration, as the runtime uses it. `parcela.yaml` is parsed from
/// [`File`] into this shape, which is also saved with each run.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub project: Project,
    /// The HTTP service served at the environment's own URL.
    pub default_service: Option<String>,
    /// Routed HTTP services, keyed like `processes`.
    pub services: BTreeMap<String, Service>,
    /// Every long-running process, HTTP or not.
    pub processes: BTreeMap<String, Process>,
    pub hooks: Hooks,
    pub tasks: BTreeMap<String, Task>,
    /// Non-HTTP ports leased per run and exported as `PARCELA_PORT_<NAME>`; never routed or probed.
    pub ports: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub shutdown_timeout_seconds: u64,
    /// How long `up` waits for the services to be ready, unless `--readiness-timeout` says.
    #[serde(default = "default_readiness_timeout")]
    pub readiness_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub id: String,
    /// DNS-safe project name, distinct from the stable ID.
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    /// Hostname prefix; `None` only for the default service.
    pub subdomain: Option<String>,
    pub readiness_path: String,
    /// Also routes every subdomain of the service's host, for subdomain-per-tenant apps.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wildcard: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Process {
    pub command: String,
    pub working_dir: Option<String>,
    pub env: BTreeMap<String, String>,
    pub depends_on: Vec<String>,
    /// A shell command that exits 0 once a non-HTTP service is ready.
    pub healthcheck: Option<String>,
    /// A shell command that stops what `command` started outside its own processes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    /// The Compose services `command` starts, for a `compose` service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose: Option<Compose>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Compose {
    /// Relative to the checkout root.
    pub file: String,
    pub services: Vec<String>,
    /// HTTP services that serve a container port, by service name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub routes: BTreeMap<String, ComposeTarget>,
    /// `services` and everything they depend on, from `docker compose config` at `up`.
    #[serde(default)]
    pub started: Vec<String>,
    /// The ports `started` publishes, each leased like a named port.
    #[serde(default)]
    pub ports: Vec<ComposePort>,
    /// The Compose file's own volumes and networks, which each environment gets a copy of.
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub networks: Vec<String>,
}

/// A container port of a Compose service.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComposeTarget {
    pub service: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComposePort {
    /// The named port, exported as `PARCELA_PORT_<NAME>`.
    pub name: String,
    pub service: String,
    /// The container port.
    pub target: u16,
    pub protocol: String,
}

/// Commands that prepare an environment. Each runs after the services in its `depends_on` and before every other service.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    /// Runs on the first `up` of an environment, and again with `up --setup`. Install packages, create and seed the database, copy secrets.
    pub setup: Option<Hook>,
    /// Runs on every `up` and `restart`, after `setup`. Run migrations.
    pub prestart: Option<Hook>,
    /// Runs on `parcela rm`, after the environment stopped. Drop its database or delete outside data. It cannot use `depends_on`.
    pub teardown: Option<Hook>,
}

/// The hooks `up` runs, in order. They also name the hook's process, so no service may use them.
pub const HOOKS: [&str; 2] = ["setup", "prestart"];

impl Hooks {
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &Hook)> {
        HOOKS
            .into_iter()
            .zip([&self.setup, &self.prestart])
            .filter_map(|(name, hook)| hook.as_ref().map(|hook| (name, hook)))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Hook {
    /// A shell command, run from the checkout root. A nonzero exit stops `up`.
    pub command: String,
    /// Services to start, and wait for, before the hook runs.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// A shell command, run from the checkout root. Arguments after `--` are appended.
    pub command: String,
    /// Stop the task after this many seconds and exit 124.
    #[cfg_attr(test, schemars(range(min = 1)))]
    pub timeout_seconds: Option<u64>,
}

/// The only `parcela.yaml` format version this build reads.
pub const VERSION: u32 = 1;
const SHUTDOWN_TIMEOUT_SECONDS: u64 = 300;
const READINESS_TIMEOUT_SECONDS: u64 = 60;

fn default_readiness_timeout() -> u64 {
    READINESS_TIMEOUT_SECONDS
}

/// `parcela.yaml` as written. Every field has one spelling and one shape.
/// Reference: https://parcela.run/docs/configuration/
#[derive(Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[cfg_attr(test, schemars(title = "parcela.yaml", extend("patternProperties" = {"^x-": {}})))]
#[serde(deny_unknown_fields)]
struct File {
    /// The file format version.
    #[serde(rename = "version")]
    #[cfg_attr(test, schemars(extend("const" = 1)))]
    _version: u32,
    /// The project name, used in every URL: `https://<project>.localhost:1443`.
    #[cfg_attr(
        test,
        schemars(regex(pattern = r"^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$"))
    )]
    project: String,
    /// Variables for every service, hook and task. Values can use `${NAME}` for any variable Parcela sets.
    #[serde(default)]
    env: BTreeMap<String, String>,
    /// Extra ports to lease per environment, exported as `PARCELA_PORT_<NAME>`.
    #[serde(default)]
    ports: Vec<String>,
    /// Every long-running process, keyed by name.
    services: BTreeMap<String, FileService>,
    #[serde(default)]
    hooks: Hooks,
    /// Commands to run with `parcela task NAME`.
    #[serde(default)]
    tasks: BTreeMap<String, Task>,
    /// How long `up` waits for every service to be ready, in seconds. Time spent in hooks does not count. Default 60.
    #[cfg_attr(test, schemars(range(min = 1, max = 86400)))]
    readiness_timeout_seconds: Option<u64>,
}

/// A long-running process.
#[derive(Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct FileService {
    /// A shell command, run from the checkout root. HTTP services listen on `127.0.0.1:$PORT`. Required unless the service has `compose`.
    command: Option<String>,
    /// Runs containers from a Docker Compose file instead of `command`, with their own names, ports and volumes in every environment.
    compose: Option<FileCompose>,
    /// Gives the service a port in `$PORT` and an HTTPS URL.
    http: Option<Http>,
    /// For services without `http`: a shell command that exits 0 once the service is ready.
    healthcheck: Option<String>,
    /// A shell command that stops what `command` started outside its own processes, like `docker compose stop`. It runs when the environment stops, before the service is signaled, even if `command` already exited.
    stop: Option<String>,
    /// Services to start, and wait for, before this one.
    #[serde(default)]
    depends_on: Vec<String>,
    /// A directory relative to the checkout root to run `command` in.
    working_dir: Option<String>,
    /// Variables for this service only.
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct FileCompose {
    /// The Compose file, relative to the checkout root, like `docker-compose.yml`.
    file: String,
    /// The Compose services to start. The service is ready once each one runs and, if it has a healthcheck, is healthy.
    #[serde(default)]
    services: Vec<String>,
    /// HTTPS URLs for containers that serve HTTP, keyed by service name like services with `http`. Their Compose services start too.
    #[serde(default)]
    http: BTreeMap<String, ComposeHttp>,
}

/// A container port served at its own URL, like a service with `http`.
#[derive(Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct ComposeHttp {
    /// The Compose service. Default: the key.
    service: Option<String>,
    /// The container port that serves HTTP, like `8025`. It does not need to be published.
    port: u16,
    /// The URL prefix, as in `http`. Leave it out on the one service that gets the environment's own URL.
    subdomain: Option<String>,
    /// A path that answers 2xx once the container serves, like `/`.
    #[cfg_attr(test, schemars(regex(pattern = "^/")))]
    healthcheck: String,
    /// Also send every subdomain of this service's host to it.
    #[serde(default)]
    wildcard: bool,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct Http {
    /// The URL prefix: `https://<subdomain>-<environment>.localhost`. Leave it out on the one service that gets the environment's own URL.
    subdomain: Option<String>,
    /// A path that answers 2xx once the service is ready, like `/up`.
    #[cfg_attr(test, schemars(regex(pattern = "^/")))]
    healthcheck: String,
    /// Also send every subdomain of this service's host to it, like `https://kobe.shop.localhost`, for subdomain-per-tenant apps.
    #[serde(default)]
    wildcard: bool,
}

pub fn dns_label(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 63,
        "DNS label must contain 1–63 ASCII characters: {value:?}"
    );
    ensure!(
        value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !value.starts_with('-')
            && !value.ends_with('-'),
        "DNS label must use lowercase ASCII letters, digits and internal hyphens: {value:?}"
    );
    Ok(())
}

/// A named port is a DNS label, so its `PARCELA_PORT_<NAME>` variable is unique.
pub fn port_name(name: &str) -> Result<()> {
    dns_label(name).with_context(|| format!("invalid port name {name:?}"))
}

pub fn port_variable(name: &str) -> String {
    format!(
        "PARCELA_PORT_{}",
        name.to_ascii_uppercase().replace('-', "_")
    )
}

/// Lowercase ASCII words joined by single hyphens, cut to one DNS label.
pub fn sanitize_label(name: &str) -> Option<String> {
    let label = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let label = label[..label.len().min(63)]
        .trim_end_matches('-')
        .to_owned();
    (!label.is_empty()).then_some(label)
}

/// Cuts an inferred checkout label to `budget` bytes, ending in a hash of the full label so
/// long names that share a prefix stay distinct.
pub fn fit_label(label: &str, budget: usize) -> String {
    if label.len() <= budget {
        return label.to_owned();
    }
    let hash = label.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    let hash = format!("{:04x}", hash & 0xffff);
    let head = label[..budget.saturating_sub(hash.len() + 1)].trim_end_matches('-');
    if head.is_empty() {
        hash
    } else {
        format!("{head}-{hash}")
    }
}

/// The project is named after the main checkout, so every jj workspace and Git worktree agrees.
pub fn inferred_project_name(source: &Path) -> Result<String> {
    let source = source.canonicalize()?;
    let root = main_checkout(&source).unwrap_or(source);
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    sanitize_label(name).with_context(|| {
        format!("cannot name the project after {name:?}; add `project: my-app` to parcela.yaml")
    })
}

pub fn main_checkout(source: &Path) -> Option<PathBuf> {
    let root = source
        .ancestors()
        .find(|path| path.join(".jj").is_dir() || path.join(".git").exists())?;
    let jj = root.join(".jj");
    if jj.is_dir() {
        let pointer = jj.join("repo");
        if pointer.is_dir() {
            return Some(root.to_path_buf());
        }
        let repo = jj
            .join(fs::read_to_string(pointer).ok()?.trim())
            .canonicalize()
            .ok()?;
        return repo.parent()?.parent().map(Path::to_path_buf);
    }
    let git = root.join(".git");
    if git.is_dir() {
        return Some(root.to_path_buf());
    }
    let text = fs::read_to_string(git).ok()?;
    let gitdir = root.join(text.strip_prefix("gitdir:")?.trim());
    let common = gitdir.join(fs::read_to_string(gitdir.join("commondir")).ok()?.trim());
    let common = common.canonicalize().ok()?;
    if common.file_name()? == ".git" {
        common.parent().map(Path::to_path_buf)
    } else {
        Some(common)
    }
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid {}", path.display()))
    }

    /// Parse and validate `parcela.yaml` text.
    pub fn parse(text: &str) -> Result<Self> {
        let document: Value = serde_saphyr::from_str(text)?;
        let Value::Object(mut fields) = document else {
            bail!("parcela.yaml must be a mapping that starts with `version: {VERSION}`");
        };
        match fields.get("version") {
            Some(version) if version.as_u64() == Some(VERSION.into()) => {}
            Some(version) => {
                bail!("unsupported version {version}; this Parcela reads version {VERSION}")
            }
            None => bail!(
                "add `version: {VERSION}` at the top of parcela.yaml; files from Parcela 0.2 and earlier also need the format at https://parcela.run/docs/configuration/"
            ),
        }
        // `x-` keys are free for YAML anchors and other tools, as in Docker Compose.
        fields.retain(|key, _| !key.starts_with("x-"));
        let file: File = serde_json::from_value(Value::Object(fields))?;
        let manifest = Self::from_file(file)?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn from_file(file: File) -> Result<Self> {
        dns_label(&file.project).context("project must be a DNS label, like `shop`")?;
        let mut services = BTreeMap::new();
        let mut processes = BTreeMap::new();
        let mut default_service = None;
        let mut route = |name: &str, http: Http| -> Result<()> {
            if http.subdomain.is_none() {
                ensure!(
                    default_service.replace(name.to_owned()).is_none(),
                    "only one http service can leave out subdomain; it gets the environment's own URL"
                );
            }
            services.insert(
                name.to_owned(),
                Service {
                    subdomain: http.subdomain,
                    readiness_path: http.healthcheck,
                    wildcard: http.wildcard,
                },
            );
            Ok(())
        };
        let mut forwarders = Vec::new();
        for (name, service) in file.services {
            let routed = service.http.is_some();
            if let Some(http) = service.http {
                ensure!(
                    service.healthcheck.is_none(),
                    "service {name} has http, so its readiness is http.healthcheck; remove healthcheck"
                );
                route(&name, http)?;
            }
            let process = match (service.command, service.compose) {
                (Some(command), None) => Process {
                    command,
                    working_dir: service.working_dir,
                    env: service.env,
                    depends_on: service.depends_on,
                    healthcheck: service.healthcheck,
                    stop: service.stop,
                    compose: None,
                },
                (None, Some(compose)) => {
                    ensure!(
                        !routed
                            && service.healthcheck.is_none()
                            && service.stop.is_none()
                            && service.working_dir.is_none(),
                        "service {name} has compose, so Parcela runs, checks and stops it; remove http, healthcheck, stop and working_dir"
                    );
                    let mut started = compose.services;
                    let mut routes = BTreeMap::new();
                    for (route_name, http) in compose.http {
                        let target = ComposeTarget {
                            service: http.service.unwrap_or_else(|| route_name.clone()),
                            port: http.port,
                        };
                        if !started.contains(&target.service) {
                            started.push(target.service.clone());
                        }
                        route(
                            &route_name,
                            Http {
                                subdomain: http.subdomain,
                                healthcheck: http.healthcheck,
                                wildcard: http.wildcard,
                            },
                        )?;
                        forwarders.push((route_name.clone(), name.clone(), target.clone()));
                        routes.insert(route_name, target);
                    }
                    let (command, healthcheck, stop) = crate::compose::commands(&started);
                    Process {
                        command,
                        working_dir: None,
                        env: service.env,
                        depends_on: service.depends_on,
                        healthcheck: Some(healthcheck),
                        stop: Some(stop),
                        compose: Some(Compose {
                            file: compose.file,
                            services: started,
                            routes,
                            started: Vec::new(),
                            ports: Vec::new(),
                            volumes: Vec::new(),
                            networks: Vec::new(),
                        }),
                    }
                }
                (Some(_), Some(_)) => {
                    bail!("service {name} has both command and compose; keep one")
                }
                (None, None) => bail!("service {name} needs a command"),
            };
            ensure!(
                processes.insert(name.clone(), process).is_none(),
                "two services are named {name}"
            );
        }
        for (name, compose, target) in forwarders {
            let process = Process {
                command: crate::compose::forward_command(&target)?,
                working_dir: None,
                env: BTreeMap::new(),
                depends_on: vec![compose],
                healthcheck: None,
                stop: None,
                compose: None,
            };
            ensure!(
                processes.insert(name.clone(), process).is_none(),
                "two services are named {name}"
            );
        }
        Ok(Self {
            project: Project {
                id: file.project.clone(),
                name: file.project,
            },
            default_service,
            services,
            processes,
            hooks: file.hooks,
            tasks: file.tasks,
            ports: file.ports,
            environment: file.env,
            shutdown_timeout_seconds: SHUTDOWN_TIMEOUT_SECONDS,
            readiness_timeout_seconds: file
                .readiness_timeout_seconds
                .unwrap_or(READINESS_TIMEOUT_SECONDS),
        })
    }

    pub fn validate(&self) -> Result<()> {
        dns_label(&self.project.name)?;
        ensure!(
            !self.processes.is_empty(),
            "at least one service is required"
        );
        ensure!(
            self.default_service
                .as_ref()
                .map_or(self.services.is_empty(), |name| self
                    .services
                    .contains_key(name)),
            "one service needs http without a subdomain, like `http: {{healthcheck: /}}`; it gets the environment's URL"
        );
        let mut names = std::collections::BTreeSet::new();
        for (name, service) in &self.services {
            ensure!(
                service.readiness_path.starts_with('/')
                    && service.readiness_path.is_ascii()
                    && !service
                        .readiness_path
                        .bytes()
                        .any(|b| b <= b' ' || b == 127),
                "service {name}: http.healthcheck must be a path like /up"
            );
            if let Some(subdomain) = &service.subdomain {
                dns_label(subdomain)
                    .with_context(|| format!("service {name}: invalid http.subdomain"))?;
                ensure!(
                    names.insert(subdomain),
                    "two services use the subdomain {subdomain}"
                );
            }
        }
        for (name, process) in &self.processes {
            dns_label(name).with_context(|| format!("invalid service name {name:?}"))?;
            ensure!(
                !HOOKS.contains(&name.as_str()),
                "{name} is reserved for hooks.{name}; rename the service"
            );
            ensure!(
                !process.command.trim().is_empty() && !process.command.contains('\0'),
                "service {name} needs a command"
            );
            if let Some(stop) = &process.stop {
                ensure!(
                    !stop.trim().is_empty() && !stop.contains('\0'),
                    "service {name}: stop needs a command"
                );
            }
            if let Some(dir) = &process.working_dir {
                ensure!(
                    Path::new(dir).is_relative(),
                    "service {name}: working_dir must be relative to parcela.yaml"
                );
            }
            check_environment(&process.env)
                .with_context(|| format!("service {name}: invalid env"))?;
            self.check_dependencies(&format!("service {name}"), &process.depends_on)?;
            ensure!(
                !process.depends_on.contains(name),
                "service {name} depends on itself"
            );
        }
        self.check_cycles()?;
        if let Some(teardown) = &self.hooks.teardown {
            ensure!(
                !teardown.command.trim().is_empty() && !teardown.command.contains('\0'),
                "hooks.teardown needs a command"
            );
            ensure!(
                teardown.depends_on.is_empty(),
                "hooks.teardown runs after every service stopped, so it cannot use depends_on"
            );
        }
        for (name, hook) in self.hooks.iter() {
            ensure!(
                !hook.command.trim().is_empty() && !hook.command.contains('\0'),
                "hooks.{name} needs a command"
            );
            self.check_dependencies(&format!("hooks.{name}"), &hook.depends_on)?;
        }
        ensure!(
            (1..=86400).contains(&self.readiness_timeout_seconds),
            "readiness_timeout_seconds must be between 1 and 86400"
        );
        let mut ports = std::collections::BTreeSet::new();
        for name in &self.ports {
            port_name(name)?;
            ensure!(ports.insert(name), "duplicate port name: {name}");
        }
        for (name, task) in &self.tasks {
            dns_label(name).with_context(|| format!("invalid task name {name:?}"))?;
            ensure!(
                !task.command.trim().is_empty() && !task.command.contains('\0'),
                "task {name} needs a command"
            );
            ensure!(
                task.timeout_seconds != Some(0),
                "task {name} timeout must be positive"
            );
        }
        check_environment(&self.environment).context("invalid env")?;
        self.check_compose()
    }

    /// The one service that runs a Compose file, if any.
    pub fn compose(&self) -> Option<&Compose> {
        self.processes
            .values()
            .find_map(|process| process.compose.as_ref())
    }

    fn check_compose(&self) -> Result<()> {
        let mut composes = self
            .processes
            .iter()
            .filter_map(|(name, process)| Some((name, process.compose.as_ref()?)));
        let Some((name, compose)) = composes.next() else {
            return Ok(());
        };
        ensure!(
            composes.next().is_none(),
            "only one service can use compose; list every Compose service in its services"
        );
        ensure!(
            Path::new(&compose.file).is_relative()
                && !compose.file.is_empty()
                && !compose.file.contains([':', '\0']),
            "service {name}: compose.file must be a path relative to parcela.yaml"
        );
        ensure!(
            !compose.services.is_empty(),
            "service {name}: compose needs at least one Compose service in services or http"
        );
        let targets = compose.routes.values().map(|target| &target.service);
        for service in compose.services.iter().chain(targets) {
            ensure!(
                crate::compose::service_name(service),
                "service {name}: invalid Compose service name {service:?}"
            );
        }
        for key in crate::compose::VARIABLES {
            ensure!(
                !self.environment.contains_key(key)
                    && self
                        .processes
                        .values()
                        .all(|process| !process.env.contains_key(key)),
                "{key} is set by Parcela for the compose service"
            );
        }
        Ok(())
    }

    fn check_dependencies(&self, owner: &str, depends_on: &[String]) -> Result<()> {
        for dependency in depends_on {
            ensure!(
                self.processes.contains_key(dependency),
                "{owner} depends on {dependency}, which is not a service"
            );
        }
        Ok(())
    }

    fn check_cycles(&self) -> Result<()> {
        fn visit<'a>(
            manifest: &'a Manifest,
            name: &'a str,
            path: &mut Vec<&'a str>,
            done: &mut std::collections::BTreeSet<&'a str>,
        ) -> Result<()> {
            if done.contains(name) {
                return Ok(());
            }
            ensure!(
                !path.contains(&name),
                "depends_on forms a cycle: {} -> {name}",
                path.join(" -> ")
            );
            path.push(name);
            for dependency in &manifest.processes[name].depends_on {
                visit(manifest, dependency, path, done)?;
            }
            path.pop();
            done.insert(name);
            Ok(())
        }
        let mut done = std::collections::BTreeSet::new();
        for name in self.processes.keys() {
            visit(self, name, &mut Vec::new(), &mut done)?;
        }
        Ok(())
    }

    /// The services a hook waits for, with everything they depend on.
    pub fn hook_dependencies(&self) -> std::collections::BTreeSet<&str> {
        let mut pending: Vec<&str> = self
            .hooks
            .iter()
            .flat_map(|(_, hook)| hook.depends_on.iter().map(String::as_str))
            .collect();
        let mut closure = std::collections::BTreeSet::new();
        while let Some(name) = pending.pop() {
            if closure.insert(name) {
                pending.extend(self.processes[name].depends_on.iter().map(String::as_str));
            }
        }
        closure
    }

    pub fn service_hosts(&self, base: &str) -> Result<BTreeMap<String, String>> {
        if self.services.is_empty() {
            return Ok(BTreeMap::new());
        }
        let base = base
            .strip_suffix(".localhost")
            .context("invalid environment hostname")?;
        dns_label(base)?;
        self.services
            .iter()
            .map(|(name, service)| {
                let label = if Some(name) == self.default_service.as_ref() {
                    base.to_owned()
                } else {
                    format!("{}-{base}", service.subdomain.as_deref().unwrap_or(name))
                };
                dns_label(&label)?;
                Ok((name.clone(), format!("{label}.localhost")))
            })
            .collect()
    }

    pub fn hostname(&self, primary: bool, label: &str) -> Result<String> {
        dns_label(label)?;
        let base = if primary {
            self.project.name.clone()
        } else {
            format!("{label}-{}", self.project.name)
        };
        ensure!(
            primary || base.len() + self.longest_prefix() <= 63,
            "{label}: too long for a hostname with project {}; choose a shorter --name",
            self.project.name
        );
        dns_label(&base)?;
        Ok(format!("{base}.localhost"))
    }

    /// The most bytes a preview label can have so every service host is one DNS label.
    pub fn label_budget(&self) -> usize {
        63usize.saturating_sub(self.project.name.len() + 1 + self.longest_prefix())
    }

    /// `admin-` in `admin-signup-shop`: the longest subdomain prefix a service adds.
    fn longest_prefix(&self) -> usize {
        self.services
            .iter()
            .filter(|(name, _)| Some(*name) != self.default_service.as_ref())
            .map(|(name, service)| service.subdomain.as_deref().unwrap_or(name).len() + 1)
            .max()
            .unwrap_or(0)
    }
}

fn check_environment(environment: &BTreeMap<String, String>) -> Result<()> {
    for (key, value) in environment {
        ensure!(
            !key.is_empty()
                && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !key.as_bytes()[0].is_ascii_digit(),
            "invalid variable name: {key}"
        );
        ensure!(
            !key.starts_with("PARCELA_") && key != "PORT" && key != "HOST",
            "{key} is set by Parcela"
        );
        ensure!(!value.contains('\0'), "{key} contains NUL");
    }
    Ok(())
}

/// Resolve a checkout from the nearest manifest, not from a branch/bookmark.
pub fn locate(start: &Path) -> Result<PathBuf> {
    let start = start
        .canonicalize()
        .with_context(|| format!("resolve {}", start.display()))?;
    ensure!(start.is_dir(), "checkout path must be a directory");
    for directory in start.ancestors() {
        if directory.join("parcela.yaml").is_file() {
            return Ok(directory.to_path_buf());
        }
    }
    bail!(
        "no parcela.yaml in {} or above; create one with `parcela init`, or pass --path to a configured checkout",
        start.display()
    )
}

#[cfg(test)]
mod schema {
    /// The schema editors load from https://parcela.run/schema/v1.json.
    const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/website/schema/v1.json");

    #[test]
    fn published_schema_matches_the_parser() {
        let mut schema = serde_json::to_value(schemars::schema_for!(super::File)).unwrap();
        schema["$id"] = "https://parcela.run/schema/v1.json".into();
        let text = serde_json::to_string_pretty(&schema).unwrap() + "\n";
        if std::env::var_os("PARCELA_UPDATE_SCHEMA").is_some() {
            std::fs::write(PATH, &text).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(PATH).unwrap_or_default(),
            text,
            "regenerate with PARCELA_UPDATE_SCHEMA=1 cargo test --lib schema"
        );
    }
}

use crate::{
    config::Manifest,
    control::{self, Action, EndpointHealth, LiveState, LiveStatus},
    http,
    lifecycle::current_session,
    owner::{Phase, Record},
    private::{self, Lock},
    process::{Listeners, OwnedChild, Signals, exit_code},
    registry::{Endpoint, Registry, Session, SessionStatus, Workspace},
    router::{self, Router},
    share::{self, Share},
    supervisor::{self, Supervisor},
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::os::fd::AsFd;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub fn session_environment(
    manifest: &Manifest,
    workspace: &Workspace,
    runtime: &std::path::Path,
    session: &Session,
    share: &Share,
) -> Result<BTreeMap<String, String>> {
    environment(
        manifest,
        workspace,
        runtime,
        session.public_url.as_deref().unwrap_or_default(),
        session.backend_port,
        &session.endpoints(manifest.default_service.as_deref()),
        &session.ports,
        share,
    )
}
/// The variables every service and task sees. Without a run, pass no endpoints or ports.
#[allow(clippy::too_many_arguments)]
pub fn environment(
    manifest: &Manifest,
    workspace: &Workspace,
    runtime: &std::path::Path,
    url: &str,
    port: Option<u16>,
    endpoints: &[Endpoint],
    ports: &BTreeMap<String, u16>,
    share: &Share,
) -> Result<BTreeMap<String, String>> {
    let mut context = BTreeMap::from([
        ("PARCELA_PROJECT_ID".into(), workspace.project_id.clone()),
        ("PARCELA_ROLE".into(), workspace.role.as_str().into()),
        ("PARCELA_ENVIRONMENT_ID".into(), workspace.id.clone()),
        (
            "PARCELA_GENERATION".into(),
            workspace.generation.to_string(),
        ),
        (
            "PARCELA_RUNTIME_DIR".into(),
            runtime.to_str().context("non-UTF-8 runtime path")?.into(),
        ),
        (
            "PARCELA_DATA_DIR".into(),
            runtime
                .join("data")
                .to_str()
                .context("non-UTF-8 runtime path")?
                .into(),
        ),
        ("PARCELA_ENVIRONMENT_NAME".into(), workspace.label.clone()),
        (
            "PARCELA_PRIMARY_DIR".into(),
            crate::config::main_checkout(&workspace.source)
                .unwrap_or_else(|| workspace.source.clone())
                .to_str()
                .context("non-UTF-8 checkout path")?
                .into(),
        ),
        ("PARCELA_URL".into(), url.into()),
        ("PARCELA_HOSTNAME".into(), workspace.hostname.clone()),
        ("PARCELA_SHARE_HOSTS".into(), share.hosts(endpoints)),
        ("HOST".into(), "127.0.0.1".into()),
        (
            "BROWSER".into(),
            browser(runtime)
                .to_str()
                .context("non-UTF-8 runtime path")?
                .into(),
        ),
    ]);
    if manifest.services.is_empty() {
        context.remove("PARCELA_URL");
        context.remove("PARCELA_HOSTNAME");
    }
    if let Some(port) = port {
        context.insert("PORT".into(), port.to_string());
    }
    for (service, hostname) in manifest.service_hosts(&workspace.hostname)? {
        let prefix = format!(
            "PARCELA_SERVICE_{}",
            service.to_ascii_uppercase().replace('-', "_")
        );
        let endpoint = endpoints
            .iter()
            .find(|endpoint| endpoint.service == service);
        let service_url = endpoint.map_or_else(
            || url.replacen(&workspace.hostname, &hostname, 1),
            |endpoint| endpoint.public_url.clone(),
        );
        context.insert(format!("{prefix}_URL"), service_url);
        context.insert(format!("{prefix}_HOSTNAME"), hostname);
        if let Some(port) = endpoint.map(|endpoint| endpoint.backend_port).or_else(|| {
            (Some(service.as_str()) == manifest.default_service.as_deref())
                .then_some(port)
                .flatten()
        }) {
            context.insert(format!("{prefix}_PORT"), port.to_string());
        }
    }
    for (name, port) in ports {
        context.insert(crate::config::port_variable(name), port.to_string());
    }
    context.extend(crate::compose::environment(
        manifest,
        &workspace.source,
        runtime,
        &workspace.label,
    )?);
    for (key, value) in &manifest.environment {
        let resolved = resolve(key, value, &context)?;
        context.insert(key.clone(), resolved);
    }
    Ok(context)
}

/// Insert `${NAME}` placeholders from `context`; an unknown placeholder is an error.
pub(crate) fn resolve(
    key: &str,
    value: &str,
    context: &BTreeMap<String, String>,
) -> Result<String> {
    let mut resolved = value.to_owned();
    for (name, value) in context {
        resolved = resolved.replace(&format!("${{{name}}}"), value);
    }
    ensure!(
        !resolved.contains("${"),
        "unresolved placeholder in env {key}"
    );
    Ok(resolved)
}
pub(crate) fn command_env(command: &mut Command, context: &BTreeMap<String, String>) {
    // Another environment's variables would point at its ports and directories.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PARCELA_") {
            command.env_remove(key);
        }
    }
    command.env_remove("PORT");
    command.envs(context);
}
fn runtime(registry: &Registry, workspace: &Workspace) -> Result<PathBuf> {
    private::directory(&registry.state_path().join("environments"))?;
    let path = registry.runtime_path(workspace);
    private::directory(&path)?;
    for name in ["data", "tmp", "log", "cache", "bin"] {
        private::directory(&path.join(name))?;
    }
    let parcela = std::env::current_exe()?;
    let parcela = parcela.to_str().context("non-UTF-8 Parcela path")?;
    let shim = browser(&path);
    let script = format!(
        "#!/bin/sh\nexec '{}' browse \"$@\"\n",
        parcela.replace('\'', "'\\''")
    );
    // Running processes may call the shim at any moment, so it is replaced only when it changes.
    if std::fs::read(&shim).ok().as_deref() != Some(script.as_bytes()) {
        private::write_executable(&shim, script.as_bytes())?;
    }
    Ok(path)
}
/// The `BROWSER` every process sees: one executable path, as Python's `webbrowser` requires.
fn browser(runtime: &std::path::Path) -> PathBuf {
    runtime.join("bin").join("browser")
}

#[derive(Clone, Copy)]
pub enum Launch<'a> {
    /// `parcela run -- argv`: one process as the web service.
    Command(&'a [String]),
    /// `parcela up`: the graph from `parcela.yaml`.
    Up,
}
pub fn run(
    registry: &mut Registry,
    manifest: &Manifest,
    workspace: &Workspace,
    launch: Launch<'_>,
    backend_port: Option<u16>,
    timeout: u64,
    request: &share::Request,
) -> Result<i32> {
    preflight(
        registry,
        manifest,
        workspace,
        launch,
        backend_port,
        timeout,
        request,
    )?;
    let router = if manifest.services.is_empty() {
        None
    } else {
        let router = crate::proxy::ensure_running(registry.state_path(), &workspace.hostname)?;
        crate::proxy::offer_trust(registry.state_path());
        Some(router)
    };
    let path = runtime(registry, workspace)?;
    let owner_lock = Lock::acquire(&path.join("owner.lock"))?;
    converge_crashed_session(registry, workspace, &owner_lock)?;
    let (endpoints, ports, probes) =
        crate::ports::allocate(registry, manifest, workspace, router.as_ref(), backend_port)?;
    let default = endpoints
        .iter()
        .find(|endpoint| Some(endpoint.service.as_str()) == manifest.default_service.as_deref());
    let share = Share::plan(
        registry.state_path(),
        request,
        &endpoints,
        router
            .as_ref()
            .map_or(1443, |router| router.front_door.https),
    )?;
    let kind = match launch {
        Launch::Command(_) => "run",
        Launch::Up => "up",
    };
    // Registered before the session exists: an interrupt during startup reaches the owner loop
    // instead of killing Parcela with leased ports and no owner record.
    let signals = Signals::new()?;
    let session = match (router.as_ref(), default) {
        (Some(router), Some(default)) => registry.begin_endpoints(
            workspace,
            &router.token,
            default.backend_port,
            kind,
            &default.public_url,
            &endpoints,
            &ports,
        )?,
        (None, None) => registry.begin_processes(workspace, &ports)?,
        _ => anyhow::bail!("HTTP routing configuration is incomplete"),
    };
    // owner.json is saved before any process starts, so recovery always knows what ran.
    let launched = (|| -> Result<_> {
        let mut workspace = registry.current(&workspace.source, manifest)?;
        workspace.generation = session.generation;
        let env = session_environment(manifest, &workspace, &path, &session, &share)?;
        let sockets = crate::sockets::environment(&path)?;
        private::write_json(
            &path.join("context.json"),
            &json!({"session":session,"environment":env,"source":workspace.source,"sockets":{"control":sockets.control},"isolation":"routing-only; project must consume declared private paths"}),
        )?;
        let nodes = match launch {
            Launch::Command(argv) => {
                ensure!(!argv.is_empty(), "literal command argv required after --");
                vec![supervisor::literal(
                    manifest
                        .default_service
                        .as_deref()
                        .context("run needs an HTTP service")?,
                    argv,
                    &env,
                    &workspace.source,
                )]
            }
            Launch::Up => {
                let hooks = path.join(HOOKS_DIR);
                if hooks.exists() {
                    std::fs::remove_dir_all(&hooks)?;
                }
                private::directory(&hooks)?;
                if let Some(compose) = manifest.compose() {
                    crate::compose::write_override(&path, compose, &session.ports)?;
                }
                let endpoints = session.endpoints(manifest.default_service.as_deref());
                supervisor::plan(
                    manifest,
                    &endpoints,
                    &env,
                    &workspace.source,
                    &path,
                    setup_pending(manifest, &path),
                    &share.processes(
                        manifest,
                        &endpoints,
                        router
                            .as_ref()
                            .map_or(1443, |router| router.front_door.https),
                    )?,
                )?
            }
        };
        let control = control::Server::bind(&path)?;
        let record = Record {
            session: session.clone(),
            phase: Phase::Starting,
            owner: crate::process::Identity::read(std::process::id())?
                .context("owner birth unavailable")?,
            lock: owner_lock.identity()?,
            control: private::Identity::read(&sockets.control)?,
            children: BTreeMap::new(),
            processes: Vec::new(),
            manifest: manifest.clone(),
            diagnostic: None,
            share,
        };
        record.save(&path)?;
        let supervisor = Supervisor::new(nodes, &path)?;
        // This is not an atomic socket handoff. Durable lease and listener ownership checks
        // prevent publishing a foreign listener.
        drop(probes);
        Ok((workspace, control, supervisor, record))
    })();
    let (workspace, control, supervisor, record) = match launched {
        Ok(launched) => launched,
        Err(error) => {
            registry.session_status(&session, SessionStatus::Stopped)?;
            return Err(error);
        }
    };
    let setup = matches!(launch, Launch::Up) && setup_pending(manifest, &path);
    let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let echo = Echo::start(
        &path,
        supervisor.names(),
        matches!(launch, Launch::Up),
        false,
        ready.clone(),
    );
    let result = Owner {
        registry,
        workspace: &workspace,
        path: &path,
        record,
        supervisor,
        control,
        signals,
        setup,
        ready,
    }
    .monitor(timeout);
    echo.finish();
    result
}

/// Prints what every process writes, as `name | line`, while the owner runs: to the terminal in
/// the foreground, and to `owner.log` when detached, there only until the run is first ready, so
/// the file stays small.
struct Echo {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: thread::JoinHandle<()>,
}
impl Echo {
    /// `takeover` is for an owner that takes over a run: it skips what the processes wrote
    /// before and prints to stderr, so the command's own output stays one document.
    fn start<'a>(
        runtime: &std::path::Path,
        names: impl Iterator<Item = &'a str>,
        prefixed: bool,
        takeover: bool,
        ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        use std::io::Write;
        use std::sync::atomic::Ordering;
        let mut names: Vec<String> = names.map(str::to_owned).collect();
        names.sort();
        let width = names.iter().map(String::len).max().unwrap_or(0);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let detached = stdout_is(&runtime.join("log/owner.log"));
        let mut follower = crate::logs::Follower::new(
            &crate::logs::directory(runtime),
            takeover.then_some(0),
            Some(LOG_LIMIT),
        );
        let thread = thread::spawn(move || {
            let paint = if takeover {
                crate::paint::Paint::stderr()
            } else {
                crate::paint::Paint::stdout()
            };
            loop {
                let last = stopped.load(Ordering::Relaxed);
                let quiet = detached && ready.load(Ordering::Relaxed);
                let mut out: Box<dyn Write> = if takeover {
                    Box::new(std::io::stderr().lock())
                } else {
                    Box::new(std::io::stdout().lock())
                };
                // Still read when quiet, so the logs keep rotating.
                let _ = follower.poll(|name, line| {
                    if quiet {
                        return;
                    }
                    let _ = if prefixed {
                        let index = names.iter().position(|known| known == name).unwrap_or(0);
                        writeln!(out, "{} {line}", paint.log_prefix(index, name, width))
                    } else {
                        writeln!(out, "{line}")
                    };
                });
                let _ = out.flush();
                drop(out);
                if last {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        Self { stop, thread }
    }
    /// Prints what is left, then stops.
    fn finish(self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.thread.join();
    }
}

/// Whether stdout is `path`, as for the owner `up --detach` starts with its output in owner.log.
fn stdout_is(path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(file), Ok(out)) = (
        std::fs::metadata(path),
        std::io::stdout().as_fd().try_clone_to_owned(),
    ) else {
        return false;
    };
    std::fs::File::from(out)
        .metadata()
        .is_ok_and(|out| out.dev() == file.dev() && out.ino() == file.ino())
}

/// A process log is copied to `<name>.log.1` and emptied past this size.
const LOG_LIMIT: u64 = 16 * 1024 * 1024;
/// How long the running loop keeps going while it cannot inspect its processes and listeners.
const INSPECTION_GRACE: Duration = Duration::from_secs(30);

/// Whether this run still owes the environment its one-time `hooks.setup`.
fn setup_pending(manifest: &Manifest, runtime: &std::path::Path) -> bool {
    manifest.hooks.setup.is_some() && !runtime.join(SETUP_DONE).exists()
}

/// Written once `hooks.setup` has run and every service answered. `up --setup` removes it.
pub const SETUP_DONE: &str = "setup-complete";
/// Each hook writes `<name>.exit` here when it finishes.
const HOOKS_DIR: &str = "hooks";

/// The graph of a recorded run, for an owner that takes it over.
fn recorded_graph(
    registry: &Registry,
    record: &Record,
    workspace: &Workspace,
    path: &std::path::Path,
) -> Result<Vec<supervisor::Node>> {
    let env = session_environment(
        &record.manifest,
        workspace,
        path,
        &record.session,
        &record.share,
    )?;
    if record.session.kind == "run" {
        // Its one process always started, so its command is never needed again.
        return Ok(vec![supervisor::literal(
            record
                .manifest
                .default_service
                .as_deref()
                .context("run needs an HTTP service")?,
            &[],
            &env,
            &workspace.source,
        )]);
    }
    let endpoints = record
        .session
        .endpoints(record.manifest.default_service.as_deref());
    let https = if record.manifest.services.is_empty() {
        1443
    } else {
        crate::frontdoor::FrontDoor::load(registry.state_path())?.https
    };
    supervisor::plan(
        &record.manifest,
        &endpoints,
        &env,
        &workspace.source,
        path,
        record.children.contains_key("setup") || setup_pending(&record.manifest, path),
        &record
            .share
            .processes(&record.manifest, &endpoints, https)?,
    )
}

pub fn validate_request(
    manifest: &Manifest,
    launch: Launch<'_>,
    port: Option<u16>,
    timeout: u64,
    request: &share::Request,
) -> Result<()> {
    ensure!(
        matches!(launch, Launch::Up) || manifest.services.len() == 1,
        "literal run supports one HTTP service; use up for a service set"
    );
    ensure!(
        (1..=86400).contains(&timeout),
        "readiness timeout must be between 1 and 86400"
    );
    ensure!(
        !manifest.services.is_empty() || (port.is_none() && !request.any()),
        "--backend-port, --lan, --lan-ip, --tailscale and --domain need an HTTP service"
    );
    Ok(())
}

pub fn preflight(
    registry: &Registry,
    manifest: &Manifest,
    workspace: &Workspace,
    launch: Launch<'_>,
    port: Option<u16>,
    timeout: u64,
    request: &share::Request,
) -> Result<()> {
    validate_request(manifest, launch, port, timeout, request)?;
    let path = registry.runtime_path(workspace);
    let mut resolved = workspace.clone();
    resolved.generation += 1;
    // Checked against the saved front door, so a failing launch never starts the proxy.
    if manifest.services.is_empty() {
        crate::proxy::check_watcher_compatibility(registry.state_path())?;
    }
    let active_router = if manifest.services.is_empty() {
        None
    } else {
        Some(Router {
            token: String::new(),
            front_door: crate::frontdoor::FrontDoor::load(registry.state_path())?,
            version: router::VERSION.into(),
            protocol: router::PROTOCOL,
        })
    };
    registry.check_hosts(manifest, workspace)?;
    let mut reusable = false;
    if let Some(port) = port {
        ensure!(
            port >= 1024
                && active_router
                    .as_ref()
                    .is_none_or(|router| !router.listens_on(port)),
            "--backend-port {port} is the proxy's port or below 1024; pick another"
        );
        ensure!(
            !registry
                .sessions()?
                .iter()
                .any(|session| session.leased_ports().contains(&port)
                    && session.environment_id != workspace.id
                    && session.status != SessionStatus::Stopped),
            "--backend-port {port} is leased by another environment; pick another"
        );
        if crate::ports::claim(port).is_none() {
            let busy = || {
                format!(
                    "--backend-port {port} is already in use by {}; pick another",
                    crate::process::port_holder(port)
                )
            };
            let session = registry.session(&workspace.id)?.with_context(busy)?;
            ensure!(
                session.backend_port == Some(port) && session.status != SessionStatus::Stopped,
                busy()
            );
            let record = Record::read(&path, &session)?;
            ensure!(
                record.routable(&path),
                "backend endpoint is not owned by the current routable generation"
            );
            reusable = true;
        }
    }
    let (mut endpoints, ports, _probes) = crate::ports::allocate(
        registry,
        manifest,
        workspace,
        active_router.as_ref(),
        if reusable { None } else { port },
    )?;
    if reusable {
        endpoints
            .iter_mut()
            .find(|endpoint| Some(endpoint.service.as_str()) == manifest.default_service.as_deref())
            .context("default endpoint missing")?
            .backend_port = port.context("reuse port missing")?;
    }
    let default = endpoints
        .iter()
        .find(|endpoint| Some(endpoint.service.as_str()) == manifest.default_service.as_deref());
    ensure!(
        matches!(launch, Launch::Up) || !request.any(),
        "sharing needs up; run starts no publishers"
    );
    let share = Share::plan(
        registry.state_path(),
        request,
        &endpoints,
        active_router
            .as_ref()
            .map_or(1443, |router| router.front_door.https),
    )?;
    let env = environment(
        manifest,
        &resolved,
        &path,
        default.map_or("", |endpoint| endpoint.public_url.as_str()),
        default.map(|endpoint| endpoint.backend_port),
        &endpoints,
        &ports,
        &share,
    )?;
    if matches!(launch, Launch::Up) {
        // Builds the graph only to check it, such as each service's `env`.
        supervisor::plan(
            manifest,
            &endpoints,
            &env,
            &workspace.source,
            &path,
            setup_pending(manifest, &path),
            &share.processes(
                manifest,
                &endpoints,
                active_router
                    .as_ref()
                    .map_or(1443, |router| router.front_door.https),
            )?,
        )?;
    }
    Ok(())
}

/// What a person sees on stderr once every service answers: the same words as `up --detach`,
/// colored only on a terminal without NO_COLOR, and the stop key only on a terminal.
fn ready_notice(
    workspace: &Workspace,
    session: &Session,
    endpoints: &[Endpoint],
    manifest: &Manifest,
    share: &Share,
) -> String {
    use std::io::IsTerminal;
    let terminal = std::io::stderr().is_terminal();
    let paint = |code: &str, text: &str| crate::paint::Paint::stderr().paint(code, text);
    let name = match workspace.role {
        crate::registry::Role::Primary => workspace.project_id.clone(),
        crate::registry::Role::Preview => {
            format!("{} ({})", workspace.label, workspace.project_id)
        }
    };
    let mut text = match session.public_url.as_deref() {
        Some(url) => format!(
            "{} {name} is running at {}\n",
            paint("32", "✓"),
            paint("1;36", url)
        ),
        None => format!(
            "{} {name} is running (no HTTP services)\n",
            paint("32", "✓")
        ),
    };
    // The main service first, so its shared URLs sit under the first line.
    let (main, others): (Vec<_>, Vec<_>) = endpoints.iter().partition(|endpoint| {
        Some(endpoint.service.as_str()) == manifest.default_service.as_deref()
    });
    for endpoint in main.into_iter().chain(others) {
        if Some(endpoint.service.as_str()) != manifest.default_service.as_deref() {
            text.push_str(&format!(
                "  {:<10}{}\n",
                endpoint.service,
                paint("1;36", &endpoint.public_url)
            ));
        }
        for url in share.urls(endpoint) {
            text.push_str(&format!("  {:<10}{}\n", "", paint("1;36", &url)));
        }
    }
    if terminal {
        text.push_str(&format!("  {}\n", paint("2", "Press Ctrl-C to stop")));
    }
    text
}

fn revoke(registry: &Registry) {
    if let Ok(router) = router::current(registry.state_path())
        && router::sync(registry, &router).is_ok()
    {
        return;
    }
    thread::sleep(Duration::from_millis(router::LEASE_MS + 250));
}

/// The owner of one running generation: its evidence record, process graph and control socket.
struct Owner<'a> {
    registry: &'a mut Registry,
    workspace: &'a Workspace,
    path: &'a std::path::Path,
    record: Record,
    supervisor: Supervisor,
    control: control::Server,
    signals: Signals,
    /// Whether this generation runs `hooks.setup`, so readiness marks it done.
    setup: bool,
    /// Set once every service answered; an adopted run that was ready starts set.
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Owner<'_> {
    /// Supervise until a stop request, a signal, the supervisor's exit or a failure, then stop
    /// the graph and record the outcome.
    fn monitor(mut self, timeout: u64) -> Result<i32> {
        let mut requested_stop = self.record.phase == Phase::Stopping;
        let result = self.watch(timeout, &mut requested_stop);
        self.wind_down()?;
        result
    }

    fn enter(&mut self, phase: Phase) -> Result<()> {
        self.record.phase = phase;
        self.record.save(self.path)?;
        self.registry
            .session_status(&self.record.session, phase.status())
    }

    fn watch(&mut self, timeout: u64, requested_stop: &mut bool) -> Result<i32> {
        let endpoints = self
            .record
            .session
            .endpoints(self.record.manifest.default_service.as_deref());
        let mut deadline = Instant::now() + Duration::from_secs(timeout);
        let mut ready = false;
        // A process that exits after the first ready is reported, not fatal.
        let mut became_ready = self.ready.load(std::sync::atomic::Ordering::Relaxed);
        let mut listener = BTreeMap::new();
        let mut published = None;
        let mut proxy_checked = Instant::now();
        let mut proxy_error: Option<String> = None;
        let mut inspection_failing: Option<Instant> = None;
        loop {
            if *requested_stop {
                return Ok(0);
            }
            self.advance()?;
            if let Some(failure) = self.supervisor.failure(!became_ready) {
                anyhow::bail!(failure);
            }
            if let Some(code) = self.supervisor.finished() {
                return Ok(code);
            }
            if self.signals.received() != 0 {
                *requested_stop = true;
                return Ok(128 + self.signals.received() as i32);
            }
            let observation = self.record.refresh(self.path).and_then(|()| {
                let listeners = Listeners::inspect(endpoints.iter().map(|e| e.backend_port))?;
                let roots = self.supervisor.roots();
                let observed = endpoints
                    .iter()
                    .map(|endpoint| {
                        Ok((
                            endpoint.service.clone(),
                            listeners.owned(endpoint.backend_port, &roots)?,
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>>>()?;
                Ok((listeners, observed))
            });
            // `ps` and `lsof` can miss their deadline on a loaded machine, so a failed check is
            // retried, and only one that keeps failing stops the environment.
            let (listeners, observed) = match observation {
                Ok(observation) => {
                    inspection_failing = None;
                    observation
                }
                Err(error) => {
                    let since = *inspection_failing.get_or_insert_with(Instant::now);
                    if since.elapsed() >= INSPECTION_GRACE {
                        return Err(error);
                    }
                    self.idle(Duration::from_millis(200), requested_stop)?;
                    continue;
                }
            };
            if ready && observed != listener {
                self.enter(Phase::Starting)?;
                if let Ok(router) = router::current(self.registry.state_path()) {
                    let _ = router::sync(self.registry, &router);
                }
                ready = false;
                deadline = Instant::now() + Duration::from_secs(timeout);
            }
            let startup = if became_ready {
                Vec::new()
            } else {
                self.supervisor.waiting()
            };
            // Why each service is not ready yet, for the timeout message.
            let waiting = endpoints
                .iter()
                .filter_map(|endpoint| {
                    let path = &self.record.manifest.services[&endpoint.service].readiness_path;
                    let port = endpoint.backend_port;
                    let reason = if observed[&endpoint.service].is_empty() {
                        if listeners.on(port).is_ok_and(|held| !held.is_empty()) {
                            format!(
                                "port {port} is held by {}",
                                crate::process::port_holder(port)
                            )
                        } else {
                            format!("nothing listens on its $PORT, {port}")
                        }
                    } else if ready {
                        return None;
                    } else {
                        match listeners
                            .address(port)
                            .and_then(|address| http::status(address, &endpoint.hostname, path))
                        {
                            Ok(code) if (200..300).contains(&code) => return None,
                            Ok(code) => format!("GET {path} returned {code}"),
                            Err(error) => format!("GET {path} failed: {error:#}"),
                        }
                    };
                    Some(format!("{}: {reason}", endpoint.service))
                })
                .chain(startup)
                .collect::<Vec<_>>();
            let healthy = waiting.is_empty();
            listener = observed;
            if healthy != ready {
                ready = healthy;
                became_ready |= ready;
                if ready {
                    self.ready.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.enter(if ready {
                    Phase::Running
                } else {
                    Phase::Starting
                })?;
                published = None;
                if ready {
                    eprint!(
                        "{}",
                        ready_notice(
                            self.workspace,
                            &self.record.session,
                            &endpoints,
                            &self.record.manifest,
                            &self.record.share,
                        )
                    );
                    private::write_json(
                        &self.path.join("last-successful.json"),
                        &self.record.manifest,
                    )?;
                    if self.setup {
                        private::write_bytes(&self.path.join(SETUP_DONE), b"")?;
                    }
                }
            }
            // The routes come back by themselves once the new proxy runs: its token differs.
            if !endpoints.is_empty() && proxy_checked.elapsed() >= Duration::from_secs(2) {
                proxy_checked = Instant::now();
                if router::current(self.registry.state_path()).is_err() {
                    match crate::proxy::revive(self.registry.state_path()) {
                        Ok(true) => {
                            proxy_error = None;
                            eprintln!("parcela: the proxy had stopped; started it again")
                        }
                        Ok(false) => {}
                        // Retried every 2s until the port frees up; said once per cause.
                        Err(error) => {
                            let error = format!("{error:#}");
                            if proxy_error.as_ref() != Some(&error) {
                                eprintln!(
                                    "parcela: the proxy is down and did not restart: {error}"
                                );
                                proxy_error = Some(error);
                            }
                        }
                    }
                }
            }
            if !endpoints.is_empty()
                && let Ok(router) = router::current(self.registry.state_path())
                && published.as_ref() != Some(&router.token)
            {
                if endpoints
                    .iter()
                    .all(|endpoint| router.url(&endpoint.hostname) == endpoint.public_url)
                {
                    self.registry
                        .rebind_router(&self.record.session, &router.token)?;
                    if router::sync(self.registry, &router).is_ok() {
                        published = Some(router.token);
                    }
                } else {
                    eprintln!(
                        "parcela: router URL changed; application stays alive but unrouted until explicit restart"
                    );
                }
            }
            // A hook can take longer than the readiness timeout, which counts from its end.
            if !ready && self.supervisor.hooks_running() {
                deadline = Instant::now() + Duration::from_secs(timeout);
            }
            ensure!(
                ready || Instant::now() < deadline,
                "not ready after {timeout}s: {}",
                waiting.join("; ")
            );
            self.idle(Duration::from_millis(200), requested_stop)?;
        }
    }

    /// Wait between checks, answering control requests as they arrive rather than after the wait.
    fn idle(&mut self, wait: Duration, requested_stop: &mut bool) -> Result<()> {
        let until = Instant::now() + wait;
        while !*requested_stop && Instant::now() < until {
            let Some(pending) = self.control.receive(&self.record.session)? else {
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            let response = self.answer(&pending.action, requested_stop);
            if let Err(error) = pending.reply(&self.record.session, response) {
                eprintln!("parcela: owner reply failed: {error:#}");
            }
        }
        Ok(())
    }

    /// Ticks the graph and saves which processes started and exited, right after each start.
    fn advance(&mut self) -> Result<()> {
        let (record, path) = (&mut self.record, self.path);
        let changed = self.supervisor.tick(|graph| {
            record.children = graph.children();
            record.save(path)
        })?;
        if changed {
            self.record.children = self.supervisor.children();
            self.record.save(self.path)?;
        }
        Ok(())
    }

    /// One control request while the generation runs.
    fn answer(&mut self, action: &Action, requested_stop: &mut bool) -> Result<serde_json::Value> {
        current_session(self.registry, &self.record.session)?;
        match action {
            Action::Status => live_status(
                self.registry,
                &self.record.session,
                &self.supervisor.roots(),
                &self.record.manifest,
                &self.record.share,
            )
            .map(|mut live| {
                live["services"] = self.supervisor.list()["services"].take();
                live
            }),
            Action::Stop => {
                self.enter(Phase::Stopping)?;
                *requested_stop = true;
                Ok(stop_accepted(&self.record.session))
            }
            Action::Services => Ok(self.supervisor.list()),
        }
    }

    /// Withdraw the routes, stop the graph and record whether it provably stopped.
    fn wind_down(&mut self) -> Result<()> {
        self.record.refresh(self.path)?;
        self.enter(Phase::Stopping)?;
        if self.record.manifest.default_service.is_some() {
            revoke(self.registry);
        }
        self.supervisor.stop();
        let deadline =
            Instant::now() + Duration::from_secs(self.record.manifest.shutdown_timeout_seconds);
        // Processes left behind by a child that exited, signaled by verified identity.
        let mut stragglers: Option<Instant> = None;
        loop {
            self.advance()?;
            self.record.refresh(self.path)?;
            if self.supervisor.finished().is_some() {
                if self.record.quiescent() {
                    break;
                }
                let signal = match stragglers {
                    None => libc::SIGTERM,
                    Some(sent) if sent.elapsed() >= Duration::from_secs(10) => libc::SIGKILL,
                    Some(_) => 0,
                };
                if signal != 0 {
                    stragglers.get_or_insert_with(Instant::now);
                    for identity in &self.record.processes {
                        crate::process::signal_verified(identity, signal, false)?;
                    }
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            if let Some(pending) = self.control.receive(&self.record.session)? {
                let response = match pending.action {
                    Action::Stop => Ok(stop_accepted(&self.record.session)),
                    Action::Status => live_status(
                        self.registry,
                        &self.record.session,
                        &self.supervisor.roots(),
                        &self.record.manifest,
                        &self.record.share,
                    ),
                    Action::Services => Ok(self.supervisor.list()),
                };
                let _ = pending.reply(&self.record.session, response);
            }
            thread::sleep(Duration::from_millis(50));
        }
        if !(self.supervisor.finished().is_some() && self.record.quiescent()) {
            self.record.diagnostic =
                Some("shutdown incomplete; lease retained until its processes exit".into());
            self.enter(Phase::Interrupted)?;
            anyhow::bail!(
                "shutdown incomplete after {}s; endpoint lease retained",
                self.record.manifest.shutdown_timeout_seconds
            );
        }
        // Its processes are gone, but what the stop command manages may still run.
        if let Some(failure) = self.supervisor.stop_failure() {
            self.record.diagnostic = Some(failure.clone());
            self.enter(Phase::Interrupted)?;
            anyhow::bail!("shutdown incomplete: {failure}; run parcela down again");
        }
        self.record.phase = Phase::Stopped;
        self.record.save(self.path)?;
        self.registry
            .session_status(&self.record.session, SessionStatus::Stopped)
    }
}

fn stop_accepted(session: &Session) -> serde_json::Value {
    json!({"stop_accepted":true,"operation_id":session.token,"generation":session.generation})
}

pub fn recover(
    registry: &mut Registry,
    workspace: &Workspace,
    expected: &Session,
    stop: bool,
) -> Result<serde_json::Value> {
    ensure!(
        expected.environment_id == workspace.id,
        "recovery target changed"
    );
    let session = current_session(registry, expected)?;
    ensure!(
        stop || session.status != SessionStatus::Stopped,
        "session is already stopped; use an explicit up to start it"
    );
    let path = registry.runtime_path(workspace);
    private::existing_directory(&path)?;
    let sockets = crate::sockets::environment(&path)?;
    let budget = Record::read(&path, &session)?
        .manifest
        .shutdown_timeout_seconds;
    let deadline = Instant::now() + Duration::from_secs(budget + 10);
    let lock = loop {
        current_session(registry, expected)?;
        if let Some(lock) = Lock::try_acquire(&path.join("owner.lock"))? {
            break lock;
        }
        ensure!(
            stop,
            "recovery operation is already owned by another process"
        );
        ensure!(
            Instant::now() < deadline,
            "same-generation stop recovery did not complete; ownership retained, inspect owner logs"
        );
        let record = Record::read(&path, expected)?;
        if matches!(record.phase, Phase::Starting | Phase::Running)
            && record.owner.alive()
            && record.control.matches(&sockets.control)
            && record.lock.locked(&path.join("owner.lock"))?
        {
            let _ = control::request(&path, expected, Action::Stop);
        }
        thread::sleep(Duration::from_millis(50));
    };
    let session = current_session(registry, expected)?;
    if stop && session.status == SessionStatus::Stopped {
        return Ok(
            json!({"environment_id":workspace.id,"generation":session.generation,"state":"stopped","already_stopped":true}),
        );
    }
    let mut record = Record::read(&path, &session)?;
    ensure!(
        record.abandoned(&lock)?,
        "original owner or ownership lock is not safely recoverable"
    );
    if converge_crashed(registry, workspace, &session, &lock)? {
        return Ok(
            json!({"environment_id":workspace.id,"generation":session.generation,"state":"stopped","recovered":true}),
        );
    }
    record.control.remove_socket(&sockets.control)?;
    let control = control::Server::bind(&path)?;
    record.owner = crate::process::Identity::read(std::process::id())?
        .context("recovery owner birth unavailable")?;
    record.control = private::Identity::read(&sockets.control)?;
    // Exits recorded after the first ready are not startup failures for the new owner either.
    let was_ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        record.phase == Phase::Running,
    ));
    record.phase = if stop
        || session.status == SessionStatus::Stopping
        || matches!(record.phase, Phase::Stopping | Phase::Interrupted)
    {
        Phase::Stopping
    } else {
        Phase::Starting
    };
    record.refresh(&path)?;
    record.save(&path)?;
    registry.session_status(&session, record.phase.status())?;
    let mut resolved = workspace.clone();
    resolved.generation = session.generation;
    let timeout = record.manifest.readiness_timeout_seconds;
    let nodes = recorded_graph(registry, &record, &resolved, &path)?;
    let supervisor = Supervisor::adopt(nodes, &path, &record.children);
    let setup = record.session.kind == "up" && setup_pending(&record.manifest, &path);
    // A stop prints nothing, so `down` stays one JSON document.
    let echo = (!stop).then(|| {
        Echo::start(
            &path,
            supervisor.names(),
            record.session.kind == "up",
            true,
            was_ready.clone(),
        )
    });
    let result = Owner {
        registry,
        workspace: &resolved,
        path: &path,
        record,
        supervisor,
        control,
        signals: Signals::new()?,
        setup,
        ready: was_ready,
    }
    .monitor(timeout);
    if let Some(echo) = echo {
        echo.finish();
    }
    result?;
    Ok(
        json!({"environment_id":workspace.id,"generation":session.generation,"state":"stopped","recovered":true}),
    )
}

/// Marks a run stopped once its owner and every recorded process are gone by birth identity.
/// It signals nothing; any surviving or unprovable identity leaves the session untouched.
fn converge_crashed(
    registry: &mut Registry,
    workspace: &Workspace,
    session: &Session,
    lock: &Lock,
) -> Result<bool> {
    let path = &registry.runtime_path(workspace);
    let Ok(mut record) = Record::read(path, session) else {
        // The owner died after leasing ports and before saving its record, which it does before
        // starting any process. The caller holds owner.lock, so that owner is gone.
        if session.status != SessionStatus::Starting {
            return Ok(false);
        }
        control::remove_dead(path)?;
        registry.session_status(session, SessionStatus::Stopped)?;
        return Ok(true);
    };
    if !record.abandoned(lock)? || !record.quiescent() {
        return Ok(false);
    }
    finish_stop_commands(registry, workspace, &mut record, path)?;
    record.phase = Phase::Stopped;
    record.save(path)?;
    record
        .control
        .remove_socket(&crate::sockets::environment(path)?.control)?;
    registry.session_status(session, SessionStatus::Stopped)?;
    if record.manifest.default_service.is_some()
        && let Ok(router) = router::current(registry.state_path())
    {
        let _ = router::sync(registry, &router);
    }
    Ok(true)
}

/// Runs the `stop` commands a crashed run still owes, for services whose processes are gone but
/// whose containers or other outside resources may still run. The caller holds owner.lock.
fn finish_stop_commands(
    registry: &mut Registry,
    workspace: &Workspace,
    record: &mut Record,
    path: &std::path::Path,
) -> Result<()> {
    let owed = record.children.iter().any(|(name, child)| {
        !child.stopped
            && record
                .manifest
                .processes
                .get(name)
                .is_some_and(|process| process.stop.is_some())
    });
    if !owed {
        return Ok(());
    }
    let mut resolved = workspace.clone();
    resolved.generation = record.session.generation;
    let nodes = recorded_graph(registry, record, &resolved, path)?;
    let mut supervisor = Supervisor::adopt(nodes, path, &record.children);
    supervisor.stop();
    let deadline = Instant::now() + Duration::from_secs(record.manifest.shutdown_timeout_seconds);
    while supervisor.finished().is_none() && Instant::now() < deadline {
        if supervisor.tick(|_| Ok(()))? {
            record.children = supervisor.children();
            record.save(path)?;
        }
        thread::sleep(Duration::from_millis(50));
    }
    record.children = supervisor.children();
    let failure = supervisor.stop_failure().or_else(|| {
        supervisor
            .finished()
            .is_none()
            .then(|| "a stop command did not finish".to_owned())
    });
    if let Some(failure) = failure {
        record.diagnostic = Some(failure.clone());
        record.phase = Phase::Interrupted;
        record.save(path)?;
        registry.session_status(&record.session, SessionStatus::Interrupted)?;
        anyhow::bail!("shutdown incomplete: {failure}; run parcela down again");
    }
    record.save(path)
}

/// Lets a new run replace a predecessor that crashed together with its whole recorded tree.
pub fn converge_crashed_session(
    registry: &mut Registry,
    workspace: &Workspace,
    lock: &Lock,
) -> Result<bool> {
    if let Some(session) = registry.session(&workspace.id)?
        && session.status != SessionStatus::Stopped
    {
        return converge_crashed(registry, workspace, &session, lock);
    }
    Ok(false)
}

pub fn task(
    registry: &Registry,
    manifest: &Manifest,
    workspace: &Workspace,
    name: &str,
    args: &[String],
) -> Result<i32> {
    let task = manifest.tasks.get(name).with_context(|| {
        if manifest.tasks.is_empty() {
            format!("unknown task {name}; parcela.yaml has no tasks")
        } else {
            let names: Vec<&str> = manifest.tasks.keys().map(String::as_str).collect();
            format!("unknown task {name}; parcela.yaml has {}", names.join(", "))
        }
    })?;
    let path = runtime(registry, workspace)?;
    let _lock = Lock::acquire(&path.join("task.lock"))?;
    let session = registry.session(&workspace.id)?;
    let url = crate::lifecycle::optional_url(registry, workspace)?.unwrap_or_default();
    let env = if let Some(session) = &session {
        session_environment(manifest, workspace, &path, session, &Share::default())?
    } else {
        environment(
            manifest,
            workspace,
            &path,
            &url,
            None,
            &[],
            &BTreeMap::new(),
            &Share::default(),
        )?
    };
    // Extra arguments follow the command, like `parcela task test -- spec/models`.
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(format!("{} \"$@\"", task.command))
        .arg(name)
        .args(args)
        .current_dir(&workspace.source);
    command_env(&mut command, &env);
    let signals = Signals::new()?;
    let mut child = OwnedChild::spawn(&mut command)?;
    let deadline = task
        .timeout_seconds
        .map(|seconds| Instant::now() + Duration::from_secs(seconds));
    let (code, outcome) = loop {
        if let Some(status) = child.poll()? {
            break (exit_code(status), "exited");
        }
        if signals.received() != 0 {
            break (
                exit_code(child.stop(signals.received() as i32)?),
                "interrupted",
            );
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            child.stop(libc::SIGTERM)?;
            break (124, "timeout");
        }
        thread::sleep(Duration::from_millis(25));
    };
    private::write_json(
        &path.join("last-task.json"),
        &json!({"task":name,"exit_code":code,"outcome":outcome,"generation":workspace.generation,"environment_id":workspace.id,"automatic_retry":false}),
    )?;
    Ok(code)
}

/// Runs `hooks.teardown` for a stopped environment before `rm` forgets it. Its output goes to
/// stderr, so `--json` stays parseable. Runs from the checkout, or the runtime directory once the
/// checkout is gone.
pub fn teardown(registry: &Registry, manifest: &Manifest, workspace: &Workspace) -> Result<()> {
    let Some(hook) = &manifest.hooks.teardown else {
        return Ok(());
    };
    let path = registry.runtime_path(workspace);
    let env = match registry.session(&workspace.id)? {
        Some(session) => {
            session_environment(manifest, workspace, &path, &session, &Share::default())?
        }
        None => environment(
            manifest,
            workspace,
            &path,
            &crate::lifecycle::optional_url(registry, workspace)?.unwrap_or_default(),
            None,
            &[],
            &BTreeMap::new(),
            &Share::default(),
        )?,
    };
    let directory = if workspace.source.is_dir() {
        workspace.source.clone()
    } else {
        path.clone()
    };
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(&hook.command)
        .current_dir(directory)
        .stdout(std::process::Stdio::from(
            std::io::stderr().as_fd().try_clone_to_owned()?,
        ));
    command_env(&mut command, &env);
    let status = command.status().context("run hooks.teardown")?;
    ensure!(
        status.success(),
        "hooks.teardown failed with exit code {}; the environment stays registered",
        exit_code(status)
    );
    Ok(())
}

fn live_status(
    registry: &Registry,
    session: &Session,
    roots: &[u32],
    manifest: &Manifest,
    share: &Share,
) -> Result<serde_json::Value> {
    let current = current_session(registry, session)?;
    let mut endpoints = BTreeMap::new();
    let mut listeners = Vec::new();
    let all = session.endpoints(manifest.default_service.as_deref());
    let snapshot = Listeners::inspect(all.iter().map(|endpoint| endpoint.backend_port))?;
    for endpoint in all {
        let pids = snapshot.owned(endpoint.backend_port, roots)?;
        let http_ready = !pids.is_empty()
            && snapshot
                .address(endpoint.backend_port)
                .and_then(|address| {
                    http::readiness(
                        address,
                        &endpoint.hostname,
                        &manifest.services[&endpoint.service].readiness_path,
                    )
                })
                .unwrap_or(false);
        listeners.extend(&pids);
        endpoints.insert(
            endpoint.service.clone(),
            EndpointHealth {
                shared: share.urls(&endpoint),
                hostname: endpoint.hostname,
                public_url: endpoint.public_url,
                backend_port: endpoint.backend_port,
                listener_owned: !pids.is_empty(),
                listener_pids: pids,
                http_ready,
            },
        );
    }
    listeners.sort_unstable();
    listeners.dedup();
    let http_ready =
        !endpoints.is_empty() && endpoints.values().all(|endpoint| endpoint.http_ready);
    let listener_owned =
        !endpoints.is_empty() && endpoints.values().all(|endpoint| endpoint.listener_owned);
    let current_router = if manifest.services.is_empty() {
        None
    } else {
        router::current(registry.state_path()).ok()
    };
    let router_owned = current_router
        .as_ref()
        .is_some_and(|router| Some(&router.token) == current.router_token.as_ref());
    let route_published = current_router
        .as_ref()
        .is_some_and(|router| router::route_published(registry, router, &current));
    let state = if current.status == SessionStatus::Stopping {
        LiveState::Stopping
    } else if current.status == SessionStatus::Ready
        && (manifest.services.is_empty() || (http_ready && router_owned && route_published))
    {
        LiveState::Ready
    } else if current.status == SessionStatus::Starting {
        LiveState::Starting
    } else {
        LiveState::Unhealthy
    };
    Ok(serde_json::to_value(LiveStatus {
        state,
        http_ready,
        listener_owned,
        router_owned,
        route_published,
        owner_pid: std::process::id(),
        listener_pids: listeners,
        endpoints,
        checked_at_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    })?)
}

//! `parcela up` and `parcela run`: check a request, lease its ports and start an owner.
use super::{
    echo::Echo,
    health::Health,
    owner::Owner,
    recovery::converge_crashed_session,
    variables::{build, for_session, prepare_directory},
};
use crate::{
    environment::control,
    environment::record::{Phase, Record},
    environment::supervisor::{self, Supervisor},
    process::Signals,
    project::config::Manifest,
    proxy::router::{self, Router},
    share::{self, Share},
    state::private::{self, Lock},
    state::registry::{Registry, SessionStatus, Workspace},
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::collections::BTreeMap;

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
    let path = prepare_directory(registry, workspace)?;
    let owner_lock = Lock::acquire(&path.join("owner.lock"))?;
    converge_crashed_session(registry, workspace, &owner_lock)?;
    let (endpoints, ports, probes) = crate::state::ports::allocate(
        registry,
        manifest,
        workspace,
        router.as_ref(),
        backend_port,
    )?;
    let default = endpoints
        .iter()
        .find(|endpoint| Some(endpoint.service.as_str()) == manifest.default_service.as_deref());
    let share = Share::plan(
        registry.state_path(),
        request,
        &endpoints,
        https_port(router.as_ref()),
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
        let env = for_session(manifest, &workspace, &path, &session, &share)?;
        let sockets = crate::state::sockets::environment(&path)?;
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
                    crate::project::compose::write_override(&path, compose, &session.ports)?;
                }
                let endpoints = session.endpoints(manifest.default_service.as_deref());
                supervisor::plan(
                    manifest,
                    &endpoints,
                    &env,
                    &workspace.source,
                    &path,
                    setup_pending(manifest, &path),
                    &share.processes(manifest, &endpoints, https_port(router.as_ref()))?,
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
    let health = Health::start(&record);
    let result = Owner {
        registry,
        health,
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

/// Whether this run still owes the environment its one-time `hooks.setup`.
pub(super) fn setup_pending(manifest: &Manifest, runtime: &std::path::Path) -> bool {
    manifest.hooks.setup.is_some() && !runtime.join(SETUP_DONE).exists()
}

/// Written once `hooks.setup` has run and every service answered. `up --setup` removes it.
pub const SETUP_DONE: &str = "setup-complete";
/// Each hook writes `<name>.exit` here when it finishes.
const HOOKS_DIR: &str = "hooks";

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
            front_door: crate::proxy::frontdoor::FrontDoor::load(registry.state_path())?,
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
        if crate::state::ports::claim(port).is_none() {
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
    let (mut endpoints, ports, _probes) = crate::state::ports::allocate(
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
        https_port(active_router.as_ref()),
    )?;
    let env = build(
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
            &share.processes(manifest, &endpoints, https_port(active_router.as_ref()))?,
        )?;
    }
    Ok(())
}

/// The front door's HTTPS port, or the default one for a run that needs no proxy.
fn https_port(router: Option<&Router>) -> u16 {
    router.map_or(crate::proxy::frontdoor::DEFAULT_HTTPS_PORT, |router| {
        router.front_door.https
    })
}

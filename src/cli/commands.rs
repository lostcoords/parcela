use super::{
    args::{Cli, Command, Global},
    output::{Output, status_diagnostics},
    render,
};
use anyhow::{Context, Result, ensure};
use parcela::{
    config::{Manifest, locate},
    context::ResolvedContext,
    frontdoor::FrontDoor,
    lifecycle,
    registry::{NOT_STARTED, Registry, Workspace},
    share::Request as Share,
    startup,
};
use std::path::{Path, PathBuf};

pub(super) fn state_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    let path = if let Some(path) = explicit {
        path
    } else if let Some(path) = std::env::var_os("PARCELA_STATE_DIR") {
        path.into()
    } else if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        let path = PathBuf::from(path);
        ensure!(path.is_absolute(), "XDG_STATE_HOME must be absolute");
        path.join("parcela")
    } else {
        PathBuf::from(std::env::var_os("HOME").context("HOME missing; set --state-dir")?)
            .join(".local/state/parcela")
    };
    ensure!(
        !path.as_os_str().is_empty(),
        "state directory must not be empty"
    );
    Ok(path)
}
pub(super) fn project(path: &Path) -> Result<(PathBuf, Manifest)> {
    let source = locate(path)?;
    let manifest = Manifest::load(&source.join("parcela.yaml"))?;
    Ok((source, manifest))
}
/// The environment `--workspace ID` names, else the one registered for the checkout at `path`.
pub(super) fn target_workspace(
    registry: &Registry,
    path: &Path,
    id: Option<uuid::Uuid>,
) -> Result<Workspace> {
    match id {
        Some(id) => by_id(registry, id),
        None => {
            let (source, manifest) = project(path)?;
            registry.current(&source, &manifest)
        }
    }
}
/// The environment `selector` names: its ID, one of its URLs or hostnames, with or without
/// `.localhost`, `.local` and the port, or its name when only one environment has it.
fn select(registry: &Registry, selector: &str) -> Result<Workspace> {
    if let Ok(id) = uuid::Uuid::parse_str(selector) {
        return by_id(registry, id);
    }
    let lower = selector.to_ascii_lowercase();
    let host = lower
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(['/', ':'])
        .next()
        .unwrap_or_default();
    let label = |host: &str| -> String {
        host.strip_suffix(".localhost")
            .or_else(|| host.strip_suffix(".local"))
            .unwrap_or(host)
            .to_owned()
    };
    let wanted = label(host);
    let workspaces = registry.list(None)?;
    for workspace in &workspaces {
        let hosts = registry.hosts(&workspace.id)?;
        if hosts
            .values()
            .chain([&workspace.hostname])
            .any(|host| label(host) == wanted)
        {
            return Ok(workspace.clone());
        }
    }
    let mut named: Vec<_> = workspaces
        .into_iter()
        .filter(|workspace| workspace.label == selector)
        .collect();
    match named.len() {
        0 => anyhow::bail!(unknown_environment(selector)),
        1 => Ok(named.remove(0)),
        _ => anyhow::bail!(
            "{selector} names {} environments; use one of: {}",
            named.len(),
            named
                .iter()
                .map(|workspace| label(&workspace.hostname))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
fn service_url(
    registry: &Registry,
    workspace: &Workspace,
    service: Option<String>,
) -> Result<String> {
    match service {
        Some(service) => lifecycle::service_url(registry, workspace, &service),
        None => lifecycle::url(registry, workspace),
    }
}
fn unknown_environment(selector: &str) -> String {
    match uuid::Uuid::parse_str(selector) {
        Ok(_) => UNKNOWN_ID.to_owned(),
        Err(_) => format!("no environment named {selector}; list environments with parcela ls"),
    }
}
const UNKNOWN_ID: &str = "unknown environment ID; list environments with parcela ls";
fn by_id(registry: &Registry, id: uuid::Uuid) -> Result<Workspace> {
    registry
        .list(None)?
        .into_iter()
        .find(|workspace| workspace.id == id.to_string())
        .context(UNKNOWN_ID)
}
/// Open the registry for writing, which creates it on first use.
fn writable(state: &Path) -> Result<Registry> {
    Registry::open(state, true)?.context(NOT_STARTED)
}
fn startup_registry(
    state: &Path,
    source: &Path,
    manifest: &Manifest,
    request: &startup::Request,
) -> Result<(Registry, Workspace)> {
    let existing = Registry::open(state, false)?;
    let identity = request.resolve(existing.as_ref(), manifest, source)?;
    drop(existing);
    let mut registry = writable(state)?;
    let workspace = registry.adopt(manifest, source, identity.role, &identity.label)?;
    Ok((registry, workspace))
}
/// `parcela` alone: status inside a started checkout, otherwise the next command to run.
pub fn bare(global: Global) -> Result<i32> {
    let out = Output::detect(global.json);
    let hint = match project(&global.path) {
        Err(_) => "No parcela.yaml here. Create one with `parcela init`, then run `parcela up`.",
        Ok((source, manifest)) => {
            let registered = Registry::open(&state_dir(global.state_dir.clone())?, false)?
                .is_some_and(|registry| registry.current(&source, &manifest).is_ok());
            if registered {
                return run(Cli::new(global, Command::Status));
            }
            "Not started yet. Run `parcela up`."
        }
    };
    out.emit(&serde_json::json!({"hint": hint}), |_| {
        format!("{hint}\nSee `parcela --help` for every command.\n")
    })?;
    Ok(0)
}
/// Show what a failed detached start wrote to its log, not output from earlier runs.
fn print_new_log_lines(log: &Path, start: u64, count: usize) {
    use std::io::{Read, Seek};
    let mut text = String::new();
    let Ok(mut file) = std::fs::File::open(log) else {
        return;
    };
    if file.seek(std::io::SeekFrom::Start(start)).is_err()
        || file.read_to_string(&mut text).is_err()
    {
        return;
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return;
    }
    eprintln!("parcela: last lines of {}:", log.display());
    for line in &lines[lines.len().saturating_sub(count)..] {
        let line: String = line
            .chars()
            .filter(|c| !c.is_control() || *c == '\t')
            .collect();
        eprintln!("  {line}");
    }
}

pub fn run(mut cli: Cli) -> Result<i32> {
    let out = Output::detect(cli.json);
    if matches!(cli.command, Command::Skill) {
        use std::io::Write;
        std::io::stdout()
            .lock()
            .write_all(include_bytes!("../../.agents/skills/parcela/SKILL.md"))?;
        return Ok(0);
    }
    if let Command::Completion { shell } = cli.command {
        use clap::CommandFactory;
        clap_complete::generate(
            shell,
            &mut super::args::Args::command(),
            "parcela",
            &mut std::io::stdout(),
        );
        return Ok(0);
    }
    if let Command::ComposeForward { service, port } = &cli.command {
        parcela::compose::forward(service, *port)?;
        return Ok(0);
    }
    if let Command::Upgrade { nightly } = cli.command {
        return super::upgrade::run(nightly);
    }
    let state = state_dir(cli.state_dir)?;
    let mut id = None;
    if let Some(selector) = &cli.workspace
        && !matches!(
            cli.command,
            Command::Init { .. } | Command::Telemetry { .. }
        )
    {
        let registry =
            Registry::open(&state, false)?.with_context(|| unknown_environment(selector))?;
        let workspace = select(&registry, selector)?;
        id = Some(uuid::Uuid::parse_str(&workspace.id)?);
        cli.path = workspace.source;
    }
    let reader = || -> Result<(Registry, Workspace)> {
        let Some(registry) = Registry::open(&state, false)? else {
            // Report a missing parcela.yaml before the unstarted checkout.
            project(&cli.path)?;
            anyhow::bail!(NOT_STARTED);
        };
        let workspace = target_workspace(&registry, &cli.path, id)?;
        Ok((registry, workspace))
    };
    match cli.command {
        Command::Skill
        | Command::Upgrade { .. }
        | Command::Completion { .. }
        | Command::ComposeForward { .. } => unreachable!("handled above"),
        Command::Create {
            name,
            directory,
            revision,
            from,
        } => super::environments::create(
            &state,
            &cli.path,
            &name,
            directory.as_deref(),
            revision.as_deref(),
            from.as_deref(),
            out,
        ),
        Command::Snapshot { name } => {
            let (registry, workspace) = reader()?;
            let (_, manifest) = project(&workspace.source)?;
            super::environments::snapshot(&registry, &manifest, &workspace, &name, out)
        }
        Command::Restore { name } => {
            let (registry, workspace) = reader()?;
            let (_, manifest) = project(&workspace.source)?;
            super::environments::restore(&registry, &manifest, &workspace, &name, out)
        }
        Command::Init { name, command } => {
            let front_door = FrontDoor::load(&state)?;
            super::init::run(
                &cli.path,
                name.as_deref(),
                command.as_deref(),
                front_door,
                out,
            )?;
            Ok(0)
        }
        Command::Doctor { caddy } => {
            let ready = super::doctor::run(
                super::doctor::Probes {
                    path: &cli.path,
                    state: &state,
                    caddy: caddy.as_deref(),
                },
                out,
            )?;
            Ok(if ready { 0 } else { 1 })
        }
        Command::Proxy { command } => super::proxy::run(&state, command, out),
        Command::Telemetry { command } => super::telemetry::run(
            &state,
            command.unwrap_or(super::args::TelemetryCommand::Status),
            out,
        ),
        Command::Trust { print, caddy } => super::proxy::trust(&state, print, &caddy),
        Command::Run {
            name,
            primary,
            backend_port,
            readiness_timeout,
            argv,
        } => {
            let (source, manifest) =
                project(&cli.path).context("`parcela run` needs a project's parcela.yaml")?;
            let request = startup::Request { name, primary };
            let (mut registry, workspace) = startup_registry(&state, &source, &manifest, &request)?;
            parcela::runtime::run(
                &mut registry,
                &manifest,
                &workspace,
                parcela::runtime::Launch::Command(&argv),
                backend_port,
                readiness_timeout,
                Default::default(),
            )
        }
        Command::Up {
            name,
            primary,
            backend_port,
            readiness_timeout,
            detach,
            setup,
            lan,
            lan_ip,
            tailscale,
            launch_file,
        } => start(
            &state,
            &cli.path,
            Start {
                request: startup::Request { name, primary },
                restart: false,
                backend_port,
                readiness_timeout,
                detach,
                setup,
                share: Share {
                    lan,
                    lan_ip,
                    tailscale,
                },
                launch_file,
            },
            out,
        ),
        Command::Restart {
            backend_port,
            readiness_timeout,
            detach,
            setup,
            lan,
            lan_ip,
            tailscale,
            launch_file,
        } => start(
            &state,
            &cli.path,
            Start {
                request: startup::Request::default(),
                restart: true,
                backend_port,
                readiness_timeout,
                detach,
                setup,
                share: Share {
                    lan,
                    lan_ip,
                    tailscale,
                },
                launch_file,
            },
            out,
        ),
        Command::Recover { detach } => {
            let (existing, workspace) = reader()?;
            let result = if detach {
                lifecycle::recover_detached(&existing, &workspace)?
            } else {
                let session = existing
                    .session(&workspace.id)?
                    .context("no session to recover")?;
                drop(existing);
                parcela::runtime::recover(&mut writable(&state)?, &workspace, &session, false)?
            };
            out.emit(&result, |style| render::recovered(&result, style))?;
            Ok(0)
        }
        Command::Down {
            all: true,
            all_projects,
        }
        | Command::Down {
            all_projects: all_projects @ true,
            ..
        } => {
            let project = if all_projects {
                None
            } else {
                Some(project(&cli.path)?.1.project.id)
            };
            super::inventory::down_all(&state, project.as_deref(), out)
        }
        Command::Down { .. } => {
            let (registry, workspace) = reader()?;
            let result = lifecycle::down(&registry, &workspace)?;
            let name = render::name(&serde_json::to_value(&workspace)?);
            out.emit(&result, |style| render::down(&result, &name, style))?;
            Ok(0)
        }
        Command::Ls => {
            super::inventory::list(&state, &cli.path, out)?;
            Ok(0)
        }
        Command::Status => {
            let (registry, workspace) = reader()?;
            let status = lifecycle::status_with_processes(&registry, &workspace)?;
            if out == Output::Human {
                status_diagnostics(std::slice::from_ref(&status));
            }
            out.emit(&status, |style| render::status(&status, style))?;
            Ok(0)
        }
        Command::Logs {
            service,
            tail,
            follow: true,
        } => {
            let (registry, workspace) = reader()?;
            lifecycle::follow_logs(&registry, &workspace, service.as_deref(), tail)?;
            Ok(0)
        }
        Command::Logs { service, tail, .. } => {
            let (registry, workspace) = reader()?;
            let logs = lifecycle::logs(&registry, &workspace, service.as_deref(), tail as usize)?;
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            let paint = parcela::paint::Paint::stdout();
            let width = logs.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
            for (index, (name, lines)) in logs.iter().enumerate() {
                for line in lines {
                    if service.is_some() {
                        writeln!(out, "{line}")?;
                    } else {
                        writeln!(out, "{} {line}", paint.log_prefix(index, name, width))?;
                    }
                }
            }
            Ok(0)
        }
        Command::Url { service } => {
            let (registry, workspace) = reader()?;
            let url = service_url(&registry, &workspace, service)?;
            println!("{url}");
            Ok(0)
        }
        Command::Open { service } => {
            let (registry, workspace) = reader()?;
            let status = lifecycle::status(&registry, &workspace)?;
            anyhow::ensure!(
                status["state"] != "stopped",
                "{} is not running; start it with `parcela up`",
                render::name(&status["workspace"])
            );
            let url = service_url(&registry, &workspace, service)?;
            // $BROWSER may carry arguments, like `firefox -P work`, so the shell splits it.
            let browser = std::env::var("BROWSER")
                .ok()
                .filter(|browser| !browser.is_empty());
            let name = browser.clone().unwrap_or_else(|| {
                if cfg!(target_os = "macos") {
                    "open"
                } else {
                    "xdg-open"
                }
                .into()
            });
            let opened = std::process::Command::new("/bin/sh")
                .args(["-c", &format!("{name} \"$1\""), "sh", &url])
                .status()
                .with_context(|| format!("run {name}"))?;
            anyhow::ensure!(opened.success(), "{name} could not open {url}");
            out.emit(&serde_json::json!({ "url": url }), |style| {
                format!("Opened {}\n", style.url(&url))
            })?;
            Ok(0)
        }
        Command::Rm { prune } => {
            super::inventory::remove(&state, &cli.path, id, prune, out)?;
            Ok(0)
        }
        Command::Task { name, args } => {
            let (source, manifest) = project(&cli.path)?;
            let registry = writable(&state)?;
            let workspace = registry.current(&source, &manifest)?;
            parcela::runtime::task(&registry, &manifest, &workspace, &name, &args)
        }
        Command::Context => {
            let (source, manifest) = project(&cli.path)?;
            let registry = Registry::open(&state, false)?.context(NOT_STARTED)?;
            let workspace = registry.current(&source, &manifest)?;
            let result =
                serde_json::to_value(ResolvedContext::new(&registry, &manifest, workspace)?)?;
            out.emit(&result, |style| render::context(&result, style))?;
            Ok(0)
        }
    }
}

/// What `up` and `restart` were asked to do.
struct Start {
    request: startup::Request,
    restart: bool,
    backend_port: Option<u16>,
    /// `--readiness-timeout`; `readiness_timeout_seconds` from parcela.yaml otherwise.
    readiness_timeout: Option<u64>,
    detach: bool,
    setup: bool,
    share: Share,
    /// Set only for the detached owner `up --detach` launches, pinning its source and manifest.
    launch_file: Option<PathBuf>,
}

fn start(state: &Path, path: &Path, start: Start, out: Output) -> Result<i32> {
    let launch = parcela::runtime::Launch::Up;
    let snapshot: Option<lifecycle::LaunchSnapshot> = start
        .launch_file
        .as_ref()
        .map(|path| parcela::private::read_json(path))
        .transpose()?;
    let (source, manifest) = if let Some(snapshot) = &snapshot {
        ensure!(
            path.canonicalize()? == snapshot.source,
            "launch source changed"
        );
        (snapshot.source.clone(), snapshot.manifest.clone())
    } else {
        let (source, mut manifest) = project(path)?;
        parcela::compose::resolve(&mut manifest, &source)?;
        (source, manifest)
    };
    let readiness_timeout = start
        .readiness_timeout
        .unwrap_or(manifest.readiness_timeout_seconds);
    parcela::runtime::validate_request(
        &manifest,
        launch,
        start.backend_port,
        readiness_timeout,
        start.share,
    )?;
    let (mut registry, workspace) = if start.restart || snapshot.is_some() {
        ensure!(
            start.request.name.is_none() && !start.request.primary,
            "pinned startup cannot change environment identity"
        );
        let registry = writable(state)?;
        let workspace = registry.current(&source, &manifest)?;
        (registry, workspace)
    } else {
        startup_registry(state, &source, &manifest, &start.request)?
    };
    if let Some(snapshot) = &snapshot {
        ensure!(
            start.launch_file.as_ref()
                == Some(&registry.runtime_path(&workspace).join("launch.json")),
            "unexpected launch snapshot path"
        );
        let current = registry.session(&workspace.id)?;
        ensure!(
            current.as_ref().map(|s| (&s.token, s.generation))
                == snapshot
                    .predecessor
                    .as_ref()
                    .map(|s| (&s.token, s.generation)),
            "launch predecessor changed; successor preserved"
        );
    } else if !start.restart
        && let Some(session) = lifecycle::orphaned(&registry, &workspace)?
    {
        eprintln!(
            "parcela: the Parcela process that started {} exited; taking back control of its running services",
            render::name(&serde_json::to_value(&workspace)?)
        );
        if !manifest.services.is_empty() {
            parcela::proxy::ensure_running(registry.state_path(), &workspace.hostname)?;
        }
        let result = if start.detach {
            lifecycle::recover_detached(&registry, &workspace)?
        } else {
            parcela::runtime::recover(&mut registry, &workspace, &session, false)?
        };
        out.emit(&result, |style| render::recovered(&result, style))?;
        return Ok(0);
    } else if !start.restart
        && let Some(result) = already_running(&registry, &workspace, start.share)?
    {
        out.emit(&result, |style| render::started(&result, style))?;
        return Ok(0);
    }
    let restart_lock = if start.restart {
        parcela::private::directory(&registry.runtime_path(&workspace))?;
        Some(parcela::private::Lock::wait(
            &registry.runtime_path(&workspace).join("restart.lock"),
        )?)
    } else {
        None
    };
    // A service was added, removed or renamed: its URLs can only be reserved once the old run stops.
    let new_hosts = start.restart
        && snapshot.is_none()
        && registry.hosts(&workspace.id)? != manifest.service_hosts(&workspace.hostname)?;
    if start.restart || start.detach {
        parcela::runtime::preflight(
            &registry,
            &manifest,
            &workspace,
            launch,
            start.backend_port,
            readiness_timeout,
            start.share,
        )?;
    }
    if start.restart {
        ensure!(
            registry
                .session(&workspace.id)?
                .is_none_or(|session| session.kind == "up"),
            "restart does not replay arbitrary run commands; use down then an explicit run"
        );
        lifecycle::down(&registry, &workspace)?;
    }
    let workspace = if new_hosts {
        let workspace = registry
            .adopt(&manifest, &source, workspace.role, &workspace.label)
            .context("the environment stopped, but the new service URLs could not be reserved")?;
        parcela::runtime::preflight(
            &registry,
            &manifest,
            &workspace,
            launch,
            start.backend_port,
            readiness_timeout,
            start.share,
        )?;
        workspace
    } else {
        workspace
    };
    if start.setup {
        let marker = registry
            .runtime_path(&workspace)
            .join(parcela::runtime::SETUP_DONE);
        match std::fs::remove_file(&marker) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error).context("forget the finished setup");
            }
            _ => {}
        }
    }
    if !start.detach {
        drop(restart_lock);
        return parcela::runtime::run(
            &mut registry,
            &manifest,
            &workspace,
            launch,
            start.backend_port,
            readiness_timeout,
            start.share,
        );
    }
    // Started here so its notice reaches this terminal, not the detached owner's log.
    if !manifest.services.is_empty() {
        parcela::proxy::ensure_running(registry.state_path(), &workspace.hostname)?;
        parcela::proxy::offer_trust(registry.state_path());
    }
    let log = registry.runtime_path(&workspace).join("log/owner.log");
    let offset = std::fs::metadata(&log).map_or(0, |metadata| metadata.len());
    let result = lifecycle::detached(
        &registry,
        &workspace,
        start.backend_port,
        readiness_timeout,
        start.share,
        &manifest,
    )
    .inspect_err(|_| print_new_log_lines(&log, offset, 20))?;
    out.emit(&result, |style| render::started(&result, style))?;
    Ok(0)
}

/// `up` on a ready environment reports it like a fresh start, as `docker compose up -d` does.
/// A ready run without the sharing asked for is refused.
fn already_running(
    registry: &Registry,
    workspace: &Workspace,
    share: Share,
) -> Result<Option<serde_json::Value>> {
    if registry
        .session(&workspace.id)?
        .is_none_or(|session| session.status == parcela::registry::SessionStatus::Stopped)
    {
        return Ok(None);
    }
    let status = lifecycle::status(registry, workspace)?;
    let name = render::name(&serde_json::to_value(workspace)?);
    // Anything but ready goes on to the start path, which takes back or replaces a crashed run.
    let state = status["state"].as_str().unwrap_or("unknown");
    if state != "ready" {
        return Ok(None);
    }
    let shared: Vec<&str> = status["live"]["endpoints"]
        .as_object()
        .into_iter()
        .flat_map(|endpoints| endpoints.values())
        .flat_map(|endpoint| endpoint["shared"].as_array().into_iter().flatten())
        .filter_map(serde_json::Value::as_str)
        .collect();
    let host = |url: &str| -> String {
        let authority = url.split('/').nth(2).unwrap_or_default();
        authority.split(':').next().unwrap_or_default().to_owned()
    };
    for (asked, flag, suffix) in [
        (share.lan || share.lan_ip.is_some(), "--lan", ".local"),
        (share.tailscale, "--tailscale", ".ts.net"),
    ] {
        ensure!(
            !asked || shared.iter().any(|url| host(url).ends_with(suffix)),
            "{name} is already running without {flag}; run `parcela restart {flag}`"
        );
    }
    Ok(Some(serde_json::json!({
        "already_running": true,
        "workspace": status["workspace"],
        "session": status["recorded_session"],
        "live": status["live"],
        "state": state,
    })))
}

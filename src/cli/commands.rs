use super::{
    args::{Cli, Command, Global},
    output::Output,
};
use anyhow::{Context, Result, ensure};
use parcela::{
    environment::lifecycle,
    project::config::{Manifest, locate},
    project::startup,
    proxy::frontdoor::FrontDoor,
    share::Request as Share,
    state::registry::{NOT_STARTED, Registry, Workspace},
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
            let (source, _) = project(path)?;
            registry.registered(&source)
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
pub(super) fn writable(state: &Path) -> Result<Registry> {
    Registry::open(state, true)?.context(NOT_STARTED)
}
pub(super) fn startup_registry(
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
        Ok((source, _)) => {
            let registered = Registry::open(&state_dir(global.state_dir.clone())?, false)?
                .is_some_and(|registry| registry.registered(&source).is_ok());
            if registered {
                return run(Cli::new(global, Command::Status { current: true }));
            }
            "Not started yet. Run `parcela up`."
        }
    };
    out.emit(&serde_json::json!({"hint": hint}), |_| {
        format!("{hint}\nSee `parcela --help` for every command.\n")
    })?;
    Ok(0)
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
        parcela::project::compose::forward(service, *port)?;
        return Ok(0);
    }
    if let Command::Upgrade { nightly } = cli.command {
        return super::upgrade::run(nightly, &state_dir(cli.state_dir)?);
    }
    let state = state_dir(cli.state_dir)?;
    let mut id = None;
    if let Some(selector) = &cli.workspace
        && !matches!(
            cli.command,
            Command::Init { .. }
                | Command::Telemetry { .. }
                | Command::Config { .. }
                | Command::Relay { .. }
                | Command::Autostart { .. }
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
        Command::Config { command } => super::config::run(
            &state,
            command.unwrap_or(super::args::ConfigCommand::List),
            out,
        ),
        Command::Autostart { command } => super::autostart::run(
            &state,
            command.unwrap_or(super::args::AutostartCommand::Status),
            out,
        ),
        Command::Relay { command } => super::proxy::relay(
            &state,
            command.unwrap_or(super::args::RelayCommand::Status),
            out,
        ),
        Command::Trust {
            print,
            export,
            remove,
            caddy,
        } => super::proxy::trust(
            &state,
            super::proxy::TrustAction::from_flags(print, export, remove),
            &caddy.unwrap_or_else(parcela::state::host::caddy),
        ),
        Command::Run {
            name,
            primary,
            backend_port,
            readiness_timeout,
            argv,
        } => super::start::literal(
            &state,
            &cli.path,
            &startup::Request { name, primary },
            backend_port,
            readiness_timeout,
            &argv,
        ),
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
            domains,
            domain_port,
            no_share,
            launch_file,
        } => super::start::start(
            &state,
            &cli.path,
            super::start::Start {
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
                    domains,
                    domain_port,
                },
                no_share,
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
            domains,
            domain_port,
            no_share,
            launch_file,
        } => super::start::start(
            &state,
            &cli.path,
            super::start::Start {
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
                    domains,
                    domain_port,
                },
                no_share,
                launch_file,
            },
            out,
        ),
        Command::Recover { detach } => {
            let (existing, workspace) = reader()?;
            super::lifecycle::recover(&state, existing, &workspace, detach, out)
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
            super::lifecycle::down(&registry, &workspace, out)
        }
        Command::Ls => {
            super::inventory::list(&state, &cli.path, out, false)?;
            Ok(0)
        }
        Command::Ui => super::lifecycle::ui(&state, out),
        Command::Status { current: false } if id.is_none() => {
            super::inventory::list(&state, &cli.path, out, true)?;
            Ok(0)
        }
        Command::Status { .. } => {
            let (registry, workspace) = reader()?;
            super::lifecycle::status(&registry, &workspace, out)
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
            super::lifecycle::logs(&registry, &workspace, service.as_deref(), tail as usize)
        }
        Command::Browse { targets } => super::lifecycle::browse(&targets, out),
        Command::Url { service } => {
            let (registry, workspace) = reader()?;
            super::lifecycle::url(&registry, &workspace, service.as_deref())
        }
        Command::Open { service } => {
            let (registry, workspace) = reader()?;
            super::lifecycle::open(&registry, &workspace, service.as_deref(), out)
        }
        Command::Rm { prune } => {
            super::inventory::remove(&state, &cli.path, id, prune, out)?;
            Ok(0)
        }
        Command::Task { name, args } => super::lifecycle::task(&state, &cli.path, &name, &args),
        Command::Context => super::lifecycle::context(&state, &cli.path, out),
    }
}

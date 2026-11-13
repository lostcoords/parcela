//! `parcela up`, `restart` and `run`: pin or find the checkout, apply sharing defaults, take back
//! an orphaned run or report a ready one, then start in the foreground or detached.
use super::{
    commands::{project, startup_registry, writable},
    output::Output,
    render,
};
use anyhow::{Context, Result, ensure};
use parcela::{
    environment::lifecycle,
    project::startup,
    share::Request as Share,
    state::registry::{Registry, Workspace},
};
use std::path::{Path, PathBuf};

/// `parcela run -- argv`: one literal command as the web service, in the foreground.
pub(super) fn literal(
    state: &Path,
    path: &Path,
    request: &startup::Request,
    backend_port: Option<u16>,
    readiness_timeout: u64,
    argv: &[String],
) -> Result<i32> {
    let (source, manifest) =
        project(path).context("`parcela run` needs a project's parcela.yaml")?;
    let (mut registry, workspace) = startup_registry(state, &source, &manifest, request)?;
    parcela::environment::launch::run(
        &mut registry,
        &manifest,
        &workspace,
        parcela::environment::launch::Launch::Command(argv),
        backend_port,
        readiness_timeout,
        &Default::default(),
    )
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

/// What `up` and `restart` were asked to do.
pub(super) struct Start {
    pub(super) request: startup::Request,
    pub(super) restart: bool,
    pub(super) backend_port: Option<u16>,
    /// `--readiness-timeout`; `readiness_timeout_seconds` from parcela.yaml otherwise.
    pub(super) readiness_timeout: Option<u64>,
    pub(super) detach: bool,
    pub(super) setup: bool,
    pub(super) share: Share,
    pub(super) no_share: bool,
    /// Set only for the detached owner `up --detach` launches, pinning its source and manifest.
    pub(super) launch_file: Option<PathBuf>,
}

pub(super) fn start(state: &Path, path: &Path, mut start: Start, out: Output) -> Result<i32> {
    let launch = parcela::environment::launch::Launch::Up;
    let snapshot: Option<lifecycle::LaunchSnapshot> = start
        .launch_file
        .as_ref()
        .map(|path| parcela::state::private::read_json(path))
        .transpose()?;
    let (source, manifest) = if let Some(snapshot) = &snapshot {
        ensure!(
            path.canonicalize()? == snapshot.source,
            "launch source changed"
        );
        (snapshot.source.clone(), snapshot.manifest.clone())
    } else {
        let (source, mut manifest) = project(path)?;
        parcela::project::compose::resolve(&mut manifest, &source)?;
        (source, manifest)
    };
    if start.no_share {
        start.share = Share::default();
    } else if snapshot.is_none() {
        parcela::state::host::Host::current()?.share(&mut start.share, &manifest);
    }
    ensure!(
        start.share.domain_port.is_none() || !start.share.domains.is_empty(),
        "--domain-port requires --domain or `parcela config add sharing.domains`"
    );
    let readiness_timeout = start
        .readiness_timeout
        .unwrap_or(manifest.readiness_timeout_seconds);
    parcela::environment::launch::validate_request(
        &manifest,
        launch,
        start.backend_port,
        readiness_timeout,
        &start.share,
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
    if start.no_share && !start.restart && registry.session(&workspace.id)?.is_some() {
        let status = lifecycle::status(&registry, &workspace)?;
        let state = status["state"].as_str().unwrap_or("unknown");
        let shared = status["live"]["endpoints"]
            .as_object()
            .is_some_and(|endpoints| {
                endpoints.values().any(|endpoint| {
                    endpoint["shared"]
                        .as_array()
                        .is_some_and(|urls| !urls.is_empty())
                })
            });
        ensure!(
            state == "stopped" || (state == "ready" && !shared),
            "existing run is shared or cannot be verified local-only; run `parcela restart --no-share`"
        );
    }
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
        parcela::paint::Paint::note(&format!(
            "the Parcela process that started {} exited; taking back control of its running services",
            render::name(&serde_json::to_value(&workspace)?)
        ));
        if !manifest.services.is_empty() {
            parcela::proxy::ensure_running(registry.state_path(), &workspace.hostname)?;
        }
        let result = if start.detach {
            lifecycle::recover_detached(&registry, &workspace)?
        } else {
            parcela::environment::recovery::recover(&mut registry, &workspace, &session, false)?
        };
        out.emit(&result, |style| render::recovered(&result, style))?;
        return Ok(0);
    } else if !start.restart
        && let Some(result) = already_running(&registry, &workspace, &start.share)?
    {
        out.emit(&result, |style| render::started(&result, style))?;
        return Ok(0);
    }
    let restart_lock = if start.restart {
        parcela::state::private::directory(&registry.runtime_path(&workspace))?;
        Some(parcela::state::private::Lock::wait(
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
        parcela::environment::launch::preflight(
            &registry,
            &manifest,
            &workspace,
            launch,
            start.backend_port,
            readiness_timeout,
            &start.share,
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
        parcela::environment::launch::preflight(
            &registry,
            &manifest,
            &workspace,
            launch,
            start.backend_port,
            readiness_timeout,
            &start.share,
        )?;
        workspace
    } else {
        workspace
    };
    if start.setup {
        let marker = registry
            .runtime_path(&workspace)
            .join(parcela::environment::launch::SETUP_DONE);
        match std::fs::remove_file(&marker) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error).context("forget the finished setup");
            }
            _ => {}
        }
    }
    if !start.detach {
        drop(restart_lock);
        return parcela::environment::launch::run(
            &mut registry,
            &manifest,
            &workspace,
            launch,
            start.backend_port,
            readiness_timeout,
            &start.share,
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
        &start.share,
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
    share: &Share,
) -> Result<Option<serde_json::Value>> {
    if registry
        .session(&workspace.id)?
        .is_none_or(|session| session.status == parcela::state::registry::SessionStatus::Stopped)
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
    let domains: Vec<(String, String)> = share
        .domains
        .iter()
        .map(|domain| {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            (format!("--domain {domain}"), format!(".{domain}"))
        })
        .collect();
    let asked = [
        (
            share.lan || share.lan_ip.is_some(),
            "--lan".into(),
            ".local".into(),
        ),
        (share.tailscale, "--tailscale".into(), ".ts.net".into()),
    ]
    .into_iter()
    .chain(
        domains
            .into_iter()
            .map(|(flag, suffix)| (true, flag, suffix)),
    );
    for (asked, flag, suffix) in asked {
        ensure!(
            !asked || shared.iter().any(|url| host(url).ends_with(&suffix)),
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

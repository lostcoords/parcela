//! Client-side local lifecycle commands. Effects go through the live owner, never saved PIDs.
use crate::{
    control::{self, Action, LiveState, LiveStatus},
    frontdoor::FrontDoor,
    private,
    process::{OwnedChild, Signals},
    registry::{Registry, Session, SessionStatus, Workspace},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    process::Command,
    time::{Duration, Instant},
};

pub fn status(registry: &Registry, workspace: &Workspace) -> Result<Value> {
    Ok(status_and_services(registry, workspace)?.0)
}

/// The status, plus the process list when the owner sent one with it.
fn status_and_services(
    registry: &Registry,
    workspace: &Workspace,
) -> Result<(Value, Option<Value>)> {
    let recorded = registry.session(&workspace.id)?;
    let mut live = None;
    let mut services = None;
    let mut error = None;
    let mut state = "stopped";
    if let Some(session) = &recorded
        && session.status != SessionStatus::Stopped
    {
        state = "unknown";
        match control::request(&registry.runtime_path(workspace), session, Action::Status).and_then(
            |mut value| {
                services = value.get_mut("services").map(Value::take);
                Ok(serde_json::from_value::<LiveStatus>(value)?)
            },
        ) {
            Ok(value) => {
                state = match value.state {
                    LiveState::Ready => "ready",
                    LiveState::Unhealthy => "unhealthy",
                    LiveState::Starting => "starting",
                    LiveState::Stopping => "stopping",
                };
                live = Some(value);
            }
            Err(failure) => error = Some(format!("{failure:#}")),
        }
    }
    Ok((
        json!({"workspace": workspace, "url": optional_url(registry, workspace)?, "source_exists": workspace.source.is_dir(), "recorded_session": recorded,
        "state": state, "live_health_verified": live.is_some(), "live": live, "diagnostic": error}),
        services,
    ))
}

/// One environment's status plus `processes`: every service, hook and worker of the run with its
/// `name`, `state` and `pid` (null when not running). The list is empty unless a live owner answers.
pub fn status_with_processes(registry: &Registry, workspace: &Workspace) -> Result<Value> {
    let (mut status, services) = status_and_services(registry, workspace)?;
    let mut processes = Vec::new();
    if status["live_health_verified"] == true && status["recorded_session"]["kind"] == "up" {
        // Owners from releases before the list came with the status still answer separately.
        let listed = match services {
            Some(services) => Ok(json!({ "services": services })),
            None => service_request(registry, workspace, Action::Services),
        };
        match listed {
            Ok(listed) => {
                processes = listed["services"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|process| {
                        json!({
                            "name": process["name"],
                            "state": process["status"].as_str().map(str::to_ascii_lowercase),
                            "pid": process["pid"].as_u64().filter(|pid| *pid > 0),
                        })
                    })
                    .collect();
            }
            Err(error) => {
                status["diagnostic"] = json!(format!("process list unavailable: {error:#}"))
            }
        }
    }
    status["processes"] = json!(processes);
    Ok(status)
}

pub fn down(registry: &Registry, workspace: &Workspace) -> Result<Value> {
    let Some(session) = registry.session(&workspace.id)? else {
        return Ok(
            json!({"environment_id": workspace.id, "state": "stopped", "already_stopped": true}),
        );
    };
    if session.status == SessionStatus::Stopped {
        return Ok(
            json!({"environment_id": workspace.id, "state": "stopped", "already_stopped": true}),
        );
    }
    let path = registry.runtime_path(workspace);
    let finish_stop = || -> Result<Value> {
        let mut writer =
            Registry::open(registry.state_path(), true)?.context("registry missing")?;
        crate::runtime::recover(&mut writer, workspace, &session, true)
    };
    let owner_died =
        || crate::owner::Record::read(&path, &session).is_ok_and(|record| !record.owner.alive());
    if session.status != SessionStatus::Stopping
        && let Err(error) = control::request(&path, &session, Action::Stop)
    {
        let current = current_session(registry, &session)?;
        if current.status == SessionStatus::Starting
            && path.join("owner.lock").exists()
            && let Some(lock) = private::Lock::try_acquire(&path.join("owner.lock"))?
        {
            let mut writer =
                Registry::open(registry.state_path(), true)?.context("registry missing")?;
            if crate::runtime::converge_crashed_session(&mut writer, workspace, &lock)? {
                return Ok(
                    json!({"environment_id": workspace.id, "generation": session.generation, "state": "stopped", "already_stopped": false}),
                );
            }
        }
        if current.status != SessionStatus::Stopping && current.status != SessionStatus::Stopped {
            let evidence = crate::owner::Record::read(&path, &session).with_context(|| {
                format!("{error:#}; authenticated recovery evidence unavailable")
            })?;
            ensure!(
                !evidence.owner.alive(),
                "owner still exists; retry the same generation after the pending command"
            );
            return finish_stop();
        }
    }
    if session.status == SessionStatus::Stopping && owner_died() {
        return finish_stop();
    }
    let budget = crate::owner::Record::read(&path, &session)
        .map_or(300, |record| record.manifest.shutdown_timeout_seconds);
    let deadline = Instant::now() + Duration::from_secs(budget + 10);
    loop {
        let current = registry
            .session(&workspace.id)?
            .context("session disappeared during stop")?;
        ensure!(
            current.token == session.token && current.generation == session.generation,
            "owner changed during stop; successor was not stopped"
        );
        if current.status == SessionStatus::Stopping && owner_died() {
            return finish_stop();
        }
        if current.status == SessionStatus::Interrupted {
            let reason = crate::owner::Record::read(&path, &current)
                .ok()
                .and_then(|record| record.diagnostic)
                .map(|reason| {
                    reason
                        .trim_start_matches("shutdown incomplete; ")
                        .to_owned()
                })
                .unwrap_or_else(|| "a process did not exit".into());
            anyhow::bail!("shutdown incomplete: {reason}; run parcela down again");
        }
        if current.status == SessionStatus::Stopped
            && let Ok(_released) = private::Lock::acquire(&path.join("owner.lock"))
        {
            return Ok(
                json!({"environment_id": workspace.id, "generation": session.generation, "state": "stopped", "already_stopped": false}),
            );
        }
        ensure!(
            Instant::now() < deadline,
            "stop did not complete; ownership is unknown, no PID was signaled. Inspect owner logs"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The append-only log a detached owner writes, refused unless it is a private regular file.
fn owner_log(runtime: &std::path::Path) -> Result<(std::path::PathBuf, std::fs::File)> {
    private::directory(&runtime.join("log"))?;
    let path = runtime.join("log/owner.log");
    let log = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)?;
    let metadata = log.metadata()?;
    // SAFETY: getuid has no pointers or failure case.
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::getuid() }
            && metadata.mode() & 0o077 == 0,
        "owner log must be a private user-owned regular file"
    );
    Ok((path, log))
}

pub fn detached(
    registry: &Registry,
    workspace: &Workspace,
    port: Option<u16>,
    timeout: u64,
    share: crate::share::Request,
    manifest: &crate::config::Manifest,
) -> Result<Value> {
    let path = registry.runtime_path(workspace);
    private::directory(&path)?;
    let _launch = private::Lock::acquire(&path.join("launch.lock"))?;
    if let Some(owner) = private::Lock::try_acquire(&path.join("owner.lock"))? {
        let mut writer =
            Registry::open(registry.state_path(), true)?.context("registry missing")?;
        crate::runtime::converge_crashed_session(&mut writer, workspace, &owner)?;
    }
    let previous = registry.session(&workspace.id)?;
    ensure!(
        previous
            .as_ref()
            .is_none_or(|session| session.status == SessionStatus::Stopped),
        "this environment is already running or was interrupted; see `parcela status`"
    );
    let (log_path, log) = owner_log(&path)?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--path")
        .arg(&workspace.source)
        .arg("--state-dir")
        .arg(registry.state_path())
        .args(["up", "--readiness-timeout", &timeout.to_string()]);
    if let Some(port) = port {
        command.args(["--backend-port", &port.to_string()]);
    }
    if share.lan {
        command.arg("--lan");
    }
    if let Some(ip) = share.lan_ip {
        command.arg("--lan-ip").arg(ip.to_string());
    }
    if share.tailscale {
        command.arg("--tailscale");
    }
    private::write_json(
        &path.join("launch.json"),
        &LaunchSnapshot {
            source: workspace.source.clone(),
            manifest: manifest.clone(),
            predecessor: previous.clone(),
        },
    )?;
    command.arg("--launch-file").arg(path.join("launch.json"));
    let signals = Signals::new()?;
    let mut owner = OwnedChild::spawn_logged(&mut command, log)?;
    // The owner enforces the readiness timeout itself, and does not count time spent in hooks.
    loop {
        ensure!(signals.received() == 0, "detached startup interrupted");
        ensure!(
            owner.poll()?.is_none(),
            "services stopped before they were ready; full log: {}",
            log_path.display()
        );
        if let Some(session) = registry.session(&workspace.id)?
            && previous
                .as_ref()
                .is_none_or(|old| old.token != session.token)
            && session.status == SessionStatus::Ready
            && let Ok(live) = control::request(&path, &session, Action::Status)
            && live["state"] == "ready"
            && live["owner_pid"].as_u64() == Some(u64::from(owner.id()))
        {
            let mut resolved_workspace = workspace.clone();
            resolved_workspace.generation = session.generation;
            owner.detach();
            return Ok(
                json!({"workspace": resolved_workspace, "session": session, "detached": true, "live": live, "owner_log": log_path}),
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn service_request(
    registry: &Registry,
    workspace: &Workspace,
    action: Action,
) -> Result<Value> {
    let session = registry
        .session(&workspace.id)?
        .context("environment is not running")?;
    ensure!(
        session.status != SessionStatus::Stopped && session.kind == "up",
        "environment is not running"
    );
    control::request(&registry.runtime_path(workspace), &session, action)
}

pub fn current_session(registry: &Registry, expected: &Session) -> Result<Session> {
    let current = registry
        .session(&expected.environment_id)?
        .context("owner session missing")?;
    ensure!(
        current.token == expected.token && current.generation == expected.generation,
        "owner generation changed; refusing stale operation"
    );
    Ok(current)
}

fn active(registry: &Registry, workspace: &Workspace) -> Result<Option<Session>> {
    Ok(registry
        .session(&workspace.id)?
        .filter(|session| session.status != SessionStatus::Stopped))
}

/// A running environment's routed URL, else its reserved URL on the saved front door.
/// Neither proves that the service is reachable.
pub fn url(registry: &Registry, workspace: &Workspace) -> Result<String> {
    optional_url(registry, workspace)?.context("this environment has no HTTP services or URL")
}

pub fn optional_url(registry: &Registry, workspace: &Workspace) -> Result<Option<String>> {
    match active(registry, workspace)? {
        Some(session) => Ok(session.public_url),
        None if registry.hosts(&workspace.id)?.is_empty() => Ok(None),
        None => Ok(Some(
            FrontDoor::load(registry.state_path())?.url(&workspace.hostname),
        )),
    }
}

pub fn service_url(registry: &Registry, workspace: &Workspace, service: &str) -> Result<String> {
    crate::config::dns_label(service)?;
    let session = active(registry, workspace)?;
    if let Some(session) = &session
        && !session.endpoints.is_empty()
    {
        return session
            .endpoints
            .iter()
            .find(|endpoint| endpoint.service == service)
            .map(|endpoint| endpoint.public_url.clone())
            .with_context(|| {
                let names: Vec<&str> = session
                    .endpoints
                    .iter()
                    .map(|endpoint| endpoint.service.as_str())
                    .collect();
                format!(
                    "unknown service {service}; services with a URL: {}",
                    names.join(", ")
                )
            });
    }
    let hosts = registry.hosts(&workspace.id)?;
    let hostname = hosts.get(service).with_context(|| {
        let names: Vec<&str> = hosts.keys().map(String::as_str).collect();
        format!(
            "unknown service {service}; services with a URL: {}",
            names.join(", ")
        )
    })?;
    match session {
        Some(session) => Ok(session
            .public_url
            .context("this environment has no HTTP services or URL")?
            .replacen(&workspace.hostname, hostname, 1)),
        None => Ok(FrontDoor::load(registry.state_path())?.url(hostname)),
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchSnapshot {
    pub source: std::path::PathBuf,
    pub manifest: crate::config::Manifest,
    pub predecessor: Option<Session>,
}

/// A running `up` session whose owner died while recorded processes survive, by
/// birth identity. `up` takes it back through the recover path instead of starting a duplicate.
pub fn orphaned(registry: &Registry, workspace: &Workspace) -> Result<Option<Session>> {
    let Some(session) = registry.session(&workspace.id)? else {
        return Ok(None);
    };
    if session.kind != "up"
        || !matches!(
            session.status,
            SessionStatus::Starting | SessionStatus::Ready
        )
    {
        return Ok(None);
    }
    let Ok(record) = crate::owner::Record::read(&registry.runtime_path(workspace), &session) else {
        return Ok(None);
    };
    Ok((!record.owner.alive() && !record.quiescent()).then_some(session))
}

pub fn recover_detached(registry: &Registry, workspace: &Workspace) -> Result<Value> {
    let session = registry
        .session(&workspace.id)?
        .context("no session to recover")?;
    let path = registry.runtime_path(workspace);
    let evidence = crate::owner::Record::read(&path, &session)?;
    ensure!(
        !evidence.owner.alive(),
        "owner is still alive; use status/down"
    );
    let _launch = private::Lock::acquire(&path.join("launch.lock"))?;
    let (log_path, log) = owner_log(&path)?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--state-dir")
        .arg(registry.state_path())
        .arg("--workspace")
        .arg(&workspace.id)
        .arg("recover");
    let mut owner = OwnedChild::spawn_logged(&mut command, log)?;
    // The recovery owner enforces the readiness timeout itself.
    loop {
        if let Some(exit) = owner.poll()? {
            let current = current_session(registry, &session)?;
            ensure!(
                exit.success() && current.status == SessionStatus::Stopped,
                "recovery owner exited; inspect {}",
                log_path.display()
            );
            let completed = crate::owner::Record::read(&path, &current)?;
            ensure!(
                completed.phase == crate::owner::Phase::Stopped && completed.quiescent(),
                "recovery stopped receipt is incomplete; inspect {}",
                log_path.display()
            );
            return Ok(
                json!({"session":current,"environment_id":workspace.id,"generation":session.generation,"state":"stopped","recovered":true,"detached":true}),
            );
        }
        let current = current_session(registry, &session)?;
        if let Ok(live) = control::request(&path, &current, Action::Status)
            && live["state"] == "ready"
            && live["owner_pid"].as_u64() == Some(u64::from(owner.id()))
        {
            owner.detach();
            return Ok(json!({"session":current,"recovered":true,"detached":true,"live":live}));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The last `tail` lines of each process of the latest run, or of `service`, as (name, lines).
pub fn logs(
    registry: &Registry,
    workspace: &Workspace,
    service: Option<&str>,
    tail: usize,
) -> Result<Vec<(String, Vec<String>)>> {
    ensure!(
        (1..=10000).contains(&tail),
        "log tail must be between 1 and 10000"
    );
    let directory = crate::logs::directory(&registry.runtime_path(workspace));
    let names = selected(&directory, service)?;
    names
        .into_iter()
        .map(|name| {
            let lines = crate::logs::tail(&directory, &name, tail)?;
            Ok((name, lines))
        })
        .collect()
}

/// Every process with a log, or only `service`, which must be one of them.
fn selected(directory: &std::path::Path, service: Option<&str>) -> Result<Vec<String>> {
    let names = crate::logs::names(directory)?;
    ensure!(!names.is_empty(), "no logs yet; run parcela up first");
    match service {
        Some(service) => {
            ensure!(
                names.iter().any(|name| name == service),
                "unknown service {service}; the latest run has {}",
                names.join(", ")
            );
            Ok(vec![service.to_owned()])
        }
        None => Ok(names),
    }
}

/// Prints the last `tail` lines, then new lines as they come, until the environment stops or a
/// signal arrives. Each line is prefixed with its process's name unless one is named.
pub fn follow_logs(
    registry: &Registry,
    workspace: &Workspace,
    service: Option<&str>,
    tail: u32,
) -> Result<()> {
    use std::io::Write;
    // Before the first line, so Ctrl-C always ends the loop below.
    let signals = Signals::new()?;
    let directory = crate::logs::directory(&registry.runtime_path(workspace));
    let mut names = selected(&directory, service)?;
    let prefixed = service.is_none();
    let paint = crate::paint::Paint::stdout();
    let width = names.iter().map(String::len).max().unwrap_or(0);
    let print = |names: &[String], name: &str, line: &str| {
        let mut out = std::io::stdout().lock();
        let index = names.iter().position(|known| known == name).unwrap_or(0);
        let _ = if prefixed {
            writeln!(out, "{} {line}", paint.log_prefix(index, name, width))
        } else {
            writeln!(out, "{line}")
        };
    };
    // The tail and what follows come from the same reads, so no line falls between them.
    let mut follower = crate::logs::Follower::new(&directory, Some(tail as usize), None);
    let mut running = true;
    while signals.received() == 0 {
        // One more read after the environment stops, for its last lines.
        let last = !running;
        follower.poll(|name, line| {
            if service.is_none_or(|service| service == name) {
                if !names.iter().any(|known| known == name) {
                    names.push(name.to_owned());
                }
                print(&names, name, line);
            }
        })?;
        let _ = std::io::stdout().flush();
        if last {
            break;
        }
        running = active(registry, workspace)?.is_some();
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

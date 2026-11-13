//! Taking over a run whose owner died, or marking it stopped once nothing of it is left.
use super::{
    echo::Echo, health::Health, launch::setup_pending, owner::Owner, variables::for_session,
};
use crate::{
    environment::control::{self, Action},
    environment::lifecycle::current_session,
    environment::record::{Phase, Record},
    environment::supervisor::{self, Supervisor},
    process::Signals,
    project::config::Manifest,
    proxy::router,
    state::private::{self, Lock},
    state::registry::{Registry, Session, SessionStatus, Workspace},
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    thread,
    time::{Duration, Instant},
};

/// The graph of a recorded run, for an owner that takes it over.
fn recorded_graph(
    registry: &Registry,
    record: &Record,
    workspace: &Workspace,
    path: &std::path::Path,
) -> Result<Vec<supervisor::Node>> {
    let env = for_session(
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
        crate::proxy::frontdoor::DEFAULT_HTTPS_PORT
    } else {
        crate::proxy::frontdoor::FrontDoor::load(registry.state_path())?.https
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
    let sockets = crate::state::sockets::environment(&path)?;
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
    let health = Health::start(&record);
    let result = Owner {
        registry,
        health,
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
        .remove_socket(&crate::state::sockets::environment(path)?.control)?;
    registry.session_status(session, SessionStatus::Stopped)?;
    if record.manifest.default_service.is_some() {
        router::resync(registry);
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
    // A retry runs each command as parcela.yaml has it now, so fixing a failing one is enough.
    if let Ok(mut current) = Manifest::load(&workspace.source.join("parcela.yaml"))
        && crate::project::compose::resolve(&mut current, &workspace.source).is_ok()
    {
        for (name, process) in &mut record.manifest.processes {
            if let Some(now) = current.processes.get(name) {
                process.stop.clone_from(&now.stop);
            }
        }
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
        anyhow::bail!(
            "shutdown incomplete: {failure}; fix it if needed, then run parcela down again"
        );
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

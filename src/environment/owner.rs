//! The owner loop of one running generation: readiness, routes, control requests and stop.
use super::{health::Health, launch::SETUP_DONE};
use crate::{
    environment::control::{self, Action, EndpointHealth, LiveState, LiveStatus},
    environment::lifecycle::current_session,
    environment::record::{Phase, Record},
    environment::supervisor::Supervisor,
    http,
    process::{Listeners, Signals},
    project::config::Manifest,
    proxy::router,
    share::Share,
    state::private,
    state::registry::{Endpoint, Registry, Session, SessionStatus, Workspace},
};
use anyhow::{Result, ensure};
use serde_json::json;
use std::{
    collections::BTreeMap,
    thread,
    time::{Duration, Instant},
};

/// How often a ready run re-checks its process tree and listeners.
const READY_CHECK: Duration = Duration::from_secs(2);

/// How long the running loop keeps going while it cannot inspect its processes and listeners.
const INSPECTION_GRACE: Duration = Duration::from_secs(30);

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
        crate::state::registry::Role::Primary => workspace.project_id.clone(),
        crate::state::registry::Role::Preview => {
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
    if !router::resync(registry) {
        thread::sleep(Duration::from_millis(router::LEASE_MS + 250));
    }
}

/// The owner of one running generation: its evidence record, process graph and control socket.
pub(super) struct Owner<'a> {
    pub(super) registry: &'a mut Registry,
    pub(super) workspace: &'a Workspace,
    pub(super) path: &'a std::path::Path,
    pub(super) record: Record,
    pub(super) supervisor: Supervisor,
    pub(super) control: control::Server,
    pub(super) signals: Signals,
    /// Whether this generation runs `hooks.setup`, so readiness marks it done.
    pub(super) setup: bool,
    /// Set once every service answered; an adopted run that was ready starts set.
    pub(super) ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(super) health: Health,
}
impl Owner<'_> {
    /// Supervise until a stop request, a signal, the supervisor's exit or a failure, then stop
    /// the graph and record the outcome.
    pub(super) fn monitor(mut self, timeout: u64) -> Result<i32> {
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
        let mut observed_at: Option<Instant> = None;
        let mut proxy_seen: Option<String> = None;
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
            // `ps` and `lsof` run every tick only while starting; a ready run is checked less often,
            // but at once when a new proxy starts, so its routes are back within a tick.
            let proxy_token = if endpoints.is_empty() {
                None
            } else {
                router::recorded_token(self.registry.state_path())
            };
            let new_proxy = proxy_token.is_some() && proxy_token != proxy_seen;
            proxy_seen = proxy_token;
            if ready && !new_proxy && observed_at.is_some_and(|at| at.elapsed() < READY_CHECK) {
                self.idle(Duration::from_millis(200), requested_stop)?;
                continue;
            }
            observed_at = Some(Instant::now());
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
            // `ps` and `lsof` can miss their deadline on a loaded or throttled machine, so a failed
            // check is retried. One that keeps failing fails a start, but a run that was ready
            // keeps its processes: it only loses its routes until the checks work again.
            let (listeners, observed) = match observation {
                Ok(observation) => {
                    inspection_failing = None;
                    observation
                }
                Err(error) => {
                    let since = *inspection_failing.get_or_insert_with(Instant::now);
                    if since.elapsed() >= INSPECTION_GRACE {
                        if !became_ready {
                            return Err(error);
                        }
                        if ready {
                            ready = false;
                            self.enter(Phase::Starting)?;
                            router::resync(self.registry);
                            crate::paint::Paint::note(&format!(
                                "cannot check this run's processes ({error:#}); its URLs are off until that works again"
                            ));
                        }
                        // The readiness timeout counts from when the checks work again.
                        deadline = Instant::now() + Duration::from_secs(timeout);
                    }
                    self.idle(Duration::from_millis(200), requested_stop)?;
                    continue;
                }
            };
            if ready && observed != listener {
                self.enter(Phase::Starting)?;
                router::resync(self.registry);
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
                // Not just Caddy: the proxy's own process serves page loads and the dashboard,
                // and a Caddy that outlived it kept answering while every page failed.
                let state = self.registry.state_path();
                if router::current(state).is_err()
                    || crate::proxy::owner(state).is_some_and(|(daemon, _)| !daemon.alive())
                {
                    match crate::proxy::revive(self.registry.state_path()) {
                        Ok(true) => {
                            proxy_error = None;
                            crate::paint::Paint::note("the proxy had stopped; started it again")
                        }
                        Ok(false) => {}
                        // Retried every 2s until the port frees up; said once per cause.
                        Err(error) => {
                            let error = format!("{error:#}");
                            if proxy_error.as_ref() != Some(&error) {
                                crate::paint::Paint::note(&format!(
                                    "the proxy is down and did not restart: {error}"
                                ));
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
                    crate::paint::Paint::note(
                        "router URL changed; application stays alive but unrouted until explicit restart",
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
            // A caller that stopped waiting, such as an inventory past its budget, is not news.
            if let Err(error) = pending.reply(&self.record.session, response)
                && !error.chain().any(|cause| {
                    cause.downcast_ref::<std::io::Error>().is_some_and(|error| {
                        matches!(
                            error.kind(),
                            std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                        )
                    }) || cause
                        .downcast_ref::<serde_json::Error>()
                        .is_some_and(serde_json::Error::is_io)
                })
            {
                crate::paint::Paint::note(&format!("owner reply failed: {error:#}"));
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

    /// The run's live state with each process, as a status request gets it.
    fn status(&self) -> Result<serde_json::Value> {
        let mut live = live_status(
            self.registry,
            &self.record.session,
            &self.supervisor.roots(),
            &self.record.manifest,
            &self.record.share,
            &self.health,
        )?;
        live["services"] = self.supervisor.list()["services"].take();
        Ok(live)
    }

    /// One control request while the generation runs.
    fn answer(&mut self, action: &Action, requested_stop: &mut bool) -> Result<serde_json::Value> {
        current_session(self.registry, &self.record.session)?;
        match action {
            Action::Status => self.status(),
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
                    Action::Status => self.status(),
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
            anyhow::bail!(
                "shutdown incomplete: {failure}; fix it if needed, then run parcela down again"
            );
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

fn live_status(
    registry: &Registry,
    session: &Session,
    roots: &[u32],
    manifest: &Manifest,
    share: &Share,
    health: &Health,
) -> Result<serde_json::Value> {
    let current = current_session(registry, session)?;
    let mut endpoints = BTreeMap::new();
    let mut listeners = Vec::new();
    let all = session.endpoints(manifest.default_service.as_deref());
    let snapshot = Listeners::inspect(all.iter().map(|endpoint| endpoint.backend_port))?;
    for endpoint in all {
        let pids = snapshot.owned(endpoint.backend_port, roots)?;
        let http_ready = !pids.is_empty()
            && (health.recently_ready(endpoint.backend_port) || {
                snapshot
                    .address(endpoint.backend_port)
                    .and_then(|address| {
                        http::readiness(
                            address,
                            &endpoint.hostname,
                            &manifest.services[&endpoint.service].readiness_path,
                        )
                    })
                    .unwrap_or(false)
            });
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

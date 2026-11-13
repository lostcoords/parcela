//! Health checks an owner makes while someone keeps asking for its status.
use crate::{environment::record::Record, http, process::Listeners};
use std::{
    collections::BTreeMap,
    thread,
    time::{Duration, Instant},
};

/// While someone keeps asking for this run's status, how often the owner asks each service its
/// health URL, away from the thread that answers; the dashboard asks every two seconds.
const HEALTH_CHECK: Duration = Duration::from_secs(2);
/// An answer older than this is asked again, inline, before a status reply uses it.
const HEALTH_STALE: Duration = Duration::from_secs(3);
/// How long after the last status request the checks go on.
const HEALTH_WANTED: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Checks {
    results: BTreeMap<u16, (bool, Instant)>,
    wanted: Option<Instant>,
}

/// Each endpoint's latest health check, made on its own thread while status requests keep coming.
/// A reply reads it instead of asking a slow app itself, which made callers with a short budget
/// give up on the owner. Nobody asking means no checks, so an idle app's log stays quiet.
pub(super) struct Health {
    checks: std::sync::Arc<std::sync::Mutex<Checks>>,
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Health {
    pub(super) fn start(record: &Record) -> Self {
        let manifest = &record.manifest;
        let targets: Vec<(u16, String, String)> = record
            .session
            .endpoints(manifest.default_service.as_deref())
            .into_iter()
            .filter_map(|endpoint| {
                let path = manifest
                    .services
                    .get(&endpoint.service)?
                    .readiness_path
                    .clone();
                Some((endpoint.backend_port, endpoint.hostname, path))
            })
            .collect();
        let health = Self {
            checks: Default::default(),
            stopped: Default::default(),
        };
        if targets.is_empty() {
            return health;
        }
        let (checks, stopped) = (health.checks.clone(), health.stopped.clone());
        let checker = move || {
            use std::sync::atomic::Ordering::Relaxed;
            while !stopped.load(Relaxed) {
                let wanted = checks.lock().is_ok_and(|checks| {
                    checks.wanted.is_some_and(|at| at.elapsed() < HEALTH_WANTED)
                });
                if wanted
                    && let Ok(listeners) = Listeners::inspect(targets.iter().map(|target| target.0))
                {
                    for (port, host, path) in &targets {
                        let ready = listeners.on(*port).is_ok_and(|held| !held.is_empty())
                            && listeners
                                .address(*port)
                                .and_then(|address| http::readiness(address, host, path))
                                .unwrap_or(false);
                        if let Ok(mut checks) = checks.lock() {
                            checks.results.insert(*port, (ready, Instant::now()));
                        }
                    }
                }
                let next = Instant::now()
                    + if wanted {
                        HEALTH_CHECK
                    } else {
                        Duration::from_millis(200)
                    };
                while !stopped.load(Relaxed) && Instant::now() < next {
                    thread::sleep(Duration::from_millis(50));
                }
            }
        };
        if let Err(error) = thread::Builder::new()
            .name("parcela-health".into())
            .spawn(checker)
        {
            // Replies then ask the apps themselves.
            crate::paint::Paint::note(&format!("health checks run inline: {error}"));
        }
        health
    }

    /// Whether `port` answered its health URL recently, and keeps the checks going for a while.
    /// Only a healthy answer is reused: anything else is asked again by the caller, so a run that
    /// just became ready or unhealthy is never reported late.
    pub(super) fn recently_ready(&self, port: u16) -> bool {
        let Ok(mut checks) = self.checks.lock() else {
            return false;
        };
        checks.wanted = Some(Instant::now());
        checks
            .results
            .get(&port)
            .is_some_and(|(ready, at)| *ready && at.elapsed() < HEALTH_STALE)
    }
}
impl Drop for Health {
    fn drop(&mut self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

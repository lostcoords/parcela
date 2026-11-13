//! `parcela proxy run`: the background process that owns Caddy, the dashboard and the badge
//! injector, and renews route leases every second until a signal arrives.
use super::router::{
    PROTOCOL, RENEW, Router, VERSION, configuration, current, proxy_dir, socket, sync, verify,
};
use crate::{
    http,
    process::{OwnedChild, Signals, exit_code, version},
    proxy::frontdoor::FrontDoor,
    state::private::{self, Lock},
    state::registry::Registry,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    net::TcpListener,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

/// Run Caddy in the foreground for `front_door` until a signal arrives. Saves the profile once
/// Caddy answers, so the profile always names the last proxy that started.
pub fn run(registry: &Registry, binary: &str, front_door: FrontDoor) -> Result<i32> {
    let backend = version(binary, "version")?;
    ensure!(
        backend.starts_with("v2."),
        "unsupported Caddy version: {backend}"
    );
    let state = registry.state_path();
    let directory = proxy_dir(state);
    private::directory(&directory)?;
    // Read-only status probes briefly acquire this same lock before a daemon owns it.
    let _lifetime = Lock::wait(&directory.join("owner.lock"))?;
    let _update = Lock::wait(&directory.join("updates.lock"))?;
    let admin_socket = socket(state)?;
    crate::state::sockets::prepare(&admin_socket)?;
    let mut recovered = None;
    if admin_socket.exists() {
        let evidence: ProxyEvidence = private::read_json(&directory.join("owner.json"))
            .context("existing admin socket has no recovery evidence; no takeover performed")?;
        ensure!(
            evidence.socket.matches(&admin_socket),
            "admin socket was replaced; no takeover performed"
        );
        if evidence.child.alive() {
            verify(state, &evidence.router)?;
            recovered = Some(evidence.child);
        } else {
            ensure!(
                std::os::unix::net::UnixStream::connect(&admin_socket).is_err(),
                "unknown live admin socket preserved"
            );
            evidence.socket.remove_socket(&admin_socket)?;
        }
    }
    if recovered.is_none() {
        probe_listener(front_door.https)?;
    }
    let router = Router {
        token: Uuid::new_v4().to_string(),
        front_door,
        version: VERSION.into(),
        protocol: PROTOCOL,
    };
    let config = configuration(registry, &router)?;
    let _dashboard = crate::proxy::dashboard::Server::start(state, front_door)?;
    let _injector = crate::proxy::inject::Server::start(state)?;
    let config_path = directory.join("caddy.json");
    private::write_json(&config_path, &config)?;
    private::directory(&directory.join("data"))?;
    private::directory(&directory.join("config"))?;
    let mut command = Command::new(binary);
    command
        .args(["run", "--config"])
        .arg(&config_path)
        .current_dir(&directory)
        .env("XDG_DATA_HOME", directory.join("data"))
        .env("XDG_CONFIG_HOME", directory.join("config"));
    let signals = Signals::new()?;
    let log = crate::proxy::log_path(state);
    thread::spawn(move || {
        loop {
            if let Err(error) = crate::proxy::trim_log(&log) {
                eprintln!("parcela: proxy log not trimmed: {error:#}");
            }
            thread::sleep(Duration::from_secs(60));
        }
    });
    let mut child = if recovered.is_none() {
        Some(OwnedChild::spawn(&mut command)?)
    } else {
        None
    };
    if recovered.is_some() {
        let (status, response) = http::admin(
            &admin_socket,
            "POST",
            "/load",
            &serde_json::to_vec(&config)?,
        )?;
        ensure!(
            status == 200,
            "owned Caddy rejected recovery: {}",
            String::from_utf8_lossy(&response)
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.as_mut().map(OwnedChild::poll).transpose()?.flatten() {
            anyhow::bail!("Caddy exited during startup ({})", exit_code(status));
        }
        if signals.received() != 0 {
            return stop_proxy(
                &mut child,
                recovered.as_ref(),
                state,
                signals.received() as i32,
            );
        }
        if verify(state, &router).is_ok() {
            break;
        }
        ensure!(Instant::now() < deadline, "Caddy admin startup timed out");
        thread::sleep(Duration::from_millis(50));
    }
    let identity = if let Some(identity) = &recovered {
        identity.clone()
    } else {
        crate::process::Identity::read(child.as_ref().context("proxy child missing")?.id())?
            .context("proxy birth identity unavailable")?
    };
    // Saved before router.json, which is what makes this proxy read as running.
    front_door.save(state)?;
    private::write_json(
        &directory.join("owner.json"),
        &ProxyEvidence {
            daemon: crate::process::Identity::read(std::process::id())?
                .context("proxy birth identity unavailable")?,
            child: identity.clone(),
            socket: private::Identity::read(&admin_socket)?,
            router: router.clone(),
        },
    )?;
    private::write_json(&directory.join("router.json"), &router)?;
    private::write_json(&directory.join("loaded.json"), &config)?;
    drop(_update);
    crate::paint::Paint::note(&format!(
        "Caddy {backend} serving {} ({}); CA trust unchanged",
        router.url("<name>.localhost"),
        front_door.describe()
    ));
    let mut refresh = Instant::now();
    let result = loop {
        if !identity.alive() {
            break Err(anyhow::anyhow!(
                "owned Caddy exited; applications remain independently supervised"
            ));
        }
        if let Some(status) = child.as_mut().map(OwnedChild::poll).transpose()?.flatten() {
            break Ok(exit_code(status));
        }
        if signals.received() != 0 {
            break stop_proxy(
                &mut child,
                recovered.as_ref(),
                state,
                signals.received() as i32,
            );
        }
        if Instant::now() >= refresh {
            if let Err(error) = sync(registry, &router) {
                crate::paint::Paint::note(&format!(
                    "route lease renewal failed; routes expire: {error:#}"
                ));
            }
            refresh = Instant::now() + RENEW;
        }
        thread::sleep(Duration::from_millis(100));
    };
    // Caddy normally unlinks its socket itself; no delete/recovery of unknown sockets.
    result
}

/// Who owns the running proxy: the `parcela proxy run` process, its Caddy child and admin socket.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProxyEvidence {
    pub daemon: crate::process::Identity,
    pub child: crate::process::Identity,
    pub socket: private::Identity,
    pub router: Router,
}

/// Refuse a busy port, naming who holds it. Parcela never takes a listener over or falls back.
pub fn probe_listener(port: u16) -> Result<()> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => anyhow::bail!(
            "this system only lets root listen on port {port}; Parcela runs without root, so pick a port of 1024 or higher (the default is 1443)"
        ),
        Err(_) => {}
    }
    anyhow::bail!(
        "port {port} is already in use by {}; choose another with `parcela config set proxy.port N`",
        crate::process::port_holder(port)
    )
}

fn stop_proxy(
    child: &mut Option<OwnedChild>,
    peer: Option<&crate::process::Identity>,
    state: &Path,
    signal: i32,
) -> Result<i32> {
    if let Some(child) = child {
        return Ok(exit_code(child.stop(signal)?));
    }
    let peer = peer.context("proxy identity missing")?;
    stop_caddy(state, &current(state)?, peer)?;
    Ok(0)
}
/// Ask the owned Caddy to exit through its private admin socket, then wait until it is gone.
pub(crate) fn stop_caddy(
    state: &Path,
    router: &Router,
    caddy: &crate::process::Identity,
) -> Result<()> {
    ensure!(caddy.alive(), "owned Caddy identity unavailable");
    verify(state, router)?;
    let (status, _) = http::admin(&socket(state)?, "POST", "/stop", &[])?;
    ensure!(status == 200, "owned Caddy rejected stop");
    let deadline = Instant::now() + Duration::from_secs(10);
    while caddy.alive() {
        ensure!(Instant::now() < deadline, "owned Caddy stop incomplete");
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

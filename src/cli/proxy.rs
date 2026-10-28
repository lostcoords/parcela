//! `parcela proxy ...`: the background HTTPS proxy shared by every environment.
use super::{args::ProxyCommand, output::Output, render};
use anyhow::{Context, Result};
use parcela::{
    frontdoor::FrontDoor,
    proxy::{self, Started, Status},
    registry::Registry,
    router::{Router, VERSION},
};
use serde_json::{Value, json};
use std::path::Path;

/// An explicit `--port` is a port profile; otherwise the saved profile or the default.
fn requested(state: &Path, port: Option<u16>) -> Result<FrontDoor> {
    let saved = FrontDoor::load(state)?;
    match port {
        Some(https) if saved.relay && https != saved.https => anyhow::bail!(
            "the port 443 relay sends to :{}; run `parcela proxy uninstall` before moving the proxy",
            saved.https
        ),
        Some(https) => Ok(FrontDoor {
            https,
            relay: saved.relay,
        }),
        None => Ok(saved),
    }
}

/// Save `front_door` and restart a running proxy on it, so new URLs take effect.
fn switch(state: &Path, front_door: FrontDoor, out: Output) -> Result<i32> {
    front_door.save(state)?;
    if matches!(proxy::status(state)?, Status::Running(_)) {
        proxy::stop(state)?;
        proxy::start(state, "caddy", front_door, true)?;
    }
    let report = json!({"front_door": front_door, "url": front_door.url("<name>.localhost")});
    out.emit(&report, |style| {
        format!(
            "{} URLs are now {}\n  Restart running environments to use them: {}\n",
            style.ok(),
            style.url(&front_door.url("<name>.localhost")),
            style.bold("parcela restart")
        )
    })?;
    Ok(0)
}

fn running(state: &Path, router: &Router) -> Value {
    let owner = proxy::owner(state);
    json!({
        "running": true,
        "front_door": router.front_door,
        "listening": router.front_door.describe(),
        "url": router.url("<name>.localhost"),
        "pid": owner.as_ref().map(|(daemon, _)| daemon.pid),
        "caddy_pid": owner.as_ref().map(|(_, caddy)| caddy.pid),
        "version": router.version,
        "version_warning": proxy::protocol_warning(router),
        "log": proxy::log_path(state),
        "owned_admin_verified": true,
        // Whether the system trust store accepts the CA; null before it exists.
        "trusted": proxy::trusted(state),
    })
}

fn stopped(state: &Path) -> Result<Value> {
    let front_door = FrontDoor::load(state)?;
    Ok(json!({
        "running": false,
        "front_door": front_door,
        "listening": front_door.describe(),
        "url": front_door.url("<name>.localhost"),
        "parcela_version": VERSION,
        "log": proxy::log_path(state),
    }))
}

pub fn run(state: &Path, command: ProxyCommand, out: Output) -> Result<i32> {
    match command {
        ProxyCommand::Run { listen } => {
            let registry = Registry::open(state, true)?.context("cannot create the registry")?;
            let front_door = requested(registry.state_path(), listen.port)?;
            parcela::router::run(&registry, &listen.caddy, front_door)
        }
        ProxyCommand::Refresh => {
            let refreshed = proxy::refresh(state)?;
            out.emit(
                &json!({"refreshed": refreshed.is_some()}),
                |_| match refreshed {
                    Some(_) => {
                        "Updated the running proxy. Application processes keep running.\n".into()
                    }
                    None => "No proxy refresh needed.\n".into(),
                },
            )?;
            Ok(0)
        }
        ProxyCommand::Status => {
            let (report, code) = match proxy::status(state)? {
                Status::Running(router) => (running(state, &router), 0),
                Status::Stopped => (stopped(state)?, 1),
                Status::Unhealthy(error) => (
                    {
                        let mut report = stopped(state)?;
                        report["error"] = json!(format!("{error:#}"));
                        report
                    },
                    1,
                ),
            };
            out.emit(&report, |style| render::proxy_status(&report, style))?;
            Ok(code)
        }
        ProxyCommand::Start { listen } => {
            Registry::open(state, true)?.context("cannot create the registry")?;
            let started = proxy::start(
                state,
                &listen.caddy,
                requested(state, listen.port)?,
                listen.port.is_some(),
            )?;
            report_started(state, &started, out)
        }
        ProxyCommand::Restart { listen } => {
            Registry::open(state, true)?.context("cannot create the registry")?;
            // A proxy this Parcela cannot read still holds the saved port until it stops.
            let running = match proxy::status(state)? {
                Status::Running(router) => Some(router.front_door),
                Status::Unhealthy(_) => Some(FrontDoor::load(state)?),
                Status::Stopped => None,
            };
            let front_door = match (listen.port, running) {
                (Some(_), _) => requested(state, listen.port)?,
                (None, Some(running)) => running,
                (None, None) => FrontDoor::load(state)?,
            };
            // Refuse a busy new port before stopping the proxy that works.
            if running.map(|running| running.https) != Some(front_door.https) {
                parcela::router::probe_listener(front_door.https)?;
            }
            proxy::stop(state)?;
            let started = proxy::start(state, &listen.caddy, front_door, true)?;
            report_started(state, &started, out)
        }
        ProxyCommand::Install => {
            Registry::open(state, true)?.context("cannot create the registry")?;
            let front_door = parcela::relay::install(state, FrontDoor::load(state)?)?;
            switch(state, front_door, out)
        }
        ProxyCommand::Uninstall => {
            let front_door = parcela::relay::uninstall(FrontDoor::load(state)?)?;
            switch(state, front_door, out)
        }
        ProxyCommand::Stop => {
            let stopped = proxy::stop(state)?;
            let running = match Registry::open(state, false)? {
                Some(registry) => registry
                    .sessions()?
                    .iter()
                    .filter(|session| session.status != parcela::registry::SessionStatus::Stopped)
                    .count(),
                None => 0,
            };
            if stopped && running > 0 {
                eprintln!(
                    "parcela: {running} running environment(s) lose their URLs until `parcela proxy start` or the next `parcela up`"
                );
            }
            let report = json!({"stopped": stopped, "running": false});
            out.emit(&report, |style| render::proxy_stopped(&report, style))?;
            Ok(0)
        }
    }
}

fn report_started(state: &Path, started: &Started, out: Output) -> Result<i32> {
    let mut report = running(state, started.router());
    report["already_running"] = json!(matches!(started, Started::Already(_)));
    if let Some(warning) = report["version_warning"].as_str() {
        eprintln!("parcela: {warning}");
    }
    out.emit(&report, |style| render::proxy_status(&report, style))?;
    Ok(0)
}

/// `parcela trust`: trusts the CA on request. The only other path is `offer_trust`.
pub fn trust(state: &Path, print: bool, caddy: &str) -> Result<i32> {
    use std::io::IsTerminal;

    let certificate = proxy::certificate(state);
    if print {
        anyhow::ensure!(
            certificate.is_file(),
            "no certificate authority yet at {}; start the proxy with `parcela proxy start`",
            certificate.display()
        );
        println!("{}", certificate.display());
        return Ok(0);
    }
    anyhow::ensure!(
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        "parcela trust requires a terminal; run it directly in a terminal to approve system trust"
    );
    Registry::open(state, true)?.context("cannot create the registry")?;
    proxy::ensure_running(state, "<name>.localhost")?;
    proxy::install_trust(state, caddy)?;
    Ok(0)
}

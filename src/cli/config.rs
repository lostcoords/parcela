//! `parcela config`: this machine's settings in the host configuration file.
use super::args::ConfigCommand;
use super::output::Output;
use anyhow::Result;
use parcela::{
    frontdoor::FrontDoor,
    host::{self, Host},
};
use serde_json::{Map, Value, json};
use std::path::Path;

pub(super) fn run(state: &Path, command: ConfigCommand, out: Output) -> Result<i32> {
    let path = host::path()?;
    let mut host = Host::load(&path)?;
    let (key, done) = match command {
        ConfigCommand::List => {
            let entries = host.entries();
            let value: Map<String, Value> = entries
                .iter()
                .map(|(key, value)| (key.to_string(), value.clone()))
                .collect();
            out.emit(&value, |style| {
                entries
                    .iter()
                    .map(|(key, value)| match text(value) {
                        text if text.is_empty() => {
                            format!("{}\n", style.dim(&format!("{key}=")))
                        }
                        text => format!("{key}={text}\n"),
                    })
                    .collect()
            })?;
            return Ok(0);
        }
        ConfigCommand::Path => {
            let path = path.display().to_string();
            out.emit(&path, |_| path.clone())?;
            return Ok(0);
        }
        ConfigCommand::Get { key } => {
            let value = host.get(&key)?;
            if text(&value).is_empty() {
                return Ok(1);
            }
            out.emit(&value, |_| text(&value))?;
            return Ok(0);
        }
        ConfigCommand::Set { key, value } => {
            host.set(&key, &value)?;
            (key, "Set")
        }
        ConfigCommand::Add { key, value } => {
            host.add(&key, &value)?;
            (key, "Added to")
        }
        ConfigCommand::Remove { key, value } => {
            host.remove(&key, &value)?;
            (key, "Removed from")
        }
        ConfigCommand::Unset { key } => {
            host.unset(&key)?;
            (key, "Unset")
        }
    };
    let previous = Host::load(&path)?;
    if key == "proxy.port" {
        move_proxy(state, &path, &previous, &host)?;
    } else {
        host.save(&path)?;
    }
    let value = host.get(&key)?;
    out.emit(&json!({ "key": key, "value": value }), |style| {
        format!("{} {done} {key}", style.ok())
    })?;
    Ok(0)
}

/// Save a new `proxy.port` and move a running proxy to it, putting the old setting back if the
/// proxy cannot move. The relay forwards to one port, so it has to go first.
fn move_proxy(state: &Path, path: &Path, previous: &Host, host: &Host) -> Result<()> {
    let before = FrontDoor::load(state)?;
    host.save(path)?;
    let moved = (|| -> Result<()> {
        // The proxy saves the port it runs on, so going back to the default means saying so.
        if host.proxy.port.is_none() {
            FrontDoor {
                https: FrontDoor::default_port()?.https,
                relay: before.relay,
            }
            .save(state)?;
        }
        let after = FrontDoor::load(state)?;
        anyhow::ensure!(
            !before.relay || after.https == before.https,
            "the port 443 relay sends to :{}; run `parcela relay uninstall` before moving the proxy",
            before.https
        );
        super::proxy::move_running(state, after)
    })();
    if moved.is_err() {
        previous.save(path)?;
        before.save(state)?;
    }
    moved
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(text).collect::<Vec<_>>().join(","),
        Value::Null => String::new(),
        value => value.to_string(),
    }
}

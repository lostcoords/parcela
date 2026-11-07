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

/// Shown by `list` and `get` but changed by `parcela proxy start --port`.
const PROXY_PORT: &str = "proxy.port";

pub(super) fn run(state: &Path, command: ConfigCommand, out: Output) -> Result<i32> {
    let path = host::path()?;
    let mut host = Host::load(&path)?;
    let proxy_port = || -> Result<Value> { Ok(json!(FrontDoor::load(state)?.https)) };
    match command {
        ConfigCommand::List => {
            let mut entries = host.entries();
            entries.push((PROXY_PORT, proxy_port()?));
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
        }
        ConfigCommand::Get { key } if key == PROXY_PORT => {
            let value = proxy_port()?;
            out.emit(&value, |_| text(&value))?;
        }
        ConfigCommand::Set { key, .. } | ConfigCommand::Unset { key } if key == PROXY_PORT => {
            anyhow::bail!("{PROXY_PORT} is read-only; use `parcela proxy restart --port N`")
        }
        ConfigCommand::Get { key } => {
            let value = host.get(&key)?;
            if text(&value).is_empty() {
                return Ok(1);
            }
            out.emit(&value, |_| text(&value))?;
        }
        ConfigCommand::Set { key, value } => {
            host.set(&key, &value)?;
            host.save(&path)?;
            let value = host.get(&key)?;
            out.emit(&json!({ "key": key, "value": value }), |style| {
                format!("{} Set {key}", style.ok())
            })?;
        }
        ConfigCommand::Add { key, .. } | ConfigCommand::Remove { key, .. } if key == PROXY_PORT => {
            anyhow::bail!("{PROXY_PORT} is not a list; use `parcela proxy restart --port N`")
        }
        ConfigCommand::Add { key, value } => {
            host.add(&key, &value)?;
            host.save(&path)?;
            let value = host.get(&key)?;
            out.emit(&json!({ "key": key, "value": value }), |style| {
                format!("{} Added to {key}", style.ok())
            })?;
        }
        ConfigCommand::Remove { key, value } => {
            host.remove(&key, &value)?;
            host.save(&path)?;
            let value = host.get(&key)?;
            out.emit(&json!({ "key": key, "value": value }), |style| {
                format!("{} Removed from {key}", style.ok())
            })?;
        }
        ConfigCommand::Unset { key } => {
            host.unset(&key)?;
            host.save(&path)?;
            out.emit(&json!({ "key": key, "value": null }), |style| {
                format!("{} Unset {key}", style.ok())
            })?;
        }
        ConfigCommand::Path => {
            let path = path.display().to_string();
            out.emit(&path, |_| path.clone())?;
        }
    }
    Ok(0)
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(text).collect::<Vec<_>>().join(","),
        Value::Null => String::new(),
        value => value.to_string(),
    }
}

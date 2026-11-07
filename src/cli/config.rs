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
                    .map(|(key, value)| match value {
                        Value::Null => format!("{}\n", style.dim(&format!("{key}="))),
                        value => format!("{key}={}\n", text(value)),
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
        ConfigCommand::Get { key } => match host.get(&key)? {
            Value::Null => return Ok(1),
            value => out.emit(&value, |_| text(&value))?,
        },
        ConfigCommand::Set { key, value } => {
            host.set(&key, &value)?;
            host.save(&path)?;
            let value = host.get(&key)?;
            out.emit(&json!({ "key": key, "value": value }), |style| {
                format!("{} Set {key}", style.ok())
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
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string)
}

//! `parcela config`: this machine's settings in the host configuration file.
use super::args::ConfigCommand;
use super::output::Output;
use anyhow::Result;
use parcela::host::{self, Host};
use serde_json::{Map, Value, json};

pub(super) fn run(command: ConfigCommand, out: Output) -> Result<i32> {
    let path = host::path()?;
    let mut host = Host::load(&path)?;
    match command {
        ConfigCommand::List => {
            let entries = host.entries();
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

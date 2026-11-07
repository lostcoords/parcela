//! `parcela autostart`: start environments left running again at login.
use super::args::AutostartCommand;
use super::output::Output;
use anyhow::{Context, Result};
use parcela::{autostart, registry::Registry};
use serde_json::json;
use std::{path::Path, process::Stdio};

pub(super) fn run(state: &Path, command: AutostartCommand, out: Output) -> Result<i32> {
    match command {
        AutostartCommand::Enable => {
            autostart::enable(state)?;
            report(state, out)
        }
        AutostartCommand::Disable => {
            autostart::disable()?;
            report(state, out)
        }
        AutostartCommand::Status => report(state, out),
        AutostartCommand::Run => start(state, out),
    }
}

fn report(state: &Path, out: Output) -> Result<i32> {
    let enabled = autostart::enabled();
    let pending = match Registry::open(state, false)? {
        Some(registry) => autostart::pending(&registry)?.len(),
        None => 0,
    };
    let location = autostart::location()?;
    let value = json!({"enabled": enabled, "location": location, "pending": pending});
    out.emit(&value, |style| {
        if enabled {
            format!("{} Autostart enabled\n", style.ok())
        } else {
            "Autostart disabled\n".into()
        }
    })?;
    Ok(0)
}

/// Start each environment whose run was interrupted, as `parcela up --detach` in its checkout,
/// with the sharing flags its last run had.
fn start(state: &Path, out: Output) -> Result<i32> {
    let pending = match Registry::open(state, false)? {
        Some(registry) => autostart::pending(&registry)?,
        None => Vec::new(),
    };
    let parcela = std::env::current_exe().context("locate parcela")?;
    let mut results = Vec::new();
    let mut failed = false;
    for autostart::Pending { id, flags } in pending {
        let status = std::process::Command::new(&parcela)
            .arg("--state-dir")
            .arg(state)
            .args(["--workspace", &id, "up", "--detach"])
            .args(&flags)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .context("run parcela up")?;
        failed |= !status.success();
        results.push(json!({"workspace": id, "started": status.success()}));
    }
    let value = json!({ "environments": results });
    out.emit(&value, |style| {
        results
            .iter()
            .map(|result| {
                let id = result["workspace"].as_str().unwrap_or_default();
                if result["started"] == true {
                    format!("{} Started {id}\n", style.ok())
                } else {
                    format!("Failed to start {id}\n")
                }
            })
            .collect()
    })?;
    Ok(i32::from(failed))
}

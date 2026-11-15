//! `parcela devices`: the devices that asked to open this machine's LAN shares.
use super::args::DevicesCommand;
use super::output::{Output, cell, table};
use anyhow::Result;
use parcela::proxy::devices::{self, Decision};
use serde_json::json;
use std::path::Path;

pub(super) fn run(state: &Path, command: DevicesCommand, out: Output) -> Result<i32> {
    let (id, decision) = match command {
        DevicesCommand::List => return list(state, out),
        DevicesCommand::Revoke { all: true, .. } => {
            let revoked = devices::revoke_all(state)?;
            out.emit(&json!({"revoked": revoked}), |style| {
                format!("{} Forgot {revoked} device(s)\n", style.ok())
            })?;
            return Ok(0);
        }
        DevicesCommand::Approve { id } => (id, Decision::Approve),
        DevicesCommand::Revoke { id, .. } => (id.unwrap_or_default(), Decision::Revoke),
    };
    let device = devices::decide(state, &id, decision)?;
    let verb = match decision {
        Decision::Approve => "Approved",
        Decision::Revoke => "Forgot",
    };
    out.emit(&device.public(), |style| {
        format!("{} {verb} {} on {}\n", style.ok(), device.id, device.host)
    })?;
    Ok(0)
}

fn list(state: &Path, out: Output) -> Result<i32> {
    let listed: Vec<_> = devices::list(state)?
        .iter()
        .map(devices::Device::public)
        .collect();
    out.emit(&json!({"devices": listed}), |style| {
        if listed.is_empty() {
            return format!(
                "No devices\n{}\n",
                style.dim("A device asks here when it opens a run shared with `parcela up --lan`.")
            );
        }
        let rows: Vec<Vec<String>> = listed
            .iter()
            .map(|device| {
                ["id", "standing", "host", "address", "user_agent"]
                    .iter()
                    .map(|field| cell(&device[field]))
                    .collect()
            })
            .collect();
        table(&["ID", "STATE", "HOST", "ADDRESS", "DEVICE"], &rows)
    })?;
    Ok(0)
}

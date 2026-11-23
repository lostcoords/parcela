//! Commands that act on many registered environments at once.
use super::{
    commands::target_workspace,
    output::{Output, cell, home, status_diagnostics, status_rows, table},
    render,
};
use anyhow::{Context, Result};
use parcela::{
    environment::{data, lifecycle},
    project::config::Manifest,
    state::registry::{Registry, SessionStatus, Workspace},
};
use serde_json::json;
use std::path::Path;

pub fn list(state: &Path, path: &Path, out: Output, overview: bool) -> Result<()> {
    let mut inventory = parcela::environment::inventory::collect(state, Some(path))?;
    if overview {
        inventory.prioritize(path);
    }
    if out == Output::Human {
        status_diagnostics(&inventory.workspaces);
    }
    out.emit(&inventory, |style| {
        let mut rows = inventory.workspaces.clone();
        rows.extend(inventory.unregistered.clone());
        if overview {
            let current_project = rows
                .iter()
                .find(|row| row["current"] == true)
                .map(|row| row["workspace"]["project_id"].clone());
            rows.sort_by_key(|row| {
                (
                    Some(&row["workspace"]["project_id"]) != current_project.as_ref(),
                    row["workspace"]["project_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    row["current"] != true,
                    row["workspace"]["label"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                )
            });
        }
        status_rows(&rows, style)
    })
}

fn running(registry: &Registry, workspace: &Workspace) -> Result<bool> {
    Ok(registry
        .session(&workspace.id)?
        .is_some_and(|session| session.status != SessionStatus::Stopped))
}

/// Stop every running environment, scoped to one project unless `project` is None.
pub fn down_all(state: &Path, project: Option<&str>, out: Output) -> Result<i32> {
    let Some(registry) = Registry::open(state, false)? else {
        return out
            .emit(&json!({"environments": []}), |_| {
                "Nothing is running.\n".into()
            })
            .map(|()| 0);
    };
    let mut targets = Vec::new();
    for workspace in registry.list(project)? {
        if running(&registry, &workspace)? {
            targets.push(workspace);
        }
    }
    if out == Output::Human && !targets.is_empty() {
        let rows: Vec<Vec<String>> = targets
            .iter()
            .map(|workspace| {
                vec![
                    format!("  {}", render::name(&json!(workspace))),
                    lifecycle::url(&registry, workspace).unwrap_or_default(),
                ]
            })
            .collect();
        print!(
            "Stopping {} environment{}:\n{}",
            targets.len(),
            if targets.len() == 1 { "" } else { "s" },
            table(&["  NAME", "URL"], &rows)
        );
    }
    let mut results = Vec::new();
    let mut failed = false;
    for workspace in &targets {
        let result = match lifecycle::down(&registry, workspace) {
            Ok(result) => result,
            Err(error) => {
                failed = true;
                json!({"environment_id": workspace.id, "error": format!("{error:#}")})
            }
        };
        results.push(json!({"workspace": workspace, "result": result}));
    }
    out.emit(&json!({"environments": results}), |style| {
        if results.is_empty() {
            return "Nothing is running.\n".into();
        }
        results
            .iter()
            .map(|row| {
                let name = render::name(&row["workspace"]);
                match row["result"]["error"].as_str() {
                    Some(error) => format!("{} {name}: {error}\n", style.bad()),
                    None => render::down(&row["result"], &name, style),
                }
            })
            .collect()
    })?;
    Ok(if failed { 1 } else { 0 })
}

/// Forget registrations. Never deletes the checkout; deletes the runtime data directory only
/// with `data`, after its registration is gone, so a running environment's is never touched.
pub fn remove(
    state: &Path,
    path: &Path,
    id: Option<uuid::Uuid>,
    prune: bool,
    data: bool,
    out: Output,
) -> Result<i32> {
    let existing = Registry::open(state, false)?
        .context("no environments are registered; nothing to remove")?;
    let (targets, mut skipped) = if prune {
        let mut targets = Vec::new();
        let mut skipped = Vec::new();
        for workspace in existing.list(None)? {
            if workspace.source.is_dir() {
                continue;
            }
            if running(&existing, &workspace)? {
                skipped.push(json!({"workspace": workspace, "reason": "still running; stop it with `parcela down --workspace ID`"}));
            } else {
                targets.push(workspace);
            }
        }
        (targets, skipped)
    } else {
        (vec![target_workspace(&existing, path, id)?], Vec::new())
    };
    let mut released = Vec::new();
    let mut prepared = Vec::new();
    for workspace in targets {
        // The checkout's current file, or the one its last successful run recorded.
        let manifest = if workspace.source.is_dir() {
            Manifest::load(&workspace.source.join("parcela.yaml")).ok()
        } else {
            parcela::state::private::read_json(
                &existing
                    .runtime_path(&workspace)
                    .join("last-successful.json"),
            )
            .ok()
        };
        let cleaned = match manifest {
            Some(manifest) if !running(&existing, &workspace)? => {
                parcela::environment::tasks::teardown(&existing, &manifest, &workspace)
                    .and_then(|()| parcela::project::compose::remove(&manifest, &workspace.label))
            }
            _ => Ok(Vec::new()),
        };
        let volumes = match cleaned {
            Ok(volumes) => volumes,
            Err(error) if !prune => return Err(error),
            Err(error) => {
                skipped.push(json!({"workspace": workspace, "reason": format!("{error:#}")}));
                continue;
            }
        };
        released.push((
            existing.hosts(&workspace.id)?,
            existing.runtime_path(&workspace),
            volumes,
        ));
        prepared.push(workspace);
    }
    let targets = prepared;
    drop(existing);
    let mut registry = Registry::open(state, true)?.context("registry disappeared")?;
    let mut forgotten = Vec::new();
    let mut failed = false;
    for (workspace, (hosts, runtime, volumes)) in targets.into_iter().zip(released) {
        match registry.forget(&workspace.id) {
            Ok(workspace) => {
                let mut kept = json!({"source": workspace.source});
                if !volumes.is_empty() {
                    kept["volumes"] = json!(volumes);
                }
                let mut row = json!({
                    "workspace": workspace,
                    "released_hostnames": hosts.into_values().collect::<Vec<_>>(),
                });
                if !data {
                    kept["runtime_dir"] = json!(runtime);
                } else {
                    match parcela::environment::data::delete(&runtime) {
                        Ok(usage) => {
                            row["deleted"] = json!(usage.map(|usage| json!({
                                "runtime_dir": runtime,
                                "bytes": usage.bytes,
                            })))
                        }
                        Err(error) => {
                            failed = true;
                            kept["runtime_dir"] = json!(runtime);
                            row["error"] = json!(format!("{error:#}"));
                        }
                    }
                }
                row["kept"] = kept;
                forgotten.push(row)
            }
            Err(error) if prune => {
                skipped.push(json!({"workspace": workspace, "reason": format!("{error:#}")}))
            }
            Err(error) => return Err(error),
        }
    }
    let result = json!({"forgotten": forgotten, "skipped": skipped});
    out.emit(&result, |style| {
        let mut text = String::new();
        for row in &forgotten {
            let data = if let Some(error) = row["error"].as_str() {
                format!(
                    "{} could not delete {}: {error}",
                    style.bad(),
                    cell(&row["kept"]["runtime_dir"])
                )
            } else if row["deleted"].is_object() {
                style.dim(&format!(
                    "deleted {} of data in {}",
                    render::size(row["deleted"]["bytes"].as_u64().unwrap_or_default()),
                    cell(&row["deleted"]["runtime_dir"])
                ))
            } else if row["kept"]["runtime_dir"].is_null() {
                style.dim("no data directory to delete")
            } else {
                style.dim(&format!(
                    "data kept in {}; `parcela rm --orphans` lists it",
                    cell(&row["kept"]["runtime_dir"])
                ))
            };
            text.push_str(&format!(
                "{} Forgot {}; released {}\n  {data}\n",
                style.ok(),
                render::name(&row["workspace"]),
                row["released_hostnames"]
                    .as_array()
                    .map(|hosts| hosts.iter().map(cell).collect::<Vec<_>>().join(", "))
                    .unwrap_or_default(),
            ));
            let volumes: Vec<_> = row["kept"]["volumes"]
                .as_array()
                .map(|volumes| volumes.iter().map(cell).collect())
                .unwrap_or_default();
            if !volumes.is_empty() {
                text.push_str(&format!(
                    "  {}\n",
                    style.dim(&format!(
                        "volumes kept; delete with docker volume rm {}",
                        volumes.join(" ")
                    ))
                ));
            }
        }
        for row in &skipped {
            text.push_str(&format!(
                "{} Skipped {}: {}\n",
                style.bad(),
                render::name(&row["workspace"]),
                cell(&row["reason"])
            ));
        }
        if text.is_empty() {
            text.push_str("Nothing to remove; every registered checkout still exists.\n");
        }
        text
    })?;
    Ok(if failed { 1 } else { 0 })
}

/// List runtime directories no registered environment owns; delete them only with `delete`.
pub fn orphans(state: &Path, delete: bool, out: Output) -> Result<i32> {
    let found = data::orphans(state)?;
    let directory = found.directory.display().to_string();
    // As `render::name` names a registered one: the project for a primary checkout.
    let label = |orphan: &data::Orphan| match (&orphan.project, &orphan.role, &orphan.name) {
        (Some(project), Some(role), _) if role == "primary" => project.clone(),
        (Some(project), _, Some(name)) => format!("{name} ({project})"),
        _ => "unknown".into(),
    };
    if !delete {
        let total: u64 = found.orphans.iter().map(|orphan| orphan.usage.bytes).sum();
        let result =
            json!({"environments_dir": directory, "orphans": found.orphans, "bytes": total});
        out.emit(&result, |style| {
            if found.orphans.is_empty() {
                return format!("No orphaned data in {}.\n", home(&directory));
            }
            let rows: Vec<Vec<String>> = found
                .orphans
                .iter()
                .map(|orphan| {
                    vec![
                        format!("  {}", orphan.id),
                        render::size(orphan.usage.bytes),
                        render::ago(orphan.usage.modified),
                        label(orphan),
                        orphan
                            .source
                            .as_ref()
                            .map(|source| home(&source.display().to_string()))
                            .unwrap_or_else(|| "-".into()),
                    ]
                })
                .collect();
            format!(
                "{} data director{} in {} no environment owns, {}:\n{}\nDelete {} with {}\n",
                found.orphans.len(),
                if found.orphans.len() == 1 { "y" } else { "ies" },
                home(&directory),
                render::size(total),
                table(&["  ID", "SIZE", "MODIFIED", "NAME", "CHECKOUT"], &rows),
                if found.orphans.len() == 1 {
                    "it"
                } else {
                    "them"
                },
                style.bold("parcela rm --orphans --data"),
            )
        })?;
        return Ok(0);
    }
    let mut deleted = Vec::new();
    let mut failed = Vec::new();
    for orphan in found.orphans {
        match data::delete_orphan(state, &orphan) {
            Ok(_) => deleted.push(orphan),
            Err(error) => failed.push((orphan, format!("{error:#}"))),
        }
    }
    let total: u64 = deleted.iter().map(|orphan| orphan.usage.bytes).sum();
    let result = json!({
        "environments_dir": directory,
        "deleted": deleted,
        "bytes": total,
        "failed": failed.iter().map(|(orphan, error)| {
            let mut row = json!(orphan);
            row["error"] = json!(error);
            row
        }).collect::<Vec<_>>(),
    });
    out.emit(&result, |style| {
        if deleted.is_empty() && failed.is_empty() {
            return format!("No orphaned data in {}.\n", home(&directory));
        }
        let mut text = String::new();
        for orphan in &deleted {
            text.push_str(&format!(
                "{} Deleted {} ({}), {}\n",
                style.ok(),
                orphan.id,
                label(orphan),
                render::size(orphan.usage.bytes)
            ));
        }
        for (orphan, error) in &failed {
            text.push_str(&format!(
                "{} Kept {} ({}): {error}\n",
                style.bad(),
                orphan.id,
                label(orphan)
            ));
        }
        if !deleted.is_empty() {
            text.push_str(&format!("Freed {}.\n", render::size(total)));
        }
        text
    })?;
    Ok(if failed.is_empty() { 0 } else { 1 })
}

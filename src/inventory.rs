use crate::{
    config::Manifest,
    discovery,
    frontdoor::FrontDoor,
    lifecycle,
    registry::{Registry, Role},
    startup,
};
use anyhow::Result;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(Serialize)]
pub struct Inventory {
    pub runtime_verified: bool,
    pub workspaces: Vec<Value>,
    pub unregistered: Vec<Value>,
}

/// A read-only inventory. Owner probes share a two-second budget; missed replies stay unknown.
pub fn collect(state: &Path, path: Option<&Path>) -> Result<Inventory> {
    let registry = Registry::open(state, false)?;
    let mut rows = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    if let Some(registry) = &registry {
        for workspace in registry.list(None)? {
            let budget = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(400));
            let mut row = lifecycle::inventory_status(registry, &workspace, budget)?;
            let mut urls = serde_json::Map::new();
            for (service, host) in registry.hosts(&workspace.id)? {
                let url = row["live"]["endpoints"][&service]["public_url"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or(FrontDoor::load(state)?.url(&host));
                urls.insert(service, json!(url));
            }
            row["urls"] = json!(urls);
            rows.push(row);
        }
    }
    let mut unregistered = path
        .map(|path| unregistered(state, registry.as_ref(), path))
        .transpose()
        .unwrap_or_else(|error| {
            eprintln!("parcela: cannot list this project's other checkouts: {error:#}");
            None
        })
        .unwrap_or_default();
    if path.is_none()
        && let Some(registry) = &registry
    {
        let mut projects = std::collections::BTreeSet::new();
        for workspace in registry.list(None)? {
            if Manifest::load(&workspace.source.join("parcela.yaml"))
                .is_ok_and(|manifest| manifest.project.id == workspace.project_id)
                && projects.insert(workspace.project_id)
            {
                match self::unregistered(state, Some(registry), &workspace.source) {
                    Ok(found) => unregistered.extend(found),
                    Err(error) => {
                        eprintln!("parcela: dashboard checkout discovery failed: {error:#}")
                    }
                }
            }
        }
    }
    let runtime_verified =
        !rows.is_empty() && rows.iter().all(|row| row["live_health_verified"] == true);
    Ok(Inventory {
        runtime_verified,
        workspaces: rows,
        unregistered,
    })
}

impl Inventory {
    pub fn prioritize(&mut self, path: &Path) {
        let source = crate::config::locate(path).ok();
        let project = source
            .as_ref()
            .and_then(|source| Manifest::load(&source.join("parcela.yaml")).ok())
            .map(|manifest| manifest.project.id);
        for row in self.workspaces.iter_mut().chain(&mut self.unregistered) {
            row["current"] = json!(
                source
                    .as_ref()
                    .is_some_and(|source| row["workspace"]["source"].as_str() == source.to_str())
            );
        }
        let key = |row: &Value| {
            (
                row["workspace"]["project_id"].as_str() != project.as_deref(),
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
        };
        self.workspaces.sort_by_key(key);
        self.unregistered.sort_by_key(key);
    }
}

/// The jj workspaces and Git worktrees of the project at `path` that no environment holds,
/// with the name and URL `parcela up` would give them. Read-only: registers nothing.
fn unregistered(state: &Path, registry: Option<&Registry>, path: &Path) -> Result<Vec<Value>> {
    let Ok(source) = crate::config::locate(path) else {
        return Ok(Vec::new());
    };
    let Ok(current) = Manifest::load(&source.join("parcela.yaml")) else {
        return Ok(Vec::new());
    };
    let registered = match registry {
        Some(registry) => registry.list(None)?,
        None => Vec::new(),
    };
    let front_door = FrontDoor::load(state)?;
    let mut rows = Vec::new();
    for checkout in discovery::discover(&source)? {
        let Ok(source) = checkout.source.canonicalize() else {
            continue;
        };
        if registered
            .iter()
            .any(|workspace| workspace.source == source)
        {
            continue;
        }
        let Ok(manifest) = Manifest::load(&source.join("parcela.yaml")) else {
            continue;
        };
        if manifest.project.id != current.project.id {
            continue;
        }
        let Ok(identity) = startup::Request::default().resolve(registry, &manifest, &source) else {
            continue;
        };
        let hostname = manifest.hostname(identity.role == Role::Primary, &identity.label)?;
        rows.push(json!({
            "workspace": {"project_id": manifest.project.id, "source": source, "role": identity.role, "label": identity.label, "hostname": hostname},
            "url": front_door.url(&hostname),
            "state": "not started",
            "urls": manifest.service_hosts(&hostname)?.into_iter().map(|(name, host)| (name, front_door.url(&host))).collect::<std::collections::BTreeMap<_, _>>(),
        }));
    }
    Ok(rows)
}

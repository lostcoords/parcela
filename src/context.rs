use crate::{
    config::Manifest,
    registry::{Registry, Workspace},
};
use anyhow::Result;
use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Serialize)]
pub struct ResolvedContext {
    pub workspace: Workspace,
    pub source_exists: bool,
    pub runtime_dir: PathBuf,
    pub runtime_created: bool,
    pub state: &'static str,
    /// Names reserved by adoption, not evidence of a live route or listener.
    pub planned_urls: BTreeMap<String, String>,
    /// Named ports of the latest run; the next run reuses each one that is still free.
    pub ports: BTreeMap<String, u16>,
    pub capabilities: Capabilities,
    pub caveats: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub workspace_registry: bool,
    pub routing: bool,
    pub supervision: bool,
    pub tasks: bool,
    pub sharing: bool,
    pub data_isolation_verified: bool,
}
impl ResolvedContext {
    pub fn new(registry: &Registry, manifest: &Manifest, workspace: Workspace) -> Result<Self> {
        let reserved = registry.hosts(&workspace.id)?;
        let front_door = if manifest.services.is_empty() {
            None
        } else {
            Some(crate::frontdoor::FrontDoor::load(registry.state_path())?)
        };
        let planned_urls = manifest
            .service_hosts(&workspace.hostname)?
            .into_iter()
            .filter(|(_, host)| reserved.values().any(|reserved| reserved == host))
            .map(|(name, host)| {
                (
                    name,
                    front_door
                        .as_ref()
                        .expect("HTTP service has a front door")
                        .url(&host),
                )
            })
            .collect();
        let ports = registry
            .session(&workspace.id)?
            .map(|session| session.ports)
            .unwrap_or_default();
        Ok(Self {
            source_exists: workspace.source.is_dir(),
            runtime_dir: registry.runtime_path(&workspace),
            workspace,
            runtime_created: false,
            state: "registered",
            planned_urls,
            ports,
            capabilities: Capabilities {
                workspace_registry: true,
                routing: false,
                supervision: false,
                tasks: false,
                sharing: false,
                data_isolation_verified: false,
            },
            caveats: vec![
                "Inspection does not start services or verify planned URLs; use proxy status and status for separate ownership/intent reports",
                "Data isolation requires project bindings; recorded intent is not live health. Automatic packaging and network sharing remain unsupported",
            ],
        })
    }
}

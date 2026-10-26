use super::{commands::project, output::Output};
use anyhow::{Context, Result, ensure};
use parcela::{
    config::Manifest,
    registry::Registry,
    snapshots::{self, Snapshot},
    startup,
};
use serde_json::json;
use std::path::Path;

pub fn create(
    state: &Path,
    path: &Path,
    name: &str,
    directory: Option<&Path>,
    revision: Option<&str>,
    from: Option<&str>,
    out: Output,
) -> Result<i32> {
    let (source, manifest) = project(path)?;
    manifest.hostname(false, name)?;
    if let Some(registry) = Registry::open(state, false)? {
        ensure!(
            !registry
                .list(Some(&manifest.project.id))?
                .iter()
                .any(|workspace| workspace.label == name),
            "environment {name} is already registered; select it with --workspace or choose a new name"
        );
    }
    let snapshot = from
        .map(|name| Snapshot::open(state, &manifest, name))
        .transpose()?;
    let main =
        parcela::config::main_checkout(&source).context("checkout creation needs Git or jj")?;
    let default = main.with_file_name(format!(
        "{}-{name}",
        main.file_name()
            .context("repository needs a name")?
            .to_string_lossy()
    ));
    let checkout =
        parcela::checkout::create(&source, directory.unwrap_or(&default), name, revision)?;
    let (workspace, registry, created_manifest) = (|| -> Result<_> {
        let created_manifest = Manifest::load(&checkout.source.join("parcela.yaml"))?;
        ensure!(
            created_manifest.project.id == manifest.project.id,
            "new checkout belongs to a different project"
        );
        let mut registry = Registry::open(state, true)?.context("cannot create registry")?;
        let identity = startup::Request {
            name: Some(name.into()),
            primary: false,
        }
        .resolve(Some(&registry), &created_manifest, &checkout.source)?;
        let workspace = registry.adopt(
            &created_manifest,
            &checkout.source,
            identity.role,
            &identity.label,
        )?;
        Ok((workspace, registry, created_manifest))
    })()
    .with_context(|| {
        format!(
            "checkout was retained at {}; registration failed; no services started",
            checkout.root.display()
        )
    })?;
    if let Some(snapshot) = snapshot {
        snapshot
            .restore(&registry, &created_manifest, &workspace)
            .with_context(|| {
                format!(
                    "checkout and environment {} were retained at {}; no services started",
                    workspace.id,
                    checkout.root.display()
                )
            })?;
    }
    out.emit(&json!({"checkout": checkout, "workspace": workspace, "restored_from": from, "state": "stopped"}), |_| {
        format!("Created {name} at {} ({})\nNext step: parcela --workspace {} up --detach\n", checkout.source.display(), checkout.vcs, workspace.id)
    })?;
    Ok(0)
}

pub fn snapshot(
    registry: &Registry,
    manifest: &Manifest,
    workspace: &parcela::registry::Workspace,
    name: &str,
    out: Output,
) -> Result<i32> {
    let info = snapshots::save(registry, manifest, workspace, name)?;
    out.emit(&json!({"snapshot": info}), |_| {
        format!("Saved snapshot {name} at {}\n", info.path.display())
    })?;
    Ok(0)
}

pub fn restore(
    registry: &Registry,
    manifest: &Manifest,
    workspace: &parcela::registry::Workspace,
    name: &str,
    out: Output,
) -> Result<i32> {
    Snapshot::open(registry.state_path(), manifest, name)?
        .restore(registry, manifest, workspace)?;
    out.emit(
        &json!({"restored_from": name, "workspace": workspace, "state": "stopped"}),
        |_| {
            format!(
                "Restored {name} into {}\nNext step: parcela --workspace {} up --detach\n",
                workspace.source.display(),
                workspace.id
            )
        },
    )?;
    Ok(0)
}

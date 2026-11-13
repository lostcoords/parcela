//! The variables every service, hook and task sees, and the runtime directory they name.
use crate::{
    project::config::Manifest,
    share::Share,
    state::private,
    state::registry::{Endpoint, Registry, Session, Workspace},
};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, path::PathBuf, process::Command};

pub fn for_session(
    manifest: &Manifest,
    workspace: &Workspace,
    runtime: &std::path::Path,
    session: &Session,
    share: &Share,
) -> Result<BTreeMap<String, String>> {
    build(
        manifest,
        workspace,
        runtime,
        session.public_url.as_deref().unwrap_or_default(),
        session.backend_port,
        &session.endpoints(manifest.default_service.as_deref()),
        &session.ports,
        share,
    )
}
/// The variables of a one-off command, such as a task or `hooks.teardown`: those of the latest
/// run, or of none yet. A command shares nothing.
pub(super) fn for_command(
    registry: &Registry,
    manifest: &Manifest,
    workspace: &Workspace,
    runtime: &std::path::Path,
) -> Result<BTreeMap<String, String>> {
    if let Some(session) = registry.session(&workspace.id)? {
        return for_session(manifest, workspace, runtime, &session, &Share::default());
    }
    let url = crate::environment::lifecycle::optional_url(registry, workspace)?;
    build(
        manifest,
        workspace,
        runtime,
        url.as_deref().unwrap_or_default(),
        None,
        &[],
        &BTreeMap::new(),
        &Share::default(),
    )
}
/// The variables every service and task sees. Without a run, pass no endpoints or ports.
#[allow(clippy::too_many_arguments)]
pub fn build(
    manifest: &Manifest,
    workspace: &Workspace,
    runtime: &std::path::Path,
    url: &str,
    port: Option<u16>,
    endpoints: &[Endpoint],
    ports: &BTreeMap<String, u16>,
    share: &Share,
) -> Result<BTreeMap<String, String>> {
    let mut context = BTreeMap::from([
        ("PARCELA_PROJECT_ID".into(), workspace.project_id.clone()),
        ("PARCELA_ROLE".into(), workspace.role.as_str().into()),
        ("PARCELA_ENVIRONMENT_ID".into(), workspace.id.clone()),
        (
            "PARCELA_GENERATION".into(),
            workspace.generation.to_string(),
        ),
        (
            "PARCELA_RUNTIME_DIR".into(),
            runtime.to_str().context("non-UTF-8 runtime path")?.into(),
        ),
        (
            "PARCELA_DATA_DIR".into(),
            runtime
                .join("data")
                .to_str()
                .context("non-UTF-8 runtime path")?
                .into(),
        ),
        ("PARCELA_ENVIRONMENT_NAME".into(), workspace.label.clone()),
        (
            "PARCELA_PRIMARY_DIR".into(),
            crate::project::config::main_checkout(&workspace.source)
                .unwrap_or_else(|| workspace.source.clone())
                .to_str()
                .context("non-UTF-8 checkout path")?
                .into(),
        ),
        ("PARCELA_URL".into(), url.into()),
        ("PARCELA_HOSTNAME".into(), workspace.hostname.clone()),
        ("PARCELA_SHARE_HOSTS".into(), share.hosts(endpoints)),
        ("HOST".into(), "127.0.0.1".into()),
        (
            "BROWSER".into(),
            browser(runtime)
                .to_str()
                .context("non-UTF-8 runtime path")?
                .into(),
        ),
    ]);
    if manifest.services.is_empty() {
        context.remove("PARCELA_URL");
        context.remove("PARCELA_HOSTNAME");
    }
    if let Some(port) = port {
        context.insert("PORT".into(), port.to_string());
    }
    for (service, hostname) in manifest.service_hosts(&workspace.hostname)? {
        let prefix = format!(
            "PARCELA_SERVICE_{}",
            service.to_ascii_uppercase().replace('-', "_")
        );
        let endpoint = endpoints
            .iter()
            .find(|endpoint| endpoint.service == service);
        let service_url = endpoint.map_or_else(
            || url.replacen(&workspace.hostname, &hostname, 1),
            |endpoint| endpoint.public_url.clone(),
        );
        context.insert(format!("{prefix}_URL"), service_url);
        context.insert(format!("{prefix}_HOSTNAME"), hostname);
        if let Some(port) = endpoint.map(|endpoint| endpoint.backend_port).or_else(|| {
            (Some(service.as_str()) == manifest.default_service.as_deref())
                .then_some(port)
                .flatten()
        }) {
            context.insert(format!("{prefix}_PORT"), port.to_string());
        }
    }
    for (name, port) in ports {
        context.insert(
            crate::project::config::port_variable(name),
            port.to_string(),
        );
    }
    context.extend(crate::project::compose::environment(
        manifest,
        &workspace.source,
        runtime,
        &workspace.label,
    )?);
    for (key, value) in &manifest.environment {
        let resolved = resolve(key, value, &context)?;
        context.insert(key.clone(), resolved);
    }
    Ok(context)
}

/// Insert `${NAME}` placeholders from `context`; an unknown placeholder is an error.
pub(crate) fn resolve(
    key: &str,
    value: &str,
    context: &BTreeMap<String, String>,
) -> Result<String> {
    let mut resolved = value.to_owned();
    for (name, value) in context {
        resolved = resolved.replace(&format!("${{{name}}}"), value);
    }
    ensure!(
        !resolved.contains("${"),
        "unresolved placeholder in env {key}"
    );
    Ok(resolved)
}
pub(crate) fn command_env(command: &mut Command, context: &BTreeMap<String, String>) {
    // Another environment's variables would point at its ports and directories.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PARCELA_") {
            command.env_remove(key);
        }
    }
    command.env_remove("PORT");
    command.envs(context);
}
pub(super) fn prepare_directory(registry: &Registry, workspace: &Workspace) -> Result<PathBuf> {
    private::directory(&registry.state_path().join("environments"))?;
    let path = registry.runtime_path(workspace);
    private::directory(&path)?;
    for name in ["data", "tmp", "log", "cache", "bin"] {
        private::directory(&path.join(name))?;
    }
    let parcela = std::env::current_exe()?;
    let parcela = parcela.to_str().context("non-UTF-8 Parcela path")?;
    let shim = browser(&path);
    let script = format!(
        "#!/bin/sh\nexec '{}' browse \"$@\"\n",
        parcela.replace('\'', "'\\''")
    );
    // Running processes may call the shim at any moment, so it is replaced only when it changes.
    if std::fs::read(&shim).ok().as_deref() != Some(script.as_bytes()) {
        private::write_executable(&shim, script.as_bytes())?;
    }
    Ok(path)
}
/// The `BROWSER` every process sees: one executable path, as Python's `webbrowser` requires.
fn browser(runtime: &std::path::Path) -> PathBuf {
    runtime.join("bin").join("browser")
}

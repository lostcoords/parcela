//! The variables every service, hook and task sees, and the runtime directory they name.
use crate::{
    project::config::Manifest,
    share::Share,
    state::private,
    state::registry::{Endpoint, Registry, Session, Workspace},
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub fn for_session(
    manifest: &Manifest,
    workspace: &Workspace,
    runtime: &std::path::Path,
    session: &Session,
    share: &Share,
) -> Result<BTreeMap<String, String>> {
    with_tools(
        &workspace.source,
        build(
            manifest,
            workspace,
            runtime,
            session.public_url.as_deref().unwrap_or_default(),
            session.backend_port,
            &session.endpoints(manifest.default_service.as_deref()),
            &session.ports,
            share,
        )?,
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
    with_tools(
        &workspace.source,
        build(
            manifest,
            workspace,
            runtime,
            url.as_deref().unwrap_or_default(),
            None,
            &[],
            &BTreeMap::new(),
            &Share::default(),
        )?,
    )
}

/// `context` over the variables the host's `env.command` prints for `checkout`. A process
/// inherits the environment of whatever started its owner: a shell, the proxy behind the
/// dashboard, or the login service. Its `PATH` can name tool versions the checkout no longer
/// pins, so a version manager asked from the checkout itself decides them instead.
fn with_tools(
    checkout: &Path,
    context: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let host = crate::state::host::Host::current()?;
    let Some(command) = host.env.command.as_deref() else {
        return Ok(context);
    };
    // A checkout that is gone, as for `hooks.teardown` after `rm`, pins nothing.
    if !checkout.is_dir() {
        return Ok(context);
    }
    let mut variables = tools(command, checkout)?;
    variables.extend(context);
    Ok(variables)
}

/// The variables `command` prints as a JSON object when run in `directory`. A `null` value, as
/// `direnv export json` prints for a variable to unset, keeps the inherited one.
pub(crate) fn tools(command: &str, directory: &Path) -> Result<BTreeMap<String, String>> {
    let mut shell = Command::new("/bin/sh");
    shell.arg("-c").arg(command).current_dir(directory);
    command_env(&mut shell, &BTreeMap::new());
    let failed = || format!("env.command `{command}` in {}", directory.display());
    let output =
        crate::process::capture(&mut shell, TOOLS_TIMEOUT, 1 << 20).with_context(failed)?;
    ensure!(
        output.status.success(),
        "{} failed with exit code {}: {}",
        failed(),
        crate::process::exit_code(output.status),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let printed: BTreeMap<String, Option<String>> = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("{} must print a JSON object of strings", failed()))?;
    Ok(printed
        .into_iter()
        .filter_map(|(key, value)| Some((key, value?)))
        .collect())
}
/// A version manager may resolve plugins or read many files, but never prompt: stdin is closed.
const TOOLS_TIMEOUT: Duration = Duration::from_secs(30);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// `tool` prints `version`, from `<root>/<version>/bin`, like one version manager install.
    fn install(root: &Path, version: &str) -> PathBuf {
        let bin = root.join(version).join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let tool = bin.join("tool");
        std::fs::write(&tool, format!("#!/bin/sh\necho {version}\n")).unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn tool(path: &str, context: &BTreeMap<String, String>) -> String {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "tool"]).env("PATH", path);
        command_env(&mut command, context);
        let output = command.output().unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn the_checkout_pins_tool_versions_over_a_stale_inherited_path() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let old = install(&root, "4.0.9");
        install(&root, "4.0.10");
        let checkout = root.join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        std::fs::write(checkout.join(".tool-version"), "4.0.10\n").unwrap();
        // A version manager that, like `mise env --json`, puts the pinned version ahead of PATH.
        let manager = root.join("manager");
        std::fs::write(
            &manager,
            format!(
                "#!/bin/sh\nprintf '{{\"PATH\":\"%s:%s\",\"UNSET\":null}}' \"{}/$(cat .tool-version)/bin\" \"$PATH\"\n",
                root.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&manager, std::fs::Permissions::from_mode(0o755)).unwrap();

        // The proxy was started from a shell that had 4.0.9 active.
        let stale = format!("{}:/usr/bin:/bin", old.display());
        assert_eq!(tool(&stale, &BTreeMap::new()), "4.0.9");

        let command = format!("PATH={stale} {}", manager.display());
        let variables = tools(&command, &checkout).unwrap();
        assert!(!variables.contains_key("UNSET"), "{variables:?}");
        assert_eq!(tool(&stale, &variables), "4.0.10");
    }

    #[test]
    fn a_failing_or_malformed_env_command_names_itself() {
        let directory = tempfile::tempdir().unwrap();
        let error = tools("echo broken >&2; exit 3", directory.path()).unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("env.command `echo broken >&2; exit 3`")
                && message.contains("exit code 3")
                && message.contains("broken"),
            "{message}"
        );
        for printed in ["echo '[]'", "echo '{\"PATH\": 1}'", "echo nope"] {
            let message = format!("{:#}", tools(printed, directory.path()).unwrap_err());
            assert!(message.contains("JSON object of strings"), "{message}");
        }
    }

    #[test]
    fn the_env_command_runs_in_the_checkout_without_another_environments_variables() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let variables = tools(
            "printf '{\"HERE\":\"%s\",\"PORT\":\"%s\"}' \"$PWD\" \"${PORT-unset}\"",
            &root,
        )
        .unwrap();
        assert_eq!(variables["HERE"], root.to_str().unwrap());
        assert_eq!(variables["PORT"], "unset");
    }
}

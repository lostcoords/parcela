//! Commands on one environment that is already registered: status, logs, URLs, recovery and
//! shutdown, tasks and context, and `browse`, which runs inside an environment as `$BROWSER`.
use super::{
    commands::{project, writable},
    output::{Output, status_diagnostics},
    render,
};
use anyhow::{Context, Result, ensure};
use parcela::{
    environment::lifecycle,
    state::context::ResolvedContext,
    state::registry::{NOT_STARTED, Registry, Workspace},
};
use std::path::Path;

pub(super) fn recover(
    state: &Path,
    existing: Registry,
    workspace: &Workspace,
    detach: bool,
    out: Output,
) -> Result<i32> {
    let result = if detach {
        lifecycle::recover_detached(&existing, workspace)?
    } else {
        let session = existing
            .session(&workspace.id)?
            .context("no session to recover")?;
        drop(existing);
        parcela::environment::recovery::recover(&mut writable(state)?, workspace, &session, false)?
    };
    out.emit(&result, |style| render::recovered(&result, style))?;
    Ok(0)
}

pub(super) fn down(registry: &Registry, workspace: &Workspace, out: Output) -> Result<i32> {
    let result = lifecycle::down(registry, workspace)?;
    let name = render::name(&serde_json::to_value(workspace)?);
    out.emit(&result, |style| render::down(&result, &name, style))?;
    Ok(0)
}

pub(super) fn status(registry: &Registry, workspace: &Workspace, out: Output) -> Result<i32> {
    let status = lifecycle::status_with_processes(registry, workspace)?;
    if out == Output::Human {
        status_diagnostics(std::slice::from_ref(&status));
    }
    out.emit(&status, |style| render::status(&status, style))?;
    Ok(0)
}

/// The last `tail` lines of each process, prefixed with its name unless one `service` is asked.
pub(super) fn logs(
    registry: &Registry,
    workspace: &Workspace,
    service: Option<&str>,
    tail: usize,
) -> Result<i32> {
    use std::io::Write;
    let logs = lifecycle::logs(registry, workspace, service, tail)?;
    let mut out = std::io::stdout().lock();
    let paint = parcela::paint::Paint::stdout();
    let width = logs.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    for (index, (name, lines)) in logs.iter().enumerate() {
        for line in lines {
            if service.is_some() {
                writeln!(out, "{line}")?;
            } else {
                writeln!(out, "{} {line}", paint.log_prefix(index, name, width))?;
            }
        }
    }
    Ok(0)
}

fn service_url(
    registry: &Registry,
    workspace: &Workspace,
    service: Option<&str>,
) -> Result<String> {
    match service {
        Some(service) => lifecycle::service_url(registry, workspace, service),
        None => lifecycle::url(registry, workspace),
    }
}

pub(super) fn url(
    registry: &Registry,
    workspace: &Workspace,
    service: Option<&str>,
) -> Result<i32> {
    println!("{}", service_url(registry, workspace, service)?);
    Ok(0)
}

pub(super) fn open(
    registry: &Registry,
    workspace: &Workspace,
    service: Option<&str>,
    out: Output,
) -> Result<i32> {
    let status = lifecycle::status(registry, workspace)?;
    ensure!(
        status["state"] != "stopped",
        "{} is not running; start it with `parcela up`",
        render::name(&status["workspace"])
    );
    let url = service_url(registry, workspace, service)?;
    // $BROWSER may carry arguments, like `firefox -P work`, so the shell splits it.
    let browser = std::env::var("BROWSER")
        .ok()
        .filter(|browser| !browser.is_empty());
    let name = browser.unwrap_or_else(|| {
        if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        }
        .into()
    });
    let opened = std::process::Command::new("/bin/sh")
        .args(["-c", &format!("{name} \"$1\""), "sh", &url])
        .status()
        .with_context(|| format!("run {name}"))?;
    ensure!(opened.success(), "{name} could not open {url}");
    out.emit(&serde_json::json!({ "url": url }), |style| {
        format!("Opened {}\n", style.url(&url))
    })?;
    Ok(0)
}

/// The dashboard's URL, from the running proxy.
pub(super) fn ui(state: &Path, out: Output) -> Result<i32> {
    let router = parcela::proxy::router::current(state)?;
    ensure!(
        router.protocol == parcela::proxy::router::PROTOCOL,
        "the running proxy does not provide this dashboard; run `parcela proxy restart` to upgrade it"
    );
    let url = router.url(parcela::proxy::dashboard::HOST);
    out.emit(&serde_json::json!({"url": url}), |_| format!("{url}\n"))?;
    Ok(0)
}

/// `parcela browse`, as an environment's `$BROWSER`: records what its processes ask to open.
pub(super) fn browse(targets: &[String], out: Output) -> Result<i32> {
    let runtime = std::env::var_os("PARCELA_RUNTIME_DIR")
        .context("parcela browse runs inside a Parcela environment, as its $BROWSER")?;
    let targets = targets
        .iter()
        .map(|target| parcela::proxy::opens::Target::parse(target))
        .collect::<Result<Vec<_>>>()?;
    let opens = targets
        .into_iter()
        .map(|target| parcela::proxy::opens::record(Path::new(&runtime), target))
        .collect::<Result<Vec<_>>>()?;
    out.emit(&serde_json::json!({"opens": opens}), |_| String::new())?;
    Ok(0)
}

pub(super) fn task(state: &Path, path: &Path, name: &str, args: &[String]) -> Result<i32> {
    let (source, manifest) = project(path)?;
    let registry = writable(state)?;
    let workspace = registry.current(&source, &manifest)?;
    parcela::environment::tasks::task(&registry, &manifest, &workspace, name, args)
}

pub(super) fn context(state: &Path, path: &Path, out: Output) -> Result<i32> {
    let (source, manifest) = project(path)?;
    let registry = Registry::open(state, false)?.context(NOT_STARTED)?;
    let workspace = registry.current(&source, &manifest)?;
    let result = serde_json::to_value(ResolvedContext::new(&registry, &manifest, workspace)?)?;
    out.emit(&result, |style| render::context(&result, style))?;
    Ok(0)
}

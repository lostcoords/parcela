//! `parcela task` and `hooks.teardown`: one-off commands with the environment's variables.
use super::variables::{command_env, for_command, prepare_directory};
use crate::{
    process::{OwnedChild, Signals, exit_code},
    project::config::Manifest,
    state::private::{self, Lock},
    state::registry::{Registry, Workspace},
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::os::fd::AsFd;
use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub fn task(
    registry: &Registry,
    manifest: &Manifest,
    workspace: &Workspace,
    name: &str,
    args: &[String],
) -> Result<i32> {
    let task = manifest.tasks.get(name).with_context(|| {
        if manifest.tasks.is_empty() {
            format!("unknown task {name}; parcela.yaml has no tasks")
        } else {
            let names: Vec<&str> = manifest.tasks.keys().map(String::as_str).collect();
            format!("unknown task {name}; parcela.yaml has {}", names.join(", "))
        }
    })?;
    let path = prepare_directory(registry, workspace)?;
    let _lock = Lock::acquire(&path.join("task.lock"))?;
    let env = for_command(registry, manifest, workspace, &path)?;
    // Extra arguments follow the command, like `parcela task test -- spec/models`.
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(format!("{} \"$@\"", task.command))
        .arg(name)
        .args(args)
        .current_dir(&workspace.source);
    command_env(&mut command, &env);
    let signals = Signals::new()?;
    let mut child = OwnedChild::spawn(&mut command)?;
    let deadline = task
        .timeout_seconds
        .map(|seconds| Instant::now() + Duration::from_secs(seconds));
    let (code, outcome) = loop {
        if let Some(status) = child.poll()? {
            break (exit_code(status), "exited");
        }
        if signals.received() != 0 {
            break (
                exit_code(child.stop(signals.received() as i32)?),
                "interrupted",
            );
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            child.stop(libc::SIGTERM)?;
            break (124, "timeout");
        }
        thread::sleep(Duration::from_millis(25));
    };
    private::write_json(
        &path.join("last-task.json"),
        &json!({"task":name,"exit_code":code,"outcome":outcome,"generation":workspace.generation,"environment_id":workspace.id,"automatic_retry":false}),
    )?;
    Ok(code)
}

/// Runs `hooks.teardown` for a stopped environment before `rm` forgets it. Its output goes to
/// stderr, so `--json` stays parseable. Runs from the checkout, or the runtime directory once the
/// checkout is gone.
pub fn teardown(registry: &Registry, manifest: &Manifest, workspace: &Workspace) -> Result<()> {
    let Some(hook) = &manifest.hooks.teardown else {
        return Ok(());
    };
    let path = registry.runtime_path(workspace);
    let env = for_command(registry, manifest, workspace, &path)?;
    let directory = if workspace.source.is_dir() {
        workspace.source.clone()
    } else {
        path.clone()
    };
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(&hook.command)
        .current_dir(directory)
        .stdout(std::process::Stdio::from(
            std::io::stderr().as_fd().try_clone_to_owned()?,
        ));
    command_env(&mut command, &env);
    let status = command.status().context("run hooks.teardown")?;
    ensure!(
        status.success(),
        "hooks.teardown failed with exit code {}; the environment stays registered",
        exit_code(status)
    );
    Ok(())
}

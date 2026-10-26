//! Explicit VCS checkout creation. Discovery remains read-only.
use crate::{config, discovery, process};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Debug, Serialize)]
pub struct CreatedCheckout {
    pub root: PathBuf,
    pub source: PathBuf,
    pub vcs: &'static str,
    pub revision: String,
}

/// Create a new checkout without moving branches, starting services, or copying ignored files.
pub fn create(
    source: &Path,
    destination: &Path,
    name: &str,
    revision: Option<&str>,
) -> Result<CreatedCheckout> {
    config::dns_label(name)?;
    let source = source.canonicalize()?;
    let repository = source
        .ancestors()
        .find(|path| {
            path.join(".jj").symlink_metadata().is_ok()
                || path.join(".git").symlink_metadata().is_ok()
        })
        .context("checkout creation needs a Git repository or jj workspace")?;
    let jj = repository.join(".jj").symlink_metadata().is_ok();
    if jj {
        ensure!(
            repository.join(".jj").is_dir(),
            "invalid jj metadata; no Git fallback performed"
        );
    }
    let parent = destination
        .parent()
        .context("checkout destination needs a parent directory")?
        .canonicalize()?;
    let destination = parent.join(
        destination
            .file_name()
            .context("checkout destination needs a name")?,
    );
    ensure!(
        destination
            .symlink_metadata()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "checkout destination already exists or cannot be inspected: {}",
        destination.display()
    );
    ensure!(
        !destination.starts_with(repository),
        "create the checkout outside the source repository"
    );
    if jj {
        ensure!(
            !discovery::discover(&source)?
                .iter()
                .any(|checkout| checkout.name.as_deref() == Some(name)),
            "jj workspace {name} already exists"
        );
    }
    let mut command = Command::new(if jj { "jj" } else { "git" });
    let pinned;
    if jj {
        command
            .arg("--repository")
            .arg(repository)
            .args([
                "workspace",
                "add",
                "--name",
                name,
                "--revision",
                revision.unwrap_or("@"),
            ])
            .arg(&destination);
    } else {
        pinned = capture(Command::new("git").arg("-C").arg(repository).args([
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{}^{{commit}}", revision.unwrap_or("HEAD")),
        ]))?;
        command
            .arg("-C")
            .arg(repository)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "worktree",
                "add",
                "--detach",
            ])
            .arg(&destination)
            .arg(pinned.trim());
    }
    capture(&mut command).with_context(|| {
        format!(
            "create checkout {}; a partial checkout, if any, was retained",
            destination.display()
        )
    })?;
    let root = destination.canonicalize()?;
    let source = root.join(source.strip_prefix(repository)?);
    let revision = if jj {
        capture(Command::new("jj").arg("--repository").arg(&root).args([
            "--ignore-working-copy",
            "--no-pager",
            "log",
            "--no-graph",
            "-r",
            "@-",
            "--template",
            "commit_id",
        ]))?
    } else {
        capture(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["rev-parse", "HEAD"]),
        )?
    };
    Ok(CreatedCheckout {
        root,
        source,
        vcs: if jj { "jj" } else { "git" },
        revision: revision.trim().into(),
    })
}

fn capture(command: &mut Command) -> Result<String> {
    let output = process::capture(command, Duration::from_secs(60), 512_000)?;
    ensure!(
        output.status.success(),
        "{} failed: {}",
        command.get_program().to_string_lossy(),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

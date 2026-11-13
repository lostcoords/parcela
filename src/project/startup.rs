//! Which identity a checkout registers with: the main checkout's project name, or a preview
//! name from `--name`, the jj workspace or the Git worktree.
use crate::{
    process,
    project::config::Manifest,
    project::discovery,
    state::registry::{Registry, Role},
};
use anyhow::{Context, Result, bail, ensure};
use std::{fs, path::Path, process::Command, time::Duration};

#[derive(Clone, Default)]
pub struct Request {
    pub name: Option<String>,
    pub primary: bool,
}

pub struct Identity {
    pub role: Role,
    pub label: String,
}

impl Request {
    pub fn resolve(
        &self,
        registry: Option<&Registry>,
        manifest: &Manifest,
        source: &Path,
    ) -> Result<Identity> {
        ensure!(
            !(self.primary && self.name.is_some()),
            "--primary conflicts with --name"
        );
        if let Some(name) = &self.name {
            manifest.hostname(false, name)?;
        }
        let source = source.canonicalize()?;
        if let Some(registry) = registry
            && registry
                .list(None)?
                .iter()
                .any(|workspace| workspace.source == source)
        {
            let current = registry.current(&source, manifest)?;
            ensure!(
                self.name
                    .as_ref()
                    .is_none_or(|name| current.role == Role::Preview && current.label == *name)
                    && (!self.primary || current.role == Role::Primary),
                "checkout already uses {} identity {:?}; omit the naming flags to reuse it or select another checkout; no identity was changed",
                current.role.as_str(),
                current.label
            );
            return Ok(Identity {
                role: current.role,
                label: current.label,
            });
        }
        let identity = if let Some(name) = &self.name {
            Identity {
                role: Role::Preview,
                label: name.clone(),
            }
        } else if self.primary {
            main_identity()
        } else {
            let identity = infer(&source).context("cannot infer checkout identity; choose --name NAME for a preview or --primary explicitly")?;
            match identity.role {
                Role::Preview => Identity {
                    label: crate::project::config::fit_label(
                        &identity.label,
                        manifest.label_budget(),
                    ),
                    ..identity
                },
                Role::Primary => identity,
            }
        };
        manifest.hostname(identity.role == Role::Primary, &identity.label)?;
        Ok(identity)
    }
}

fn main_identity() -> Identity {
    Identity {
        role: Role::Primary,
        label: "primary".into(),
    }
}

fn preview_identity(name: &str) -> Result<Identity> {
    let label = crate::project::config::sanitize_label(name)
        .context("checkout name has no DNS-safe characters; choose --name NAME")?;
    Ok(Identity {
        role: Role::Preview,
        label,
    })
}

fn infer(source: &Path) -> Result<Identity> {
    let Some(root) = source
        .ancestors()
        .find(|path| path.join(".jj").is_dir() || path.join(".git").exists())
    else {
        return Ok(main_identity());
    };
    if root.join(".jj").is_dir() {
        let repository = fs::symlink_metadata(root.join(".jj/repo"))?;
        if repository.is_dir() {
            return Ok(main_identity());
        }
        ensure!(
            repository.is_file(),
            "jj repository location is not a regular workspace pointer"
        );
        let checkout = discovery::discover(root)?
            .into_iter()
            .find(|checkout| checkout.source == root)
            .context("current jj workspace is absent from its inventory")?;
        return preview_identity(
            checkout
                .name
                .as_deref()
                .context("jj workspace has no name")?,
        );
    }
    let output = process::capture(
        Command::new("git").arg("-C").arg(root).args([
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ]),
        Duration::from_secs(3),
        16384,
    )?;
    ensure!(output.status.success(), "Git checkout metadata unavailable");
    let text = std::str::from_utf8(&output.stdout)?;
    let locations = text.lines().collect::<Vec<_>>();
    ensure!(locations.len() == 2, "ambiguous Git checkout paths");
    if Path::new(locations[0]).canonicalize()? == Path::new(locations[1]).canonicalize()? {
        return Ok(main_identity());
    }
    let Some(name) = root.file_name().and_then(|name| name.to_str()) else {
        bail!("worktree directory has no UTF-8 name");
    };
    preview_identity(name)
}

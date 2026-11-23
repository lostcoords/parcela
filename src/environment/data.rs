//! Runtime data directories on disk, `<state>/environments/<id>`: how much each holds, deleting
//! one, and finding those no registered environment owns. `rm` leaves them behind unless asked.
use crate::state::{private, registry::Registry};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// Disk space a directory tree takes and its newest modification, in seconds since the epoch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub bytes: u64,
    pub modified: i64,
}

/// Walks without following symlinks. Entries that vanish or cannot be read count as empty.
pub fn usage(path: &Path) -> Usage {
    let mut total = Usage::default();
    let mut pending = vec![path.to_path_buf()];
    while let Some(path) = pending.pop() {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        // Allocated blocks, like du: a sparse SQLite file counts for what it occupies.
        total.bytes += metadata.blocks() * 512;
        total.modified = total.modified.max(metadata.mtime());
        if metadata.is_dir()
            && let Ok(entries) = fs::read_dir(&path)
        {
            pending.extend(entries.flatten().map(|entry| entry.path()));
        }
    }
    total
}

/// Delete one runtime directory, after checking it is the private directory Parcela created and
/// that no owner or task still holds it. Returns what it held, or None when it was already gone.
pub fn delete(path: &Path) -> Result<Option<Usage>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => private::existing_directory(path)?,
    }
    // Held until the tree is gone, so no owner or task starts in it meanwhile.
    let mut locks = Vec::new();
    for name in ["owner.lock", "task.lock"] {
        let lock = path.join(name);
        if fs::symlink_metadata(&lock).is_ok() {
            match private::Lock::try_acquire(&lock)? {
                Some(held) => locks.push(held),
                None => bail!("a process still holds {}; stop it first", lock.display()),
            }
        }
    }
    let usage = usage(path);
    fs::remove_dir_all(path).with_context(|| format!("delete {}", path.display()))?;
    Ok(Some(usage))
}

/// A runtime directory whose environment is no longer registered.
#[derive(Debug, Clone, Serialize)]
pub struct Orphan {
    pub id: String,
    pub path: PathBuf,
    #[serde(flatten)]
    pub usage: Usage,
    /// From the `context.json` of its last run, when one is readable.
    pub project: Option<String>,
    pub role: Option<String>,
    pub name: Option<String>,
    pub source: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Orphans {
    pub directory: PathBuf,
    pub orphans: Vec<Orphan>,
}

/// Direct children of the environments directory that are real directories named by an
/// environment ID no registration owns. Symlinks, files and other names are never listed.
/// Refuses when the registry cannot be read, since then ownership cannot be proved.
pub fn orphans(state: &Path) -> Result<Orphans> {
    let directory = state.join("environments");
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Orphans {
                directory,
                orphans: Vec::new(),
            });
        }
        Err(error) => return Err(error.into()),
        Ok(_) => private::existing_directory(&directory)?,
    }
    let registry = Registry::open(state, false)?.context(
        "the registry is missing, so Parcela cannot tell which data directories are in use; nothing was changed",
    )?;
    let directory = registry.state_path().join("environments");
    private::existing_directory(&directory)?;
    // Listed before the registry is read: `up` registers an ID before it creates the
    // directory, so every directory seen here that is in use is in the registry read below.
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let canonical = uuid::Uuid::parse_str(&id).is_ok_and(|uuid| uuid.to_string() == id);
        if canonical && entry.file_type()?.is_dir() {
            candidates.push(id);
        }
    }
    let owned: BTreeSet<String> = registry
        .list(None)?
        .into_iter()
        .map(|workspace| workspace.id)
        .collect();
    let mut orphans: Vec<Orphan> = candidates
        .into_iter()
        .filter(|id| !owned.contains(id))
        .map(|id| {
            let path = directory.join(&id);
            let context: Option<serde_json::Value> =
                private::read_json(&path.join("context.json")).ok();
            let field = |key: &str| {
                context
                    .as_ref()
                    .and_then(|context| context["environment"][key].as_str().map(str::to_owned))
            };
            Orphan {
                usage: usage(&path),
                project: field("PARCELA_PROJECT_ID"),
                role: field("PARCELA_ROLE"),
                name: field("PARCELA_ENVIRONMENT_NAME"),
                source: context
                    .as_ref()
                    .and_then(|context| context["source"].as_str().map(PathBuf::from)),
                id,
                path,
            }
        })
        .collect();
    orphans.sort_by(|a, b| b.usage.bytes.cmp(&a.usage.bytes).then(a.id.cmp(&b.id)));
    Ok(Orphans { directory, orphans })
}

/// Delete an orphan found by `orphans`, refusing if a registration has claimed its ID since.
pub fn delete_orphan(state: &Path, orphan: &Orphan) -> Result<Option<Usage>> {
    let registry = Registry::open(state, false)?.context("the registry disappeared")?;
    ensure!(
        registry
            .list(None)?
            .iter()
            .all(|workspace| workspace.id != orphan.id),
        "{} is registered again; nothing was deleted",
        orphan.id
    );
    delete(&orphan.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{project::config::Manifest, state::registry::Role};
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn private_dir(path: &Path) {
        private::directory(path).unwrap();
    }

    /// A state directory with one registered environment and its runtime directory.
    fn state() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::TempDir::new().unwrap();
        let root = t.path().canonicalize().unwrap();
        let state = root.join("state");
        let checkout = root.join("app");
        fs::create_dir(&checkout).unwrap();
        fs::write(
            checkout.join("parcela.yaml"),
            "version: 1\nproject: app\nservices: {web: {command: bin/web, http: {healthcheck: /}}}\n",
        )
        .unwrap();
        let manifest = Manifest::load(&checkout.join("parcela.yaml")).unwrap();
        let mut registry = Registry::open(&state, true).unwrap().unwrap();
        let workspace = registry
            .adopt(&manifest, &checkout, Role::Primary, "primary")
            .unwrap();
        let owned = registry.runtime_path(&workspace);
        private_dir(&owned.join("data"));
        (t, state, owned)
    }

    #[test]
    fn orphans_are_only_unregistered_real_directories_named_by_an_id() {
        let (_t, state, owned) = state();
        let environments = state.join("environments");
        let orphan = environments.join(uuid::Uuid::new_v4().to_string());
        private_dir(&orphan.join("data"));
        fs::write(orphan.join("data/db.sqlite3"), vec![1u8; 64 * 1024]).unwrap();
        private::write_json(
            &orphan.join("context.json"),
            &serde_json::json!({"source": "/gone/app-review", "environment": {"PARCELA_PROJECT_ID": "app", "PARCELA_ROLE": "preview", "PARCELA_ENVIRONMENT_NAME": "review"}}),
        )
        .unwrap();
        // Never listed: a symlink to a directory, a file, and a directory not named by an ID.
        let outside = state.parent().unwrap().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(
            &outside,
            environments.join(uuid::Uuid::new_v4().to_string()),
        )
        .unwrap();
        fs::write(environments.join(uuid::Uuid::new_v4().to_string()), "").unwrap();
        private_dir(&environments.join("notes"));
        private_dir(&environments.join(uuid::Uuid::new_v4().to_string().to_uppercase()));

        let found = orphans(&state).unwrap();
        assert_eq!(found.orphans.len(), 1, "{:?}", found.orphans);
        let found = &found.orphans[0];
        assert_eq!(found.path, orphan);
        assert!(found.usage.bytes >= 64 * 1024, "{:?}", found.usage);
        assert!(found.usage.modified > 0);
        assert_eq!(found.project.as_deref(), Some("app"));
        assert_eq!(found.name.as_deref(), Some("review"));
        assert_eq!(found.source, Some(PathBuf::from("/gone/app-review")));

        assert!(delete_orphan(&state, found).unwrap().is_some());
        assert!(!orphan.exists());
        assert!(owned.is_dir() && outside.is_dir());
        assert!(orphans(&state).unwrap().orphans.is_empty());
    }

    #[test]
    fn orphans_refuse_without_a_registry() {
        let (_t, state, owned) = state();
        fs::remove_file(state.join("registry.sqlite3")).unwrap();
        let error = orphans(&state).unwrap_err();
        assert!(
            format!("{error:#}").contains("registry is missing"),
            "{error:#}"
        );
        assert!(owned.is_dir());
    }

    #[test]
    fn delete_refuses_a_symlink_a_shared_directory_and_a_held_lock() {
        let (_t, state, owned) = state();
        let outside = state.parent().unwrap().join("outside");
        private_dir(&outside);
        let link = state.join("environments/link");
        symlink(&outside, &link).unwrap();
        assert!(delete(&link).is_err());
        assert!(outside.is_dir());

        fs::set_permissions(&owned, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(delete(&owned).is_err());
        fs::set_permissions(&owned, fs::Permissions::from_mode(0o700)).unwrap();

        let held = private::Lock::acquire(&owned.join("owner.lock")).unwrap();
        let error = delete(&owned).unwrap_err();
        assert!(format!("{error:#}").contains("still holds"), "{error:#}");
        drop(held);
        assert!(delete(&owned).unwrap().is_some());
        assert!(!owned.exists());
        assert_eq!(delete(&owned).unwrap(), None);
    }
}

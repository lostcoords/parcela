//! Allowlisted data snapshots, separate from source, configuration, secrets and process state.
use crate::{
    config::{self, Data, Manifest},
    private::{self, Lock},
    registry::{Registry, SessionStatus, Workspace},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    format: u32,
    project: String,
    environment: String,
    source: PathBuf,
    data: Data,
    entries: Vec<Entry>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Entry {
    Directory {
        path: String,
    },
    File {
        path: String,
        sha256: String,
        size: u64,
    },
    Sqlite {
        path: String,
        sha256: String,
        size: u64,
    },
}
impl Entry {
    fn path(&self) -> &str {
        match self {
            Self::Directory { path } | Self::File { path, .. } | Self::Sqlite { path, .. } => path,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Info {
    pub name: String,
    pub path: PathBuf,
    pub entries: usize,
}

pub struct Snapshot {
    path: PathBuf,
    metadata: Metadata,
}
impl Snapshot {
    pub fn open(state: &Path, manifest: &Manifest, name: &str) -> Result<Self> {
        config::dns_label(name)?;
        let path = parent(state, &manifest.project.id, false)?.join(name);
        private::existing_directory(&path)?;
        let metadata: Metadata = private::read_json(&path.join("manifest.json"))?;
        ensure!(
            metadata.format == 1 && metadata.project == manifest.project.id,
            "snapshot format or project does not match this environment"
        );
        ensure!(
            metadata.data == manifest.data,
            "snapshot data policy differs from parcela.yaml; no data was restored"
        );
        metadata.data.validate()?;
        let mut seen = BTreeSet::new();
        for entry in &metadata.entries {
            let relative = config::data_path(entry.path())?;
            ensure!(seen.insert(entry.path()), "duplicate snapshot path");
            let allowed = match entry {
                Entry::Sqlite { .. } => metadata
                    .data
                    .sqlite
                    .iter()
                    .any(|p| Path::new(p) == relative),
                _ => metadata.data.files.iter().any(|p| relative.starts_with(p)),
            };
            ensure!(
                allowed,
                "snapshot contains an undeclared data path: {}",
                entry.path()
            );
        }
        ensure!(!metadata.entries.is_empty(), "snapshot contains no data");
        for path in metadata.data.files.iter().chain(&metadata.data.sqlite) {
            ensure!(
                seen.contains(path.as_str()),
                "snapshot is missing declared path: {path}"
            );
        }
        private::existing_directory(&path.join("data"))?;
        Ok(Self { path, metadata })
    }

    pub fn restore(
        &self,
        registry: &Registry,
        manifest: &Manifest,
        workspace: &Workspace,
    ) -> Result<()> {
        ensure!(
            self.metadata.project == manifest.project.id && self.metadata.data == manifest.data,
            "snapshot project or data policy differs from the new checkout"
        );
        let _locks = stopped(registry, workspace)?;
        let runtime = registry.runtime_path(workspace);
        let destination = runtime.join("data");
        absent(&destination)
            .context("restore only fills a fresh environment; existing data is never replaced")?;
        let staging = tempfile::Builder::new()
            .prefix(".restore-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(&runtime)?;
        for entry in &self.metadata.entries {
            let relative = config::data_path(entry.path())?;
            let source = checked(&self.path.join("data"), relative)?;
            let target = staging.path().join(relative);
            match entry {
                Entry::Directory { .. } => {
                    ensure!(
                        source.is_dir(),
                        "snapshot directory was replaced: {}",
                        entry.path()
                    );
                    private::directory(&target)?;
                }
                Entry::File { sha256, size, .. } | Entry::Sqlite { sha256, size, .. } => {
                    let (actual, length) =
                        copy_file(&source, &target, matches!(entry, Entry::Sqlite { .. }))?;
                    ensure!(
                        actual == *sha256 && length == *size,
                        "snapshot checksum mismatch: {}; no data restored",
                        entry.path()
                    );
                }
            }
        }
        publish(staging.path(), &destination)?;
        Ok(())
    }
}

pub fn save(
    registry: &Registry,
    manifest: &Manifest,
    workspace: &Workspace,
    name: &str,
) -> Result<Info> {
    config::dns_label(name)?;
    manifest.data.validate()?;
    ensure!(
        !manifest.data.files.is_empty() || !manifest.data.sqlite.is_empty(),
        "declare data.files or data.sqlite in parcela.yaml; snapshots include nothing by default"
    );
    let _locks = stopped(registry, workspace)?;
    let source = registry.runtime_path(workspace).join("data");
    private::existing_directory(&source)?;
    let parent = parent(registry.state_path(), &manifest.project.id, true)?;
    let destination = parent.join(name);
    absent(&destination).context("snapshot names are immutable; choose a new name")?;
    let staging = tempfile::Builder::new()
        .prefix(".snapshot-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&parent)?;
    let data = staging.path().join("data");
    private::directory(&data)?;
    let mut entries = Vec::new();
    for value in &manifest.data.files {
        collect(&source, config::data_path(value)?, &data, &mut entries)?;
    }
    for value in &manifest.data.sqlite {
        let relative = config::data_path(value)?;
        let input = checked(&source, relative)?;
        regular(&input)?;
        let target = data.join(relative);
        private::directory(target.parent().context("database needs a parent")?)?;
        backup(&input, &target)?;
        let (sha256, size) = hash(&target)?;
        entries.push(Entry::Sqlite {
            path: value.clone(),
            sha256,
            size,
        });
    }
    let count = entries.len();
    let metadata = Metadata {
        format: 1,
        project: manifest.project.id.clone(),
        environment: workspace.id.clone(),
        source: workspace.source.clone(),
        data: manifest.data.clone(),
        entries,
    };
    let bytes = serde_json::to_vec_pretty(&metadata)?;
    ensure!(
        bytes.len() < 4_000_000,
        "snapshot manifest exceeds the supported 4 MB limit; reduce declared paths"
    );
    private::write_bytes(&staging.path().join("manifest.json"), &bytes)?;
    publish(staging.path(), &destination)?;
    Ok(Info {
        name: name.into(),
        path: destination,
        entries: count,
    })
}

fn parent(state: &Path, project: &str, create: bool) -> Result<PathBuf> {
    config::dns_label(project)?;
    let mut path = state.to_path_buf();
    private::existing_directory(&path)?;
    for part in ["snapshots", project] {
        path.push(part);
        if create {
            private::directory(&path)?;
        } else {
            private::existing_directory(&path)?;
        }
    }
    Ok(path)
}
fn stopped(registry: &Registry, workspace: &Workspace) -> Result<[Lock; 3]> {
    let runtime = registry.runtime_path(workspace);
    private::directory(&runtime)?;
    let locks = [
        Lock::acquire(&runtime.join("restart.lock"))?,
        Lock::acquire(&runtime.join("owner.lock"))?,
        Lock::acquire(&runtime.join("task.lock"))?,
    ];
    ensure!(
        registry
            .session(&workspace.id)?
            .is_none_or(|s| s.status == SessionStatus::Stopped),
        "stop this environment with `parcela down` before snapshot or restore"
    );
    Ok(locks)
}
fn absent(path: &Path) -> Result<()> {
    match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
        Ok(_) => anyhow::bail!("{} already exists; nothing was overwritten", path.display()),
    }
}
fn checked(root: &Path, relative: &Path) -> Result<PathBuf> {
    private::existing_directory(root)?;
    let mut path = root.to_path_buf();
    for part in relative.components() {
        path.push(part);
        ensure!(
            !path.symlink_metadata()?.file_type().is_symlink(),
            "data symlink is not allowed: {}",
            path.display()
        );
    }
    Ok(path)
}
fn regular(path: &Path) -> Result<()> {
    let meta = path.symlink_metadata()?;
    // SAFETY: getuid has no pointers or failure case.
    ensure!(
        meta.is_file() && meta.nlink() == 1 && meta.uid() == unsafe { libc::getuid() },
        "data must be a user-owned regular file, not a symlink, hardlink or special file: {}",
        path.display()
    );
    Ok(())
}
fn collect(
    root: &Path,
    relative: &Path,
    destination: &Path,
    entries: &mut Vec<Entry>,
) -> Result<()> {
    config::data_path(relative.to_str().context("data path must be UTF-8")?)?;
    let source = checked(root, relative)?;
    let target = destination.join(relative);
    let value = relative
        .to_str()
        .context("data path must be UTF-8")?
        .to_owned();
    if source.is_dir() {
        private::directory(&target)?;
        entries.push(Entry::Directory { path: value });
        let mut children = fs::read_dir(&source)?.collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            collect(
                root,
                &relative.join(child.file_name()),
                destination,
                entries,
            )?;
        }
    } else {
        let (sha256, size) = copy_file(&source, &target, false)?;
        entries.push(Entry::File {
            path: value,
            sha256,
            size,
        });
    }
    Ok(())
}
fn reader(path: &Path) -> Result<File> {
    regular(path)?;
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?)
}
fn copy_file(source: &Path, destination: &Path, sqlite: bool) -> Result<(String, u64)> {
    let mut input = reader(source)?;
    private::directory(destination.parent().context("data file needs a parent")?)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)?;
    let mut digest = Sha256::new();
    let mut size = 0;
    let mut buffer = [0_u8; 65536];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        ensure!(
            sqlite || size != 0 || !buffer[..count].starts_with(b"SQLite format 3\0"),
            "SQLite data must be declared in data.sqlite, never copied as a raw file"
        );
        digest.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
        size += count as u64;
    }
    output.sync_all()?;
    Ok((format!("{:x}", digest.finalize()), size))
}
fn hash(path: &Path) -> Result<(String, u64)> {
    let mut input = reader(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 65536];
    let mut size = 0;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        size += count as u64;
    }
    Ok((format!("{:x}", digest.finalize()), size))
}
fn backup(source: &Path, destination: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = source.as_os_str().to_os_string();
        sidecar.push(suffix);
        let path = PathBuf::from(sidecar);
        match path.symlink_metadata() {
            Ok(_) => regular(&path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let source = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)?;
    let mut target = Connection::open(destination)?;
    {
        let backup = Backup::new(&source, &mut target)?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match backup.step(256)? {
                StepResult::Done => break,
                StepResult::More | StepResult::Busy | StepResult::Locked => {
                    ensure!(
                        Instant::now() < deadline,
                        "SQLite snapshot timed out; no snapshot published"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => anyhow::bail!("unsupported SQLite backup result"),
            }
        }
    }
    let integrity: String = target.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    ensure!(
        integrity == "ok",
        "SQLite backup failed integrity check: {integrity}"
    );
    target.close().map_err(|(_, error)| error)?;
    File::open(destination)?.sync_all()?;
    Ok(())
}

fn publish(source: &Path, destination: &Path) -> Result<()> {
    let from = std::ffi::CString::new(source.as_os_str().as_bytes())?;
    let to = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
    // SAFETY: both paths are valid NUL-terminated strings; the syscall never replaces a destination.
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } as i64;
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } as i64;
    ensure!(
        result == 0,
        "publish {} without replacement: {}",
        destination.display(),
        std::io::Error::last_os_error()
    );
    File::open(destination.parent().context("destination needs a parent")?)?.sync_all()?;
    Ok(())
}

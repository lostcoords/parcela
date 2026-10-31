use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::Path,
};
use uuid::Uuid;

pub fn directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?,
        Err(e) => return Err(e.into()),
    }
    existing_directory(path)
}
pub fn existing_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    // SAFETY: getuid has no pointers or failure case.
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::getuid() }
            && metadata.mode() & 0o077 == 0,
        "{} must be a real, user-owned private directory (700)",
        path.display()
    );
    Ok(())
}
fn file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    // SAFETY: getuid has no pointers or failure case.
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::getuid() }
            && metadata.mode() & 0o077 == 0,
        "{} must be a user-owned private regular file (600)",
        path.display()
    );
    Ok(file)
}
/// The kernel releases this lock on exit/crash. Never signal a persisted PID.
pub struct Lock {
    _file: File,
}
impl Lock {
    pub fn identity(&self) -> Result<Identity> {
        Ok(Identity::from_metadata(&self._file.metadata()?))
    }
    pub fn acquire(path: &Path) -> Result<Self> {
        Self::try_acquire(path)?.with_context(|| {
            format!(
                "operation already owned by another process: {}",
                path.display()
            )
        })
    }
    pub fn try_acquire(path: &Path) -> Result<Option<Self>> {
        let file = file(path)?;
        // SAFETY: fd is held by file; flock does not retain a pointer.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Some(Self { _file: file }));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(None);
        }
        Err(error.into())
    }
    pub fn wait(path: &Path) -> Result<Self> {
        Self::wait_for(path, std::time::Duration::from_secs(5))
    }
    pub fn wait_for(path: &Path, timeout: std::time::Duration) -> Result<Self> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match Self::acquire(path) {
                Ok(lock) => return Ok(lock),
                Err(error) if std::time::Instant::now() >= deadline => return Err(error),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(25)),
            }
        }
    }
}
pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    write_bytes(path, &serde_json::to_vec_pretty(value)?)
}
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    write_mode(path, bytes, 0o600)
}
/// A private script, executable from the moment it appears at `path`.
pub fn write_executable(path: &Path, bytes: &[u8]) -> Result<()> {
    write_mode(path, bytes, 0o700)
}
fn write_mode(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().context("file needs a parent directory")?;
    directory(parent)?;
    if path.exists() {
        drop(file(path)?);
    }
    let temporary = parent.join(format!(".{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    // SAFETY: getuid has no pointers or failure case.
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::getuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() < 4_000_000,
        "invalid private metadata: {}",
        path.display()
    );
    use std::io::Read;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    device: u64,
    inode: u64,
}
impl Identity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
    pub fn read(path: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "identity cannot refer to a symlink"
        );
        Ok(Self::from_metadata(&metadata))
    }
    /// Device numbers can change across boots; the caller must prove the old boot ended.
    pub(crate) fn same_inode(&self, other: &Self) -> bool {
        self.inode == other.inode
    }
    pub fn matches(&self, path: &Path) -> bool {
        Self::read(path).is_ok_and(|actual| actual == *self)
    }
    pub fn locked(&self, path: &Path) -> Result<bool> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.mode() & 0o077 == 0,
            "invalid ownership lock"
        );
        ensure!(
            *self == Self::from_metadata(&metadata),
            "ownership lock was replaced"
        );
        // SAFETY: the file owns the descriptor throughout the nonblocking lock probe.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) };
        if result == 0 {
            return Ok(false);
        }
        let error = std::io::Error::last_os_error();
        ensure!(
            error.kind() == std::io::ErrorKind::WouldBlock,
            "cannot inspect ownership lock: {error}"
        );
        Ok(true)
    }
    pub fn remove_socket(&self, path: &Path) -> Result<()> {
        use std::os::unix::fs::FileTypeExt;
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            metadata.file_type().is_socket() && *self == Self::from_metadata(&metadata),
            "socket was replaced; preserving {}",
            path.display()
        );
        fs::remove_file(path)?;
        Ok(())
    }
}

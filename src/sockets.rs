//! Unix socket paths are capped near 104 bytes, so sockets live in a short per-user directory,
//! `<base>/<hash>`, keyed by the canonical state directory instead of under the state directory itself.
use crate::private;
use anyhow::{Context, Result, ensure};
use std::{
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

pub struct Environment {
    pub control: PathBuf,
}

pub fn dashboard(state: &Path) -> Result<PathBuf> {
    Ok(directory(state)?.join("dashboard.sock"))
}

pub fn proxy_admin(state: &Path) -> Result<PathBuf> {
    Ok(directory(state)?.join("proxy-admin.sock"))
}

/// `runtime` is the registry's `<state>/environments/<environment-id>` directory.
pub fn environment(runtime: &Path) -> Result<Environment> {
    let id = runtime
        .file_name()
        .and_then(|name| name.to_str())
        .context("environment runtime path has no identifier")?;
    let prefix: String = id.chars().filter(|c| *c != '-').take(12).collect();
    ensure!(
        !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_alphanumeric()),
        "environment identifier is not a socket-safe name: {id}"
    );
    let state = runtime
        .parent()
        .and_then(Path::parent)
        .context("environment runtime path has no state directory")?;
    let directory = directory(state)?;
    Ok(Environment {
        control: directory.join(format!("{prefix}-ctl.sock")),
    })
}

/// Creates the socket's directory and its parent as private (700), refusing symlinks and directories
/// another user owns, before a listener is bound there.
pub fn prepare(socket: &Path) -> Result<()> {
    let directory = socket.parent().context("socket path has no directory")?;
    private::directory(
        directory
            .parent()
            .context("socket directory has no parent")?,
    )?;
    private::directory(directory)
}

/// Refuses a socket whose directory or its parent is not a private, user-owned directory.
pub fn verify(socket: &Path) -> Result<()> {
    let directory = socket.parent().context("socket path has no directory")?;
    private::existing_directory(
        directory
            .parent()
            .context("socket directory has no parent")?,
    )?;
    private::existing_directory(directory)
}

fn directory(state: &Path) -> Result<PathBuf> {
    let state = state
        .canonicalize()
        .with_context(|| format!("resolve state directory {}", state.display()))?;
    let hash = format!("{:016x}", fnv1a(state.as_os_str().as_bytes()));
    let hash = &hash[..8];
    Ok(base().join(hash))
}

/// macOS's per-user temporary directory, which no other user can create first, or
/// `/tmp/parcela-<uid>` where there is none, as on Linux. Neither depends on the environment, so
/// every Parcela process of a user agrees on it.
pub fn base() -> PathBuf {
    user_directory()
        .filter(|path| {
            // The longest socket adds /parcela/<hash>/<12 characters>-ctl.sock.
            path.is_absolute()
                && path.as_os_str().len() + "/parcela/12345678/123456789012-ctl.sock".len() < 100
                && private::existing_directory(path).is_ok()
        })
        .map_or_else(
            // SAFETY: getuid has no pointers or failure case.
            || PathBuf::from(format!("/tmp/parcela-{}", unsafe { libc::getuid() })),
            |path| path.join("parcela"),
        )
}

#[cfg(target_os = "macos")]
fn user_directory() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut buffer = vec![0u8; 1024];
    // SAFETY: confstr writes at most buffer.len() bytes, NUL included, into the buffer.
    let length = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if length == 0 || length > buffer.len() {
        return None;
    }
    buffer.truncate(length - 1);
    Some(
        PathBuf::from(std::ffi::OsString::from_vec(buffer))
            .components()
            .collect(),
    )
}

#[cfg(not(target_os = "macos"))]
fn user_directory() -> Option<PathBuf> {
    None
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

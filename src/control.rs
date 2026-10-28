//! Bounded, generation-bound commands to the live environment owner. Never signals stored PIDs.
use crate::{private, registry::Session};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Status,
    Stop,
    Services,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveState {
    Starting,
    Ready,
    Unhealthy,
    Stopping,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct EndpointHealth {
    pub hostname: String,
    pub public_url: String,
    pub backend_port: u16,
    pub listener_owned: bool,
    pub listener_pids: Vec<u32>,
    pub http_ready: bool,
    /// Where other devices reach it, when the run is shared.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct LiveStatus {
    pub state: LiveState,
    pub http_ready: bool,
    pub listener_owned: bool,
    pub router_owned: bool,
    pub route_published: bool,
    pub owner_pid: u32,
    pub listener_pids: Vec<u32>,
    pub endpoints: std::collections::BTreeMap<String, EndpointHealth>,
    pub checked_at_unix_ms: u128,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    environment_id: String,
    token: String,
    generation: i64,
    action: Action,
}
#[derive(Serialize, Deserialize)]
struct Reply {
    environment_id: String,
    token: String,
    generation: i64,
    result: Option<Value>,
    error: Option<String>,
}

pub struct Pending {
    pub action: Action,
    stream: UnixStream,
}
impl Pending {
    pub fn reply(mut self, session: &Session, result: Result<Value>) -> Result<()> {
        reply(&mut self.stream, session, result)
    }
}
fn reply(stream: &mut UnixStream, session: &Session, result: Result<Value>) -> Result<()> {
    let (result, error) = match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    serde_json::to_writer(
        &mut *stream,
        &Reply {
            environment_id: session.environment_id.clone(),
            token: session.token.clone(),
            generation: session.generation,
            result,
            error,
        },
    )?;
    stream.write_all(b"\n")?;
    Ok(())
}
fn line(stream: &UnixStream, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    BufReader::new(stream)
        .take((limit + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    ensure!(
        bytes.len() <= limit && bytes.last() == Some(&b'\n'),
        "invalid or oversized owner message"
    );
    Ok(bytes)
}

pub struct Server {
    listener: UnixListener,
    path: PathBuf,
    device: u64,
    inode: u64,
}
/// Removes an owner's control socket that nothing listens on any more. Only for a caller that
/// holds the environment's ownership lock, so no owner can be binding it at the same time.
pub fn remove_dead(runtime: &Path) -> Result<()> {
    let path = crate::sockets::environment(runtime)?.control;
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return Ok(());
    };
    crate::sockets::verify(&path)?;
    // SAFETY: getuid has no pointers or failure case.
    ensure!(
        metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::getuid() },
        "unexpected file at the owner control socket; no takeover performed"
    );
    match UnixStream::connect(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
            Ok(fs::remove_file(&path)?)
        }
        _ => Ok(()),
    }
}

impl Server {
    pub fn bind(runtime: &Path) -> Result<Self> {
        private::directory(runtime)?;
        let path = crate::sockets::environment(runtime)?.control;
        crate::sockets::prepare(&path)?;
        let listener = UnixListener::bind(&path).context(
            "owner control socket already exists or is unavailable; no takeover performed",
        )?;
        let metadata = fs::symlink_metadata(&path)?;
        let server = Self {
            listener,
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        fs::set_permissions(&server.path, fs::Permissions::from_mode(0o600))?;
        server.listener.set_nonblocking(true)?;
        Ok(server)
    }
    pub fn receive(&self, session: &Session) -> Result<Option<Pending>> {
        let (mut stream, _) = match self.listener.accept() {
            Ok(connection) => connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let parsed = (|| -> Result<Request> {
            stream.set_nonblocking(false)?;
            if stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .and_then(|()| stream.set_write_timeout(Some(Duration::from_millis(200))))
                .is_err()
            {
                stream.set_nonblocking(true)?;
            }
            let request: Request = serde_json::from_slice(&line(&stream, 8192)?)?;
            ensure!(
                request.environment_id == session.environment_id
                    && request.token == session.token
                    && request.generation == session.generation,
                "stale or foreign owner request"
            );
            Ok(request)
        })();
        match parsed {
            Ok(request) => Ok(Some(Pending {
                action: request.action,
                stream,
            })),
            Err(error) => {
                let _ = reply(&mut stream, session, Err(error));
                Ok(None)
            }
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.dev() == self.device
                && metadata.ino() == self.inode
                && metadata.file_type().is_socket()
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn request(runtime: &Path, session: &Session, action: Action) -> Result<Value> {
    request_with_timeout(runtime, session, action, Duration::from_secs(6))
}

pub(crate) fn request_with_timeout(
    runtime: &Path,
    session: &Session,
    action: Action,
    timeout: Duration,
) -> Result<Value> {
    ensure!(!timeout.is_zero(), "inventory probe budget exhausted");
    private::existing_directory(runtime)?;
    let path = crate::sockets::environment(runtime)?.control;
    crate::sockets::verify(&path).context("live owner unavailable; interrupted runs require explicit reconciliation, never PID signaling")?;
    let metadata = fs::symlink_metadata(&path).context("live owner unavailable; interrupted runs require explicit reconciliation, never PID signaling")?;
    // SAFETY: getuid has no pointers or failure case.
    ensure!(
        metadata.file_type().is_socket()
            && metadata.uid() == unsafe { libc::getuid() }
            && metadata.mode() & 0o077 == 0,
        "invalid private owner socket"
    );
    let mut stream =
        UnixStream::connect(&path).context("live owner unavailable; no process was signaled")?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout.min(Duration::from_secs(1))))?;
    serde_json::to_writer(
        &mut stream,
        &Request {
            environment_id: session.environment_id.clone(),
            token: session.token.clone(),
            generation: session.generation,
            action,
        },
    )?;
    stream.write_all(b"\n")?;
    let reply: Reply = serde_json::from_slice(&line(&stream, 4_000_000)?)?;
    ensure!(
        reply.environment_id == session.environment_id
            && reply.token == session.token
            && reply.generation == session.generation,
        "owner response does not match selected generation"
    );
    if let Some(error) = reply.error {
        anyhow::bail!("{error}");
    }
    reply.result.context("owner omitted response")
}

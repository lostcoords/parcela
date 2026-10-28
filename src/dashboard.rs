//! Read-only browser inventory, served by the proxy over a private Unix socket.
use crate::{frontdoor::FrontDoor, inventory, private, sockets};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const HOST: &str = "parcela-ui.localhost";

#[derive(Default, Serialize)]
struct Snapshot {
    checked_at_unix_ms: Option<u128>,
    environments: Vec<Environment>,
    error: Option<String>,
}
#[derive(Serialize)]
struct Environment {
    id: Option<String>,
    project: String,
    name: String,
    state: String,
    url: Option<String>,
    services: Vec<Service>,
    processes: Vec<Process>,
}
#[derive(Serialize)]
struct Service {
    name: String,
    url: String,
}
#[derive(Serialize)]
struct Process {
    name: String,
    state: String,
    pid: Option<u64>,
}
impl Environment {
    fn from_row(row: &Value) -> Self {
        let string = |field: &Value| field.as_str().unwrap_or_default().to_owned();
        Self {
            id: row["workspace"]["id"].as_str().map(str::to_owned),
            project: string(&row["workspace"]["project_id"]),
            name: string(&row["workspace"]["label"]),
            state: string(&row["state"]),
            url: row["url"].as_str().map(str::to_owned),
            services: row["urls"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(name, url)| {
                    Some(Service {
                        name: name.clone(),
                        url: url.as_str()?.to_owned(),
                    })
                })
                .collect(),
            processes: row["processes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|process| Process {
                    name: string(&process["name"]),
                    state: string(&process["state"]),
                    pid: process["pid"].as_u64(),
                })
                .collect(),
        }
    }
}

pub(crate) struct Server {
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    socket: PathBuf,
    identity: private::Identity,
}
impl Server {
    pub(crate) fn start(state: &Path, front_door: FrontDoor) -> Result<Self> {
        let socket = sockets::dashboard(state)?;
        sockets::prepare(&socket)?;
        let evidence = state.join("proxy/dashboard-socket.json");
        if socket.exists() {
            let old: private::Identity = private::read_json(&evidence)
                .context("dashboard socket has no ownership evidence; preserving it")?;
            ensure!(
                UnixStream::connect(&socket).is_err(),
                "dashboard socket is still in use; preserving it"
            );
            old.remove_socket(&socket)?;
        }
        let listener = UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let identity = private::Identity::read(&socket)?;
        private::write_json(&evidence, &identity)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let state = state.to_owned();
        let collecting = Arc::clone(&stopped);
        let collected = Arc::clone(&snapshot);
        // This thread never holds the proxy's registry, route locks or renewal loop.
        thread::Builder::new()
            .name("parcela-inventory".into())
            .spawn(move || {
                while !collecting.load(Ordering::Relaxed) {
                    let next = match inventory::collect(&state, None) {
                        Ok(inventory) => Snapshot {
                            checked_at_unix_ms: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .ok()
                                .map(|time| time.as_millis()),
                            environments: inventory
                                .workspaces
                                .iter()
                                .chain(&inventory.unregistered)
                                .map(Environment::from_row)
                                .collect(),
                            error: None,
                        },
                        Err(error) => {
                            eprintln!("parcela: dashboard inventory unavailable: {error:#}");
                            Snapshot {
                                error: Some(
                                    "Inventory unavailable. Check parcela status in a terminal."
                                        .into(),
                                ),
                                ..Snapshot::default()
                            }
                        }
                    };
                    if let Ok(mut snapshot) = collected.lock() {
                        *snapshot = next;
                    }
                    for _ in 0..20 {
                        if collecting.load(Ordering::Relaxed) {
                            return;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            })?;
        let serving = Arc::clone(&stopped);
        let worker = thread::Builder::new()
            .name("parcela-dashboard".into())
            .spawn(move || {
                while !serving.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = serve(&mut stream, front_door, &snapshot);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        Err(error) => {
                            eprintln!("parcela: dashboard listener failed: {error}");
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            stopped,
            worker: Some(worker),
            socket,
            identity,
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = self.identity.remove_socket(&self.socket);
    }
}

fn serve(stream: &mut UnixStream, front_door: FrontDoor, snapshot: &Mutex<Snapshot>) -> Result<()> {
    stream.set_write_timeout(Some(Duration::from_millis(250)))?;
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(
            !remaining.is_zero() && bytes.len() < 16384,
            "dashboard request headers exceeded budget"
        );
        stream.set_read_timeout(Some(remaining))?;
        let mut buffer = [0; 1024];
        let size = stream.read(&mut buffer)?;
        ensure!(size > 0, "incomplete dashboard request");
        bytes.extend_from_slice(&buffer[..size]);
    }
    let header = std::str::from_utf8(&bytes)?;
    let mut lines = header.split("\r\n");
    let mut request = lines.next().unwrap_or_default().split_whitespace();
    let method = request.next().unwrap_or_default();
    let target = request.next().unwrap_or_default();
    let mut host = None;
    let mut origin = None;
    let mut fetch_site = None;
    for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').context("invalid dashboard header")?;
        match name.to_ascii_lowercase().as_str() {
            "host" => {
                ensure!(host.is_none(), "duplicate dashboard host");
                host = Some(value.trim());
            }
            "origin" => {
                ensure!(origin.is_none(), "duplicate dashboard origin");
                origin = Some(value.trim());
            }
            "sec-fetch-site" => fetch_site = Some(value.trim()),
            _ => (),
        }
    }
    let authority = front_door
        .url(HOST)
        .trim_start_matches("https://")
        .to_owned();
    let path = target.split('?').next().unwrap_or_default();
    let public_asset = path == "/widget.js";
    let foreign = origin.is_some_and(|origin| origin != front_door.url(HOST));
    let (status, mime, body) = if host != Some(authority.as_str()) {
        (403, "text/plain", b"Unknown dashboard host\n".to_vec())
    } else if method != "GET" && method != "HEAD" {
        (405, "text/plain", b"Read-only dashboard\n".to_vec())
    } else if !public_asset
        && (foreign || (path == "/api/status" && fetch_site == Some("cross-site")))
    {
        (
            403,
            "text/plain",
            b"Cross-origin inventory request refused\n".to_vec(),
        )
    } else {
        match path {
            "/" | "/workspace" => (
                200,
                "text/html; charset=utf-8",
                include_bytes!("dashboard/index.html").to_vec(),
            ),
            "/app.js" => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("dashboard/app.js").to_vec(),
            ),
            "/style.css" => (
                200,
                "text/css; charset=utf-8",
                include_bytes!("dashboard/style.css").to_vec(),
            ),
            "/widget.js" => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("dashboard/widget.js").to_vec(),
            ),
            "/api/status" => (
                200,
                "application/json",
                serde_json::to_vec(
                    &*snapshot
                        .lock()
                        .map_err(|_| anyhow::anyhow!("inventory cache unavailable"))?,
                )?,
            ),
            _ => (404, "text/plain", b"No Parcela page here\n".to_vec()),
        }
    };
    let policy = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-src https://*.localhost:*; frame-ancestors https://*.localhost:*; base-uri 'none'; form-action 'none'";
    write!(
        stream,
        "HTTP/1.1 {status} {}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: {policy}\r\nConnection: close\r\n\r\n",
        match status {
            200 => "OK",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            _ => "Error",
        },
        body.len()
    )?;
    if method != "HEAD" {
        stream.write_all(&body)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn browser_inventory_only_contains_the_declared_public_fields() {
        let row = json!({
            "workspace": {"id":"id", "project_id":"demo", "label":"auth", "source":"/private/source"},
            "recorded_session": {"token":"secret", "generation":12},
            "live": {"owner_pid":123}, "diagnostic":"private failure",
            "state":"ready", "url":"https://auth-demo.localhost:1443",
            "urls":{"web":"https://auth-demo.localhost:1443"},
            "processes":[{"name":"web", "state":"running", "pid":42, "command":"secret command"}]
        });
        let public = serde_json::to_value(Environment::from_row(&row)).unwrap();
        assert_eq!(
            public,
            json!({"id":"id", "project":"demo", "name":"auth", "state":"ready", "url":"https://auth-demo.localhost:1443", "services":[{"name":"web", "url":"https://auth-demo.localhost:1443"}], "processes":[{"name":"web", "state":"running", "pid":42}]})
        );
    }

    fn response(request: &str) -> String {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let worker = thread::spawn(move || {
            serve(
                &mut server,
                FrontDoor::on(1443),
                &Mutex::new(Snapshot::default()),
            )
        });
        client.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        worker.join().unwrap().unwrap();
        response
    }

    #[test]
    fn dashboard_enforces_host_origin_and_read_only_methods() {
        assert!(
            response("GET /api/status HTTP/1.1\r\nHost: evil.example\r\n\r\n")
                .starts_with("HTTP/1.1 403")
        );
        assert!(response("GET /api/status HTTP/1.1\r\nHost: parcela-ui.localhost:1443\r\nOrigin: https://evil.example\r\n\r\n").starts_with("HTTP/1.1 403"));
        assert!(
            response("POST /api/status HTTP/1.1\r\nHost: parcela-ui.localhost:1443\r\n\r\n")
                .starts_with("HTTP/1.1 405")
        );
        let status = response(
            "GET /api/status HTTP/1.1\r\nHost: parcela-ui.localhost:1443\r\nOrigin: https://parcela-ui.localhost:1443\r\n\r\n",
        );
        assert!(status.starts_with("HTTP/1.1 200"));
        assert!(!status.contains("Access-Control-Allow-Origin"));
        assert!(response("GET /widget.js HTTP/1.1\r\nHost: parcela-ui.localhost:1443\r\nOrigin: https://demo.localhost:1443\r\n\r\n").starts_with("HTTP/1.1 200"));
    }
}

//! Read-only browser inventory, served by the proxy over a private Unix socket.
use crate::{environment::inventory, proxy::frontdoor::FrontDoor, state::private, state::sockets};
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

mod environment;

pub const HOST: &str = "parcela-dashboard.localhost";

#[derive(Default, Serialize)]
struct Snapshot {
    checked_at_unix_ms: Option<u128>,
    environments: Vec<Environment>,
    error: Option<String>,
    #[serde(skip)]
    rows: Vec<Value>,
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
        let (listener, identity) = listen(&socket, &state.join("proxy/dashboard-socket.json"))?;
        listener.set_nonblocking(true)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let state = state.to_owned();
        let served_state = state.clone();
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
                            rows: inventory
                                .workspaces
                                .into_iter()
                                .chain(inventory.unregistered)
                                .collect(),
                        },
                        Err(error) => {
                            crate::paint::Paint::note(&format!(
                                "dashboard inventory unavailable: {error:#}"
                            ));
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
        let actions = environment::Actions::default();
        let worker = thread::Builder::new()
            .name("parcela-dashboard".into())
            .spawn(move || {
                while !serving.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // BSD sockets inherit the listener's non-blocking mode, which would
                            // cut any response off at the socket buffer.
                            let _ = stream.set_nonblocking(false);
                            let _ =
                                serve(&mut stream, &served_state, front_door, &snapshot, &actions);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        // Out of file descriptors, for one: those come back, so keep listening.
                        Err(error) => {
                            crate::paint::Paint::note(&format!("dashboard listener: {error}"));
                            thread::sleep(Duration::from_secs(1));
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

/// Binds a private socket for Caddy, replacing only a dead one this proxy's evidence names.
pub(crate) fn listen(socket: &Path, evidence: &Path) -> Result<(UnixListener, private::Identity)> {
    sockets::prepare(socket)?;
    if socket.exists() {
        let old: private::Identity = private::read_json(evidence).with_context(|| {
            format!(
                "{} has no ownership evidence; preserving it",
                socket.display()
            )
        })?;
        ensure!(
            UnixStream::connect(socket).is_err(),
            "{} is still in use; preserving it",
            socket.display()
        );
        old.remove_socket(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    let identity = private::Identity::read(socket)?;
    private::write_json(evidence, &identity)?;
    Ok((listener, identity))
}

fn serve(
    stream: &mut UnixStream,
    state: &Path,
    front_door: FrontDoor,
    snapshot: &Mutex<Snapshot>,
    actions: &environment::Actions,
) -> Result<()> {
    stream.set_write_timeout(Some(Duration::from_millis(250)))?;
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(
            !remaining.is_zero() && bytes.len() < 64 * 1024,
            "dashboard request headers exceeded budget"
        );
        stream.set_read_timeout(Some(remaining))?;
        let mut buffer = [0; 1024];
        let size = stream.read(&mut buffer)?;
        ensure!(size > 0, "incomplete dashboard request");
        bytes.extend_from_slice(&buffer[..size]);
    }
    // Only Parcela's own ASCII headers matter here; an app's cookie can hold anything.
    let header = String::from_utf8_lossy(&bytes);
    let mut lines = header.split("\r\n");
    let mut request = lines.next().unwrap_or_default().split_whitespace();
    let method = request.next().unwrap_or_default();
    let target = request.next().unwrap_or_default();
    let mut host = None;
    let mut origin = None;
    let mut fetch_site = None;
    let mut environment = None;
    let mut hostname = None;
    let mut acting = false;
    let mut access = None;
    let mut cookie = None;
    let mut user_agent = "";
    let mut fetch_dest = None;
    let mut client = "";
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
            "x-parcela-environment" => environment = Some(value.trim()),
            "x-parcela-hostname" => hostname = Some(value.trim()),
            "x-parcela-action" => acting = true,
            // HTTP/2 may split cookies across several header lines.
            "cookie" => cookie = cookie.or(crate::proxy::devices::cookie_token(value)),
            "user-agent" => user_agent = value.trim(),
            "sec-fetch-dest" => fetch_dest = Some(value.trim()),
            "x-parcela-client" => client = value.trim(),
            "x-parcela-access" => {
                ensure!(access.is_none(), "duplicate dashboard access");
                access = Some(value.trim());
            }
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
    let mut policy = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";
    let mut extra = Vec::new();
    let device_policy;
    let (status, mime, body) = if access == Some("device") {
        // Caddy's forward_auth for a LAN name: only this gate, never the panel or an action.
        let answer = crate::proxy::devices::answer(
            state,
            &crate::proxy::devices::Ask {
                host: hostname.unwrap_or_default(),
                cookie,
                navigation: matches!(fetch_dest, None | Some("document")),
                user_agent,
                address: client,
            },
        )?;
        device_policy = crate::proxy::devices::policy(answer.nonce.as_deref());
        policy = &device_policy;
        extra = answer.headers;
        (answer.status, answer.mime, answer.body)
    } else if access == Some("certificate") {
        let answer = crate::proxy::devices::certificate(state, path)?;
        device_policy = crate::proxy::devices::policy(None);
        policy = &device_policy;
        extra = answer.headers;
        (answer.status, answer.mime, answer.body)
    } else if let (true, Some(path)) = (
        host == Some(authority.as_str()),
        target.strip_prefix("/_parcela/devices"),
    ) {
        let response = environment::devices(method, path, host, origin, acting, state);
        (response.status, response.mime, response.body)
    } else if let (Some(environment), Some(hostname), Some(path)) =
        (environment, hostname, target.strip_prefix("/_parcela/"))
    {
        let response = environment::respond(
            &environment::Request {
                method,
                target: path,
                host,
                origin,
                acting,
                local: access == Some("local"),
                shared: access == Some("shared"),
                lan: access == Some("lan"),
                environment,
                hostname,
            },
            state,
            &front_door.url(HOST),
            snapshot,
            actions,
        );
        if let Some(response_policy) = response.policy {
            policy = response_policy;
        }
        (response.status, response.mime, response.body)
    } else if let (true, Some(path)) = (
        host == Some(authority.as_str()),
        target.strip_prefix("/_parcela/"),
    ) {
        // The dashboard's own name: Caddy deletes the environment headers, so this is the full
        // panel that manages every environment.
        let response = environment::respond(
            &environment::Request {
                method,
                target: path,
                host,
                origin,
                acting,
                local: true,
                shared: false,
                lan: false,
                environment: "",
                hostname: HOST,
            },
            state,
            &front_door.url(HOST),
            snapshot,
            actions,
        );
        if let Some(response_policy) = response.policy {
            policy = response_policy;
        }
        (response.status, response.mime, response.body)
    } else if host != Some(authority.as_str()) {
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
            "/" => (
                200,
                "text/html; charset=utf-8",
                include_bytes!("dashboard/index.html").to_vec(),
            ),
            "/logs" => (
                200,
                "text/html; charset=utf-8",
                include_bytes!("dashboard/logs.html").to_vec(),
            ),
            "/logs.js" => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("dashboard/logs.js").to_vec(),
            ),
            "/app.js" => (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("dashboard/app.js").to_vec(),
            ),
            "/icon.png" => (
                200,
                "image/png",
                include_bytes!("dashboard/icon.png").to_vec(),
            ),
            "/ui.css" => (
                200,
                "text/css; charset=utf-8",
                include_bytes!("dashboard/ui.css").to_vec(),
            ),
            "/widget.js" => (
                200,
                "text/javascript; charset=utf-8",
                widget(&front_door.url(HOST)),
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
    write!(
        stream,
        "HTTP/1.1 {status} {}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: {policy}\r\n{}Connection: close\r\n\r\n",
        match status {
            200 => "OK",
            202 => "Accepted",
            400 => "Bad Request",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            409 => "Conflict",
            500 => "Internal Server Error",
            _ => "Error",
        },
        body.len(),
        extra
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect::<String>()
    )?;
    if method != "HEAD" {
        stream.write_all(&body)?;
    }
    Ok(())
}

fn widget(dashboard: &str) -> Vec<u8> {
    include_str!("dashboard/widget.js")
        .replace(
            "'__PARCELA_DASHBOARD__'",
            &Value::from(dashboard).to_string(),
        )
        .into_bytes()
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
        response_in(Path::new("/nonexistent"), request)
    }

    fn response_in(state: &Path, request: &str) -> String {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let state = state.to_owned();
        let worker = thread::spawn(move || {
            serve(
                &mut server,
                &state,
                FrontDoor::on(1443),
                &Mutex::new(Snapshot::default()),
                &environment::Actions::default(),
            )
        });
        client.write_all(request.as_bytes()).unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        worker.join().unwrap().unwrap();
        String::from_utf8_lossy(&response).into_owned()
    }

    #[test]
    fn dashboard_page_frames_nothing_and_cannot_be_framed() {
        let page = response("GET / HTTP/1.1\r\nHost: parcela-dashboard.localhost:1443\r\n\r\n");
        assert!(page.starts_with("HTTP/1.1 200"));
        assert!(page.contains("frame-ancestors 'none'"));
        assert!(!page.contains("frame-src"));
        assert!(!page.contains("<iframe"));
        for (path, status) in [
            ("/ui.css", 200),
            ("/logs", 200),
            ("/logs.js", 200),
            ("/workspace", 404),
        ] {
            let reply = response(&format!(
                "GET {path} HTTP/1.1\r\nHost: parcela-dashboard.localhost:1443\r\n\r\n"
            ));
            assert!(reply.starts_with(&format!("HTTP/1.1 {status}")), "{path}");
        }
    }

    #[test]
    fn a_large_or_non_utf8_header_does_not_break_a_request() {
        let cookie = "a".repeat(32 * 1024);
        let reply = response(&format!(
            "GET /widget.js HTTP/1.1\r\nHost: parcela-dashboard.localhost:1443\r\nCookie: {cookie}\r\n\r\n"
        ));
        assert!(reply.starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn dashboard_enforces_host_origin_and_read_only_methods() {
        assert!(
            response("GET /api/status HTTP/1.1\r\nHost: evil.example\r\n\r\n")
                .starts_with("HTTP/1.1 403")
        );
        assert!(response("GET /api/status HTTP/1.1\r\nHost: parcela-dashboard.localhost:1443\r\nOrigin: https://evil.example\r\n\r\n").starts_with("HTTP/1.1 403"));
        assert!(
            response("POST /api/status HTTP/1.1\r\nHost: parcela-dashboard.localhost:1443\r\n\r\n")
                .starts_with("HTTP/1.1 405")
        );
        let status = response(
            "GET /api/status HTTP/1.1\r\nHost: parcela-dashboard.localhost:1443\r\nOrigin: https://parcela-dashboard.localhost:1443\r\n\r\n",
        );
        assert!(status.starts_with("HTTP/1.1 200"));
        assert!(!status.contains("Access-Control-Allow-Origin"));
        assert!(response("GET /widget.js HTTP/1.1\r\nHost: parcela-dashboard.localhost:1443\r\nOrigin: https://demo.localhost:1443\r\n\r\n").starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn only_the_dashboard_name_decides_about_devices() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path();
        let gate = "GET /_parcela/device HTTP/1.1\r\nHost: shop.local\r\nX-Parcela-Access: device\r\nX-Parcela-Hostname: shop.local\r\nX-Parcela-Client: 192.168.1.30\r\nUser-Agent: Phone\r\nSec-Fetch-Dest: document\r\n\r\n";
        let waiting = response_in(state, gate);
        assert!(waiting.starts_with("HTTP/1.1 403"), "{waiting}");
        assert!(waiting.contains("Set-Cookie: parcela_device="), "{waiting}");
        assert!(waiting.contains("Waiting for approval on"), "{waiting}");
        let token = waiting
            .split("parcela_device=")
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let id = crate::proxy::devices::list(state).unwrap()[0].id.clone();
        let dashboard = "parcela-dashboard.localhost:1443";
        let decide = |host: &str, origin: &str, acting: &str| {
            response_in(
                state,
                &format!(
                    "POST /_parcela/devices/{id}/approve HTTP/1.1\r\nHost: {host}\r\nOrigin: {origin}\r\n{acting}\r\n"
                ),
            )
        };
        for refused in [
            decide(dashboard, "https://evil.example", "X-Parcela-Action: 1\r\n"),
            decide(dashboard, &format!("https://{dashboard}"), ""),
            decide(
                "shop.local",
                "https://shop.local",
                "X-Parcela-Action: 1\r\n",
            ),
        ] {
            assert!(!refused.starts_with("HTTP/1.1 200"), "{refused}");
        }
        assert_eq!(
            crate::proxy::devices::list(state).unwrap()[0].standing,
            crate::proxy::devices::Standing::Pending
        );
        let listed = response_in(
            state,
            &format!("GET /_parcela/devices HTTP/1.1\r\nHost: {dashboard}\r\n\r\n"),
        );
        assert!(listed.contains(&format!("\"id\":\"{id}\"")), "{listed}");
        assert!(!listed.contains("token"), "{listed}");
        let approved = decide(
            dashboard,
            &format!("https://{dashboard}"),
            "X-Parcela-Action: 1\r\n",
        );
        assert!(approved.starts_with("HTTP/1.1 200"), "{approved}");
        let allowed = response_in(
            state,
            &gate.replace(
                "\r\n\r\n",
                &format!("\r\nCookie: a=1; parcela_device={token}\r\n\r\n"),
            ),
        );
        assert!(allowed.starts_with("HTTP/1.1 200"), "{allowed}");
    }

    #[test]
    fn a_lan_name_gets_the_root_certificate_and_its_steps_without_pairing() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path();
        let root = crate::proxy::certificate(state);
        std::fs::create_dir_all(root.parent().unwrap()).unwrap();
        std::fs::write(&root, include_str!("../../tests/fixtures/root.crt")).unwrap();
        std::fs::write(root.with_file_name("root.key"), "PRIVATE KEY").unwrap();
        let request = |path: &str| {
            response_in(
                state,
                &format!(
                    "GET {path} HTTP/1.1\r\nHost: shop.local\r\nX-Parcela-Access: certificate\r\n\r\n"
                ),
            )
        };
        let file = request("/_parcela/ca.crt");
        assert!(file.starts_with("HTTP/1.1 200"), "{file}");
        assert!(file.contains("Content-Type: application/x-x509-ca-cert\r\n"));
        assert!(file.contains("Content-Disposition: attachment; filename=\"parcela-ca.crt\"\r\n"));
        assert!(!file.contains("PRIVATE KEY"));
        let page = request("/_parcela/certificate");
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains(&crate::proxy::fingerprint(
            &crate::proxy::certificate_der(state).unwrap()
        )));
        assert!(page.contains("Certificate Trust Settings"));
        assert!(request("/_parcela/ca.key").starts_with("HTTP/1.1 404"));
        assert!(request("/").starts_with("HTTP/1.1 404"));
        assert!(
            crate::proxy::devices::list(state).unwrap().is_empty(),
            "the certificate asks for no pairing"
        );
    }
}

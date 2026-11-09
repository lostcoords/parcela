//! `/_parcela/` on an app host. Caddy names the environment in `X-Parcela-Environment`, the route's
//! own hostname in `X-Parcela-Hostname`, and sets `X-Parcela-Access`: `local` on a loopback name,
//! `shared` on a domain name. The dashboard name never carries them and LAN names never route here.
//! A shared name is read-only and sees no logs or opened files, which can hold secrets and sign-in
//! links, and a request with neither access is refused.
use super::{Snapshot, widget};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

const PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Parcela</title></head><body style=\"margin:0;background:#17181b\"><script src=\"/_parcela/widget.js\" data-page></script></body></html>";

const BADGE: &[u8] = include_bytes!("badge.png");
const POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
const UNAVAILABLE: &str = include_str!("unavailable.html");
const UNAVAILABLE_SCRIPT: &str = include_str!("unavailable.js");

pub(super) type Actions = Arc<Mutex<BTreeMap<String, Action>>>;

#[derive(Clone, Serialize)]
pub(super) struct Action {
    name: String,
    state: &'static str,
    message: Option<String>,
    at_unix_ms: u128,
}

pub(super) struct Request<'a> {
    pub method: &'a str,
    pub target: &'a str,
    pub host: Option<&'a str>,
    pub origin: Option<&'a str>,
    pub acting: bool,
    pub local: bool,
    pub shared: bool,
    /// Empty on the dashboard's own name, which may manage every environment. An app page
    /// manages only its own, so a script on one app cannot stop another or read its logs.
    pub environment: &'a str,
    pub hostname: &'a str,
}

impl Request<'_> {
    fn dashboard(&self) -> bool {
        self.environment.is_empty()
    }

    fn manages(&self, key: &str) -> bool {
        self.dashboard() || key == self.environment
    }
}

pub(super) struct Response {
    pub status: u16,
    pub mime: &'static str,
    pub body: Vec<u8>,
    pub policy: Option<&'static str>,
}
impl Response {
    fn json(status: u16, value: &Value) -> Self {
        Self::new(status, "application/json", value.to_string().into_bytes())
    }
    fn text(status: u16, text: &str) -> Self {
        Self::new(status, "text/plain", format!("{text}\n").into_bytes())
    }
    fn new(status: u16, mime: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            mime,
            body,
            policy: None,
        }
    }
}

pub(super) fn respond(
    request: &Request,
    state: &Path,
    dashboard: &str,
    snapshot: &Mutex<Snapshot>,
    actions: &Actions,
) -> Response {
    if !request.local && !request.shared {
        return Response::text(403, "Only a .localhost name reaches Parcela");
    }
    if !request
        .environment
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Response::text(400, "Invalid environment");
    }
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((request.target, ""));
    let runtime = state.join("environments").join(request.environment);
    let rows = || {
        snapshot
            .lock()
            .map(|snapshot| snapshot.rows.clone())
            .unwrap_or_default()
    };
    if request.shared {
        return shared(request, path, dashboard, &rows());
    }
    if request.method == "POST" {
        return act(request, path, state, &rows(), actions);
    }
    if request.method != "GET" && request.method != "HEAD" {
        return Response::text(405, "Method not allowed");
    }
    match path {
        "" => page(),
        "widget.js" => Response::new(200, "text/javascript; charset=utf-8", widget(dashboard)),
        "badge.png" => Response::new(200, "image/png", BADGE.to_vec()),
        "unavailable" if !request.dashboard() => Response {
            policy: Some(POLICY),
            ..Response::new(
                503,
                "text/html; charset=utf-8",
                UNAVAILABLE.as_bytes().to_vec(),
            )
        },
        "unavailable.js" if !request.dashboard() => Response::new(
            200,
            "text/javascript; charset=utf-8",
            UNAVAILABLE_SCRIPT
                .replace("__PARCELA_DASHBOARD__", dashboard)
                .into_bytes(),
        ),
        "opens" if request.dashboard() => Response::text(404, "Not found"),
        "opens" => {
            let offers: Vec<_> = crate::opens::list(&runtime)
                .unwrap_or_default()
                .into_iter()
                .map(|open| Offer::new(open, request.hostname))
                .collect();
            Response::json(
                200,
                &json!({"now_unix_ms": crate::opens::now_unix_ms(), "opens": offers}),
            )
        }
        "environments" => {
            let actions = actions
                .lock()
                .map(|actions| actions.clone())
                .unwrap_or_default();
            // An app page sees only its own environment; the dashboard's name lists them all.
            let environments: Vec<_> = rows()
                .iter()
                .filter(|row| request.dashboard() || key(row) == Some(request.environment))
                .map(|row| Listed::new(row, request, &actions))
                .collect();
            Response::json(200, &json!({"environments": environments}))
        }
        "logs" => logs(request, state, query, &rows()),
        _ if request.dashboard() => Response::text(404, "Not found"),
        _ => file(&runtime, path),
    }
}

pub(super) fn page() -> Response {
    Response {
        policy: Some(POLICY),
        ..Response::new(200, "text/html; charset=utf-8", PAGE.as_bytes().to_vec())
    }
}

/// What a domain name serves: the badge, this environment's state and the project's other running
/// environments to switch to. Nothing here changes anything or reads a log.
fn shared(request: &Request, path: &str, dashboard: &str, rows: &[Value]) -> Response {
    if request.method != "GET" && request.method != "HEAD" {
        return Response::text(403, "Only a .localhost name manages environments");
    }
    match path {
        "" => page(),
        "widget.js" => Response::new(200, "text/javascript; charset=utf-8", widget(dashboard)),
        "badge.png" => Response::new(200, "image/png", BADGE.to_vec()),
        "environments" => {
            let own = find(rows, request.environment);
            let environments: Vec<_> = rows
                .iter()
                .filter(|row| {
                    own.is_some_and(|own| {
                        row["workspace"]["project_id"] == own["workspace"]["project_id"]
                    })
                })
                .filter(|row| key(row) == Some(request.environment) || row["state"] == "ready")
                .map(|row| Listed::shared(row, request))
                .collect();
            Response::json(200, &json!({"environments": environments}))
        }
        _ => Response::text(403, "Only a .localhost name shows this"),
    }
}

fn parameter(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .and_then(|(_, value)| crate::opens::decode(&value.replace('+', " ")).ok())
}

/// Pages name environments by registered id only: an unregistered checkout is never started
/// from a page, since that would run commands nobody has started there before.
fn key(row: &Value) -> Option<&str> {
    row["workspace"]["id"].as_str()
}

fn find<'a>(rows: &'a [Value], wanted: &str) -> Option<&'a Value> {
    rows.iter().find(|row| key(row) == Some(wanted))
}

#[derive(Serialize)]
struct Listed {
    key: Option<String>,
    project: String,
    name: String,
    role: String,
    state: String,
    current: bool,
    /// Whether this page may start, stop and read the logs of the environment.
    manageable: bool,
    url: Option<String>,
    processes: Vec<Value>,
    action: Option<Action>,
}
impl Listed {
    fn new(row: &Value, request: &Request, actions: &BTreeMap<String, Action>) -> Self {
        let string = |field: &Value| field.as_str().unwrap_or_default().to_owned();
        let key = key(row).map(str::to_owned);
        Self {
            project: string(&row["workspace"]["project_id"]),
            name: string(&row["workspace"]["label"]),
            role: string(&row["workspace"]["role"]),
            state: string(&row["state"]),
            current: !request.dashboard() && key.as_deref() == Some(request.environment),
            manageable: key.as_deref().is_some_and(|key| request.manages(key)),
            url: row["url"].as_str().map(str::to_owned),
            processes: row["processes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|process| json!({"name": process["name"], "state": process["state"]}))
                .collect(),
            action: key.as_ref().and_then(|key| actions.get(key)).cloned(),
            key,
        }
    }
}

impl Listed {
    /// A row for a shared name: no id, processes or actions, and URLs under the page's own domain.
    fn shared(row: &Value, request: &Request) -> Self {
        let string = |field: &Value| field.as_str().unwrap_or_default().to_owned();
        let current = key(row) == Some(request.environment);
        let domain = request.hostname.split_once('.').map(|(_, domain)| domain);
        let label = row["url"]
            .as_str()
            .and_then(|url| url.strip_prefix("https://")?.split_once('.'))
            .map(|(label, _)| label);
        Self {
            key: None,
            project: string(&row["workspace"]["project_id"]),
            name: string(&row["workspace"]["label"]),
            role: string(&row["workspace"]["role"]),
            state: string(&row["state"]),
            current,
            manageable: false,
            url: label
                .zip(domain)
                .filter(|_| !current)
                .map(|(label, domain)| format!("https://{label}.{domain}")),
            processes: Vec::new(),
            action: None,
        }
    }
}

fn logs(request: &Request, state: &Path, query: &str, rows: &[Value]) -> Response {
    let Some(id) = parameter(query, "environment").filter(|wanted| find(rows, wanted).is_some())
    else {
        return Response::text(404, "No such environment");
    };
    if !request.manages(&id) {
        return Response::text(403, "Open the dashboard for another environment's logs");
    }
    let directory = crate::logs::directory(&state.join("environments").join(id));
    let names = crate::logs::names(&directory).unwrap_or_default();
    let process = parameter(query, "process")
        .filter(|name| names.contains(name))
        .or_else(|| names.iter().find(|name| *name == "web").cloned())
        .or_else(|| names.first().cloned());
    let count = parameter(query, "lines")
        .and_then(|lines| lines.parse().ok())
        .unwrap_or(200usize)
        .min(2000);
    let lines = process
        .as_ref()
        .map(|name| crate::logs::tail(&directory, name, count).unwrap_or_default())
        .unwrap_or_default();
    Response::json(
        200,
        &json!({"processes": names, "process": process, "lines": lines}),
    )
}

/// Start, stop or restart an environment through the CLI, in the background. Only environments
/// from the inventory can be named, only on a loopback name, only by a same-origin script (the
/// custom header makes a cross-site request preflight, which this server never approves), and
/// from an app page only its own environment.
fn act(request: &Request, path: &str, state: &Path, rows: &[Value], actions: &Actions) -> Response {
    let same_origin = match (request.origin, request.host) {
        (Some(origin), Some(host)) => origin.split_once("://").map(|(_, rest)| rest) == Some(host),
        _ => false,
    };
    if !request.acting || !same_origin {
        return Response::text(403, "Cross-origin action refused");
    }
    let Some((wanted, name)) = path
        .strip_prefix("environments/")
        .and_then(|rest| rest.rsplit_once('/'))
    else {
        return Response::text(404, "No such action");
    };
    let arguments: &[&str] = match name {
        "start" => &["up", "--detach"],
        "stop" => &["down"],
        "restart" => &["restart", "--detach"],
        _ => return Response::text(404, "No such action"),
    };
    let Some(key) = crate::opens::decode(wanted)
        .ok()
        .filter(|wanted| find(rows, wanted).is_some())
    else {
        return Response::text(404, "No such environment");
    };
    if !request.manages(&key) {
        return Response::text(403, "Manage other environments from the dashboard");
    }
    let Some(parcela) = executable() else {
        return Response::text(500, "Parcela executable unavailable");
    };
    let mut command = Command::new(parcela);
    command
        .arg("--state-dir")
        .arg(state)
        .arg("--workspace")
        .arg(&key)
        .args(arguments)
        .stdin(Stdio::null());
    // The proxy keeps the environment of the run that started it. Its sharing variables never
    // apply to what a page starts; the host's `sharing` settings do, as for any `up`.
    for name in crate::share::VARIABLES {
        command.env_remove(name);
    }
    let Ok(mut actions_now) = actions.lock() else {
        return Response::text(500, "Actions unavailable");
    };
    if actions_now
        .get(&key)
        .is_some_and(|action| action.state == "running")
    {
        return Response::text(409, "Another action is still running");
    }
    let running = Action {
        name: name.to_owned(),
        state: "running",
        message: None,
        at_unix_ms: crate::opens::now_unix_ms(),
    };
    let finished = Arc::clone(actions);
    let (key_now, running_now) = (key.clone(), running.clone());
    let spawned = std::thread::Builder::new()
        .name("parcela-action".into())
        .spawn(move || {
            let result = command.output();
            let action = Action {
                state: match &result {
                    Ok(output) if output.status.success() => "done",
                    _ => "failed",
                },
                message: match result {
                    Ok(output) if !output.status.success() => {
                        String::from_utf8_lossy(&output.stderr)
                            .lines()
                            .rev()
                            .find(|line| !line.trim().is_empty())
                            .map(str::to_owned)
                    }
                    Err(error) => Some(error.to_string()),
                    Ok(_) => None,
                },
                ..running
            };
            if let Ok(mut actions) = finished.lock() {
                actions.insert(key, action);
            }
        });
    match spawned {
        Ok(_) => {
            actions_now.insert(key_now, running_now);
            Response::json(202, &json!({"accepted": true}))
        }
        Err(_) => Response::text(500, "Could not start the action"),
    }
}

/// This binary, or the one installed in its place: Linux reports a replaced executable as
/// `PATH (deleted)`, and an upgrade puts the new release at `PATH`.
fn executable() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    if current.exists() {
        return Some(current);
    }
    let replaced = current.to_str()?.strip_suffix(" (deleted)")?;
    Some(PathBuf::from(replaced)).filter(|path| path.is_file())
}

fn file(runtime: &Path, path: &str) -> Response {
    let not_found = Response::text(404, "No Parcela page here");
    let Some((id, name)) = path
        .strip_prefix("files/")
        .and_then(|rest| rest.split_once('/'))
    else {
        return not_found;
    };
    let Ok(name) = crate::opens::decode(name) else {
        return not_found;
    };
    match crate::opens::sibling(runtime, id, &name)
        .and_then(|file| Ok((std::fs::read(&file)?, file)))
    {
        Ok((body, file)) => Response {
            policy: Some(
                "sandbox allow-popups allow-popups-to-escape-sandbox allow-top-navigation-by-user-activation; default-src 'self' data: https:; style-src 'self' 'unsafe-inline' https:; script-src 'none'; frame-ancestors 'self'",
            ),
            ..Response::new(200, content_type(&file), body)
        },
        Err(_) => not_found,
    }
}

#[derive(Serialize)]
struct Offer {
    id: String,
    at_unix_ms: u128,
    href: String,
    label: String,
}
impl Offer {
    fn new(open: crate::opens::Open, hostname: &str) -> Self {
        let (href, label) = match &open.target {
            crate::opens::Target::Url { url } => {
                let rest = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
                let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
                let host = authority
                    .rsplit_once(':')
                    .map_or(authority, |(host, _)| host);
                // Leading slashes would make `//other.example/` a link to another host.
                let href = if host.eq_ignore_ascii_case(hostname) {
                    format!("/{}", path.trim_start_matches(['/', '\\']))
                } else {
                    url.clone()
                };
                (href, rest.to_owned())
            }
            crate::opens::Target::File { path } => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let label = open.title.clone().unwrap_or_else(|| name.clone());
                (
                    format!("/_parcela/files/{}/{}", open.id, escape(&name)),
                    label,
                )
            }
        };
        Self {
            id: open.id,
            at_unix_ms: open.at_unix_ms,
            href,
            label,
        }
    }
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("txt" | "eml") => "text/plain; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn escape(name: &str) -> String {
    name.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opens::{Open, Target};

    fn offer(target: Target) -> Offer {
        let open = Open {
            id: "id".into(),
            at_unix_ms: 0,
            target,
            title: None,
        };
        Offer::new(open, "shop.localhost")
    }

    fn snapshot() -> Mutex<Snapshot> {
        Mutex::new(Snapshot {
            rows: vec![
                json!({"workspace": {"id": "abc", "source": "/src/shop"}}),
                json!({"workspace": {"id": "xyz", "source": "/src/blog"}}),
                json!({"workspace": {"source": "/src/new"}}),
            ],
            ..Snapshot::default()
        })
    }

    fn request<'a>(
        method: &'a str,
        path: &'a str,
        local: bool,
        environment: &'a str,
    ) -> Request<'a> {
        let host = if environment.is_empty() {
            "parcela-dashboard.localhost:1443"
        } else {
            "shop.localhost:1443"
        };
        Request {
            method,
            target: path,
            host: Some(host),
            origin: Some(if environment.is_empty() {
                "https://parcela-dashboard.localhost:1443"
            } else {
                "https://shop.localhost:1443"
            }),
            acting: true,
            local,
            shared: false,
            environment,
            hostname: "shop.localhost",
        }
    }

    fn post(path: &str, local: bool, actions: &Actions) -> u16 {
        let request = request("POST", path, local, "abc");
        respond(
            &request,
            Path::new("/nonexistent"),
            "",
            &snapshot(),
            actions,
        )
        .status
    }

    #[test]
    fn actions_refuse_shares_unregistered_checkouts_and_a_second_action() {
        let actions = Actions::default();
        assert_eq!(post("environments/abc/stop", false, &actions), 403);
        assert_eq!(post("environments/%2Fsrc%2Fnew/start", true, &actions), 404);
        assert_eq!(post("environments/abc/delete", true, &actions), 404);
        let running = Action {
            name: "start".into(),
            state: "running",
            message: None,
            at_unix_ms: 0,
        };
        actions.lock().unwrap().insert("abc".into(), running);
        assert_eq!(post("environments/abc/stop", true, &actions), 409);
    }

    #[test]
    fn an_app_page_manages_only_its_own_environment_and_the_dashboard_every_one() {
        let actions = Actions::default();
        let status = |method, path: &str, environment| {
            let request = request(method, path, true, environment);
            respond(
                &request,
                Path::new("/nonexistent"),
                "",
                &snapshot(),
                &actions,
            )
            .status
        };
        assert_eq!(status("POST", "environments/xyz/stop", "abc"), 403);
        assert_eq!(status("GET", "logs?environment=xyz", "abc"), 403);
        assert_eq!(status("GET", "logs?environment=abc", "abc"), 200);
        assert_eq!(status("GET", "logs?environment=xyz", ""), 200);
        assert_eq!(
            status("GET", "opens", ""),
            404,
            "the dashboard has no opened files"
        );
        let listed = respond(
            &request("GET", "environments", true, "abc"),
            Path::new("/nonexistent"),
            "",
            &snapshot(),
            &actions,
        );
        let listed: Value = serde_json::from_slice(&listed.body).unwrap();
        let keys: Vec<_> = listed["environments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, ["abc"], "an app page never lists another environment");
        let listed = respond(
            &request("GET", "environments", true, ""),
            Path::new("/nonexistent"),
            "",
            &snapshot(),
            &actions,
        );
        let listed: Value = serde_json::from_slice(&listed.body).unwrap();
        assert_eq!(listed["environments"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_shared_name_only_reads_and_never_sees_logs_ids_or_paths() {
        let row = |id: &str, project: &str, state: &str, label: &str| {
            json!({
                "workspace": {"id": id, "project_id": project, "label": label, "source": "/src/x"},
                "state": state,
                "url": format!("https://{label}.localhost:1443"),
                "processes": [{"name": "web", "state": "running", "pid": 7}],
            })
        };
        let snapshot = Mutex::new(Snapshot {
            rows: vec![
                row("abc", "shop", "ready", "shop"),
                row("sib", "shop", "ready", "auth-shop"),
                row("off", "shop", "stopped", "old-shop"),
                row("xyz", "blog", "ready", "blog"),
            ],
            ..Snapshot::default()
        });
        let actions = Actions::default();
        let shared = |method, path: &str| {
            let mut request = request(method, path, false, "abc");
            request.shared = true;
            request.hostname = "shop.tower.example.com";
            respond(&request, Path::new("/nonexistent"), "", &snapshot, &actions)
        };
        for (method, path) in [
            ("POST", "environments/abc/stop"),
            ("POST", "environments/abc/restart"),
            ("GET", "logs?environment=abc"),
            ("GET", "opens"),
            ("GET", "files/1/mail.html"),
            ("GET", "secret"),
        ] {
            assert_eq!(shared(method, path).status, 403, "{method} {path}");
        }
        assert_eq!(shared("GET", "widget.js").status, 200);
        assert_eq!(shared("GET", "badge.png").status, 200);
        assert_eq!(shared("GET", "").status, 200);
        let listed: Value = serde_json::from_slice(&shared("GET", "environments").body).unwrap();
        let text = listed.to_string();
        assert!(!text.contains("/src/") && !text.contains("pid"), "{text}");
        let rows = listed["environments"].as_array().unwrap();
        for row in rows {
            assert!(row["key"].is_null() && row["manageable"] == false, "{row}");
            assert_eq!(row["processes"], json!([]), "{row}");
        }
        let names: Vec<_> = rows
            .iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["shop", "auth-shop"], "this project's running ones");
        assert!(rows[0]["url"].is_null() && rows[0]["current"] == true);
        assert_eq!(rows[1]["url"], "https://auth-shop.tower.example.com");
    }

    #[test]
    fn own_host_urls_become_paths_that_never_name_another_host() {
        for (url, href) in [
            ("https://shop.localhost:1443/welcome?x=1", "/welcome?x=1"),
            (
                "https://shop.localhost:1443//evil.example/x",
                "/evil.example/x",
            ),
            (
                "https://shop.localhost:1443/\\evil.example/x",
                "/evil.example/x",
            ),
            (
                "https://other.localhost:1443/x",
                "https://other.localhost:1443/x",
            ),
        ] {
            let url = url.to_owned();
            assert_eq!(offer(Target::Url { url }).href, href);
        }
    }
}

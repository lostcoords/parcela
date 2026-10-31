//! `/_parcela/` on a loopback app host. Caddy names the environment in `X-Parcela-Environment`,
//! the route's own hostname in `X-Parcela-Hostname`, and sets `X-Parcela-Access: local`; the
//! dashboard name never carries them. LAN and domain names never route here, and a request
//! without loopback access is refused.
use super::{Snapshot, widget};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

const PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Parcela</title></head><body style=\"margin:0;background:#fff\"><script src=\"/_parcela/widget.js\" data-page></script></body></html>";

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
    pub environment: &'a str,
    pub hostname: &'a str,
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
    if !request.local {
        return Response::text(403, "Only a .localhost name reaches Parcela");
    }
    if request.environment.is_empty()
        || !request
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
    if request.method == "POST" {
        return act(request, path, state, &rows(), actions);
    }
    if request.method != "GET" && request.method != "HEAD" {
        return Response::text(405, "Method not allowed");
    }
    match path {
        "" => Response {
            policy: Some(
                "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            ),
            ..Response::new(200, "text/html; charset=utf-8", PAGE.as_bytes().to_vec())
        },
        "widget.js" => Response::new(200, "text/javascript; charset=utf-8", widget(dashboard)),
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
            let environments: Vec<_> = rows()
                .iter()
                .map(|row| Listed::new(row, request.environment, &actions))
                .collect();
            Response::json(200, &json!({"environments": environments}))
        }
        "logs" => logs(state, query, &rows()),
        _ => file(&runtime, path),
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
    url: Option<String>,
    processes: Vec<Value>,
    action: Option<Action>,
}
impl Listed {
    fn new(row: &Value, current: &str, actions: &BTreeMap<String, Action>) -> Self {
        let string = |field: &Value| field.as_str().unwrap_or_default().to_owned();
        let key = key(row).map(str::to_owned);
        Self {
            project: string(&row["workspace"]["project_id"]),
            name: string(&row["workspace"]["label"]),
            role: string(&row["workspace"]["role"]),
            state: string(&row["state"]),
            current: key.as_deref() == Some(current),
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

fn logs(state: &Path, query: &str, rows: &[Value]) -> Response {
    let Some(id) = parameter(query, "environment").filter(|wanted| find(rows, wanted).is_some())
    else {
        return Response::text(404, "No such environment");
    };
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
/// from the inventory can be named, only on a loopback name, and only by a same-origin script: the
/// custom header makes a cross-site request preflight, which this server never approves.
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
    // The proxy keeps the environment of the run that started it; sharing is per run, so a page
    // never shares what it starts.
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

    fn post(path: &str, local: bool, actions: &Actions) -> u16 {
        let snapshot = Mutex::new(Snapshot {
            rows: vec![
                json!({"workspace": {"id": "abc", "source": "/src/shop"}}),
                json!({"workspace": {"source": "/src/new"}}),
            ],
            ..Snapshot::default()
        });
        let request = Request {
            method: "POST",
            target: path,
            host: Some("shop.localhost:1443"),
            origin: Some("https://shop.localhost:1443"),
            acting: true,
            local,
            environment: "abc",
            hostname: "shop.localhost",
        };
        respond(&request, Path::new("/nonexistent"), "", &snapshot, actions).status
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

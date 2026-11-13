use super::output::Output;
use anyhow::Result;
use parcela::{
    config::{Manifest, locate},
    frontdoor::FrontDoor,
    process::on_path,
    proxy::Status,
};
use serde::Serialize;
use std::{os::unix::fs::MetadataExt, path::Path};

/// One readiness fact. `fix` is the command or action that turns it green.
#[derive(Serialize)]
struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
    fix: Option<String>,
}

impl Check {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
            fix: None,
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name,
            ok: false,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}

fn backend(name: &'static str, binary: &str, expect: &str, install: &str) -> Check {
    let Some(path) = on_path(binary) else {
        return Check::fail(name, format!("{binary} not found on PATH"), install);
    };
    match parcela::process::version(binary, "version") {
        Ok(version) if version.contains(expect) => {
            let version = version
                .split_whitespace()
                .find(|word| word.starts_with('v'))
                .unwrap_or(&version)
                .to_owned();
            Check::pass(name, format!("{version} at {}", path.display()))
        }
        Ok(version) => Check::fail(
            name,
            format!(
                "unsupported version at {}: {}",
                path.display(),
                version.lines().next().unwrap_or_default()
            ),
            install,
        ),
        Err(error) => Check::fail(name, format!("{error:#}"), install),
    }
}

fn tool(name: &'static str, candidates: &[&str]) -> Check {
    match candidates.iter().find_map(|candidate| on_path(candidate)) {
        Some(path) => Check::pass(name, path.display().to_string()),
        None => Check::fail(
            name,
            format!("{name} not found"),
            format!("install {name}; Parcela uses it to verify which process owns a port"),
        ),
    }
}

/// The same load and startability checks as `parcela up`, so the detail is the error up would print.
fn config(path: &Path) -> Check {
    let source = match locate(path) {
        Ok(source) => source,
        Err(error) => return Check::fail("config", format!("{error:#}"), "parcela init"),
    };
    let file = source.join("parcela.yaml");
    match Manifest::load(&file).and_then(|mut manifest| {
        parcela::compose::resolve(&mut manifest, &source)?;
        Ok(manifest)
    }) {
        Ok(manifest) => Check::pass(
            "config",
            format!("{} (project {})", file.display(), manifest.project.name),
        ),
        Err(error) => Check::fail(
            "config",
            format!("{error:#}"),
            format!("fix {}, then run parcela doctor", file.display()),
        ),
    }
}

fn state(path: &Path) -> Check {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        let parent = path.ancestors().skip(1).find(|ancestor| ancestor.exists());
        return match parent {
            Some(parent) if writable(parent) => Check::pass(
                "state",
                format!("{} will be created on first parcela up", path.display()),
            ),
            _ => Check::fail(
                "state",
                format!("cannot create {}", path.display()),
                "set PARCELA_STATE_DIR or --state-dir to a writable directory",
            ),
        };
    };
    // SAFETY: getuid takes no pointers and has no failure case.
    let uid = unsafe { libc::getuid() };
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.uid() != uid {
        Check::fail(
            "state",
            format!("{} is not a directory you own", path.display()),
            "set PARCELA_STATE_DIR or --state-dir to a private directory you own",
        )
    } else if metadata.mode() & 0o077 != 0 {
        Check::fail(
            "state",
            format!("{} is readable by other users", path.display()),
            format!("chmod 700 {}", path.display()),
        )
    } else if !writable(path) {
        Check::fail(
            "state",
            format!("{} is not writable", path.display()),
            format!("chmod u+w {}", path.display()),
        )
    } else {
        Check::pass("state", format!("{} (private)", path.display()))
    }
}

fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: path is a valid NUL-terminated string that outlives the call.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

fn proxy(state: &Path) -> Check {
    let front_door = match FrontDoor::load(state) {
        Ok(front_door) => front_door,
        Err(error) => {
            return Check::fail(
                "proxy",
                format!("{error:#}"),
                format!(
                    "fix or remove {}, then run `parcela proxy start`",
                    state.join("proxy/frontdoor.json").display()
                ),
            );
        }
    };
    if front_door.relay && !parcela::relay::installed() {
        return Check::fail(
            "proxy",
            format!(
                "URLs use port 443, but {} is missing",
                parcela::relay::location()
            ),
            "parcela relay install",
        );
    }
    match parcela::proxy::status(state) {
        Ok(Status::Running(router)) => Check::pass(
            "proxy",
            match parcela::proxy::protocol_warning(&router) {
                Some(warning) => format!("running, {}; {warning}", router.front_door.describe()),
                None => format!("running, {}", router.front_door.describe()),
            },
        ),
        Ok(Status::Stopped) => match Some(front_door.https)
            .filter(|port| std::net::TcpListener::bind(("127.0.0.1", *port)).is_err())
        {
            None => Check::pass(
                "proxy",
                format!(
                    "not running; `parcela up` starts it on {}",
                    front_door.describe()
                ),
            ),
            Some(port) => Check::fail(
                "proxy",
                format!(
                    "not running, and port {port} is in use by {}",
                    parcela::process::port_holder(port)
                ),
                "parcela proxy start --port <n>",
            ),
        },
        Ok(Status::Unhealthy(error)) => Check::fail(
            "proxy",
            format!("running but not answering: {error:#}"),
            "parcela proxy restart",
        ),
        Err(error) => Check::fail("proxy", format!("{error:#}"), "parcela proxy start"),
    }
}

/// Whether this checkout's environment already runs, so the next step is to open it.
fn running(path: &Path, state: &Path) -> bool {
    (|| -> Result<bool> {
        let source = locate(path)?;
        let manifest = Manifest::load(&source.join("parcela.yaml"))?;
        let Some(registry) = parcela::registry::Registry::open(state, false)? else {
            return Ok(false);
        };
        let workspace = registry.current(&source, &manifest)?;
        let status = parcela::lifecycle::status(&registry, &workspace)?;
        Ok(status["state"] == "ready")
    })()
    .unwrap_or(false)
}

/// Something that went wrong in the last day, from the proxy's or an environment's log. Reported,
/// but `up` can still work, so it never makes the setup not ready.
#[derive(Serialize)]
struct Problem {
    source: String,
    detail: String,
    count: usize,
    log: String,
}

/// How far back from the end of each log to look, and how old a log may be to count.
const LOG_TAIL: u64 = 1024 * 1024;
const RECENT: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

fn tail(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let modified = file.metadata().ok()?.modified().ok()?;
    if modified.elapsed().unwrap_or_default() > RECENT {
        return None;
    }
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(LOG_TAIL)))
        .ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Counts each distinct problem once, in the order first seen.
fn tally(problems: &mut Vec<Problem>, source: &str, detail: String, log: &Path) {
    match problems
        .iter_mut()
        .find(|problem| problem.source == source && problem.detail == detail)
    {
        Some(problem) => problem.count += 1,
        None => problems.push(Problem {
            source: source.to_owned(),
            detail,
            count: 1,
            log: log.display().to_string(),
        }),
    }
}

fn recent_problems(state: &Path) -> Vec<Problem> {
    let mut problems = Vec::new();
    let proxy_log = state.join("proxy/proxy.log");
    if let Some(text) = tail(&proxy_log) {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .saturating_sub(RECENT)
            .as_secs_f64();
        for entry in text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        {
            let error = entry["error"].as_str().unwrap_or_default();
            // Withdrawing a route Caddy had already dropped.
            if entry["level"] != "error"
                || entry["ts"].as_f64().is_none_or(|ts| ts < since)
                || error.starts_with("unknown object ID")
            {
                continue;
            }
            let mut detail = entry["msg"].as_str().unwrap_or("error").to_owned();
            if !error.is_empty() {
                detail = format!("{detail}: {}", error.chars().take(160).collect::<String>());
            }
            tally(&mut problems, "proxy", detail, &proxy_log);
        }
    }
    let Ok(Some(registry)) = parcela::registry::Registry::open(state, false) else {
        return problems;
    };
    for workspace in registry.list(None).unwrap_or_default() {
        let log = registry.runtime_path(&workspace).join("log/owner.log");
        let Some(text) = tail(&log) else {
            continue;
        };
        let source = parcela::config::fit_label(
            &format!("{}/{}", workspace.project_id, workspace.label),
            48,
        );
        for note in text
            .lines()
            .filter_map(|line| line.strip_prefix("parcela: "))
        {
            tally(&mut problems, &source, note.trim().to_owned(), &log);
        }
    }
    problems
}

pub struct Probes<'a> {
    pub path: &'a Path,
    pub state: &'a Path,
    pub caddy: Option<&'a str>,
}

/// Report whether `parcela up` can work here. Read-only: creates no state and changes no host setting.
pub fn run(probes: Probes<'_>, out: Output) -> Result<bool> {
    let headless = locate(probes.path)
        .and_then(|source| Manifest::load(&source.join("parcela.yaml")))
        .is_ok_and(|manifest| manifest.services.is_empty());
    let mut checks = vec![];
    if !headless {
        checks.push(backend(
            "caddy",
            probes.caddy.unwrap_or(&parcela::host::caddy()),
            "v2.",
            "install Caddy 2: brew install caddy (or https://caddyserver.com/docs/install)",
        ));
        checks.push(tool("lsof", &["/usr/sbin/lsof", "lsof"]));
    }
    checks.extend([
        tool("ps", &["/bin/ps", "ps"]),
        config(probes.path),
        state(probes.state),
    ]);
    checks.push(if headless {
        match parcela::proxy::check_watcher_compatibility(probes.state) {
            Ok(()) => Check::pass(
                "proxy",
                "no HTTP proxy needed; shared registry is compatible",
            ),
            Err(error) => Check::fail("proxy", format!("{error:#}"), "parcela proxy status"),
        }
    } else {
        proxy(probes.state)
    });
    let ready = checks.iter().all(|check| check.ok);
    let problems = recent_problems(probes.state);
    let notes = if headless {
        vec!["No HTTP services; no proxy or certificate trust is needed.".to_owned()]
    } else {
        vec![
        "HTTPS trust is not checked, and only `parcela trust` changes it. If the browser warns, run `parcela trust` once.".to_owned(),
    ]
    };
    let next = checks
        .iter()
        .find_map(|check| check.fix.clone())
        .unwrap_or_else(|| {
            if running(probes.path, probes.state) {
                "parcela open".into()
            } else {
                "parcela up".into()
            }
        });
    out.emit(
        &serde_json::json!({"ready": ready, "checks": checks, "notes": notes, "recent_problems": problems}),
        |style| {
            let width = checks
                .iter()
                .map(|check| check.name.len())
                .max()
                .unwrap_or(0)
                + 2;
            let mut text = String::new();
            for check in &checks {
                let mark = if check.ok { style.ok() } else { style.bad() };
                text.push_str(&format!("{mark} {:<width$}{}\n", check.name, check.detail));
            }
            for note in &notes {
                text.push_str(&format!("{}\n", style.dim(&format!("· {note}"))));
            }
            if !problems.is_empty() {
                text.push_str("\nIn the logs of the last day:\n");
                for problem in &problems {
                    let count = if problem.count > 1 {
                        format!(" ({}×)", problem.count)
                    } else {
                        String::new()
                    };
                    text.push_str(&format!(
                        "{} {}: {}{count}\n  {}\n",
                        style.bad(),
                        problem.source,
                        problem.detail,
                        style.dim(&problem.log)
                    ));
                }
            }
            text.push_str(&format!("\nNext step: {}\n", style.bold(&next)));
            text
        },
    )?;
    Ok(ready)
}

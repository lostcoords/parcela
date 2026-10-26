//! Parcela's background proxy: a detached `parcela proxy run` that owns Caddy.
//! Liveness is the kernel lock the daemon holds on `owner.lock`; effects go to the recorded birth
//! identity, never a bare PID.
use crate::{
    frontdoor::FrontDoor,
    private::{self, Lock},
    process::Identity,
    router::{self, PROTOCOL, ProxyEvidence, Router, VERSION},
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::OpenOptions,
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const STARTUP: Duration = Duration::from_secs(20);

pub enum Status {
    Stopped,
    Running(Router),
    /// A proxy owns the lock but does not verify, for example while forwarding is missing.
    Unhealthy(anyhow::Error),
}

/// Whether a started proxy was already there or launched by this call.
pub enum Started {
    Already(Router),
    Launched(Router),
}
impl Started {
    pub fn router(&self) -> &Router {
        match self {
            Self::Already(router) | Self::Launched(router) => router,
        }
    }
}

pub fn log_path(state: &Path) -> PathBuf {
    router::proxy_dir(state).join("proxy.log")
}

/// Read-only: never creates the proxy directory or its lock.
pub fn status(state: &Path) -> Result<Status> {
    let lock = router::proxy_dir(state).join("owner.lock");
    if std::fs::symlink_metadata(&lock).is_err() || Lock::try_acquire(&lock)?.is_some() {
        return Ok(Status::Stopped);
    }
    Ok(match router::current(state) {
        Ok(router) => Status::Running(router),
        Err(error) => Status::Unhealthy(error),
    })
}

/// Watcher sessions share the registry with a live proxy, even though they need no routing.
pub fn check_watcher_compatibility(state: &Path) -> Result<()> {
    match status(state)? {
        Status::Stopped => Ok(()),
        Status::Running(router) => {
            ensure!(router.protocol == PROTOCOL,
                "the running proxy cannot read watcher-only sessions; run `parcela proxy restart` with this Parcela first");
            Ok(())
        }
        Status::Unhealthy(error) => Err(error.context(
            "the running proxy cannot be verified; inspect `parcela proxy status` before starting watcher-only services",
        )),
    }
}

/// The recorded owner of the proxy, when its evidence is readable.
pub fn owner(state: &Path) -> Option<(Identity, Identity)> {
    let evidence: ProxyEvidence =
        private::read_json(&router::proxy_dir(state).join("owner.json")).ok()?;
    Some((evidence.daemon, evidence.child))
}

/// The running proxy, starting it on the saved front door when none runs. Used by `up`, `restart`
/// and `run`, so concurrent callers share one proxy.
pub fn ensure_running(state: &Path, host: &str) -> Result<Router> {
    // A proxy that is still starting reads as unhealthy; start() waits for it under the start lock.
    let started = match status(state)? {
        Status::Running(router) => Started::Already(router),
        Status::Stopped | Status::Unhealthy(_) => {
            start(state, "caddy", FrontDoor::load(state)?, false)?
        }
    };
    match &started {
        Started::Already(router) => warn_version(router),
        Started::Launched(router) => eprintln!(
            "parcela: {}",
            started_hint(router, host, trusted(state) == Some(true))
        ),
    }
    Ok(started.router().clone())
}

/// The notice for a proxy `up` just started, naming `parcela trust` only while it is needed.
pub fn started_hint(router: &Router, host: &str, trusted: bool) -> String {
    if trusted {
        return format!("started the proxy on :{}", router.front_door.https);
    }
    format!(
        "started the proxy on :{}; run `parcela trust` to trust {}",
        router.front_door.https,
        router.front_door.url(host)
    )
}

pub fn protocol_warning(router: &Router) -> Option<String> {
    use std::cmp::Ordering;
    match router.protocol.cmp(&PROTOCOL) {
        Ordering::Equal => None,
        Ordering::Less => Some(format!(
            "the running proxy is from Parcela {}, older than this {VERSION}; run `parcela proxy restart` to update it",
            router.version
        )),
        Ordering::Greater => Some(format!(
            "the running proxy is from Parcela {}, newer than this {VERSION}; run `parcela upgrade`",
            router.version
        )),
    }
}
fn warn_version(router: &Router) {
    if let Some(warning) = protocol_warning(router) {
        eprintln!("parcela: {warning}");
    }
}

/// Start the proxy in the background and wait until Caddy answers on its admin socket.
/// `explicit` means the person chose `front_door`, so a different running one is an error.
pub fn start(state: &Path, caddy: &str, front_door: FrontDoor, explicit: bool) -> Result<Started> {
    launch(state, caddy, front_door, explicit, false)
}

/// Written by `stop` so running environments leave a deliberately stopped proxy alone.
const STOPPED: &str = "stopped";

/// Restart a proxy that died while environments still use it, for example after Caddy crashed or
/// the system killed it. Returns false, and starts nothing, after a deliberate `parcela proxy stop`.
pub fn revive(state: &Path) -> Result<bool> {
    if router::proxy_dir(state).join(STOPPED).exists()
        || matches!(status(state)?, Status::Running(_))
    {
        return Ok(false);
    }
    match launch(state, "caddy", FrontDoor::load(state)?, false, true)? {
        Started::Launched(_) => Ok(true),
        Started::Already(_) => Ok(false),
    }
}

fn launch(
    state: &Path,
    caddy: &str,
    front_door: FrontDoor,
    explicit: bool,
    revive: bool,
) -> Result<Started> {
    let directory = router::proxy_dir(state);
    private::directory(&directory)?;
    let _start = Lock::wait_for(&directory.join("start.lock"), STARTUP)?;
    // Checked under the start lock, so a revive never undoes a stop that ran meanwhile.
    if directory.join(STOPPED).exists() {
        ensure!(!revive, "the proxy was stopped on purpose");
        std::fs::remove_file(directory.join(STOPPED))?;
    }
    match status(state)? {
        Status::Running(router) if explicit && router.front_door != front_door => bail!(
            "the proxy is already running with {}; switch with `parcela proxy restart --port {}`",
            router.front_door.describe(),
            front_door.https
        ),
        Status::Running(router) => return Ok(Started::Already(router)),
        Status::Unhealthy(error) => {
            return Err(error.context("the proxy is running but not answering"));
        }
        Status::Stopped => {}
    }
    let log_file = log_path(state);
    let log = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&log_file)?;
    let metadata = log.metadata()?;
    // SAFETY: getuid has no pointers or failure case.
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::getuid() }
            && metadata.mode() & 0o077 == 0,
        "proxy log must be a private user-owned regular file"
    );
    let offset = metadata.len();
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--state-dir")
        .arg(state.canonicalize()?)
        .args(["proxy", "run", "--caddy", caddy])
        .current_dir(&directory)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    command.args(["--port", &front_door.https.to_string()]);
    // SAFETY: setsid is async-signal-safe and touches no memory; a new session keeps terminal
    // signals for the caller away from the daemon.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().context("launch the proxy daemon")?;
    let deadline = Instant::now() + STARTUP;
    loop {
        if child.try_wait()?.is_some() {
            bail!("{}", startup_failure(&log_file, offset));
        }
        if let Status::Running(router) = status(state)?
            && owner(state).is_some_and(|(daemon, _)| daemon.pid == child.id())
        {
            // Reap the daemon if it ever exits while this process lives on, as `up` does.
            std::thread::spawn(move || child.wait());
            return Ok(Started::Launched(router));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "the proxy did not answer within {}s; see {}",
                STARTUP.as_secs(),
                log_file.display()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The daemon's own last error line, which names the cause and its fix.
fn startup_failure(log: &Path, offset: u64) -> String {
    let text = std::fs::read(log).unwrap_or_default();
    let fresh = String::from_utf8_lossy(text.get(offset as usize..).unwrap_or_default());
    let last = fresh
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim_start_matches("parcela: ").to_owned());
    match last {
        Some(line) => format!("the proxy did not start: {line}"),
        None => format!("the proxy exited during startup; see {}", log.display()),
    }
}

/// Stop the proxy. Returns false when none was running.
pub fn stop(state: &Path) -> Result<bool> {
    let directory = router::proxy_dir(state);
    if std::fs::symlink_metadata(&directory).is_err() {
        return Ok(false);
    }
    let _start = Lock::wait_for(&directory.join("start.lock"), STARTUP)?;
    private::write_bytes(&directory.join(STOPPED), b"")?;
    let lock = directory.join("owner.lock");
    let daemon_running =
        std::fs::symlink_metadata(&lock).is_ok() && Lock::try_acquire(&lock)?.is_none();
    let evidence: Option<ProxyEvidence> = private::read_json(&directory.join("owner.json")).ok();
    let Some(evidence) = evidence else {
        ensure!(
            !daemon_running,
            "the proxy lock is held by a process Parcela did not record; stop it by hand"
        );
        return Ok(false);
    };
    if evidence.daemon.alive() {
        // SAFETY: kill takes no pointers; the PID was just matched to its recorded birth identity.
        ensure!(
            unsafe { libc::kill(evidence.daemon.pid as i32, libc::SIGTERM) } == 0,
            "cannot signal the proxy: {}",
            std::io::Error::last_os_error()
        );
    } else if evidence.child.alive() {
        router::stop_caddy(state, &evidence.router, &evidence.child)?;
    } else {
        ensure!(
            !daemon_running,
            "the proxy lock is held by a process Parcela did not record; stop it by hand"
        );
        return Ok(false);
    }
    let deadline = Instant::now() + STARTUP;
    while evidence.daemon.alive() || evidence.child.alive() {
        ensure!(
            Instant::now() < deadline,
            "the proxy did not stop within {}s; see {}",
            STARTUP.as_secs(),
            log_path(state).display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(true)
}

/// The root certificate of the proxy's private CA, for `curl --cacert` and trust stores.
pub fn certificate(state: &Path) -> PathBuf {
    router::proxy_dir(state).join("data/pki/authorities/local/root.crt")
}

/// Whether the system trusts this proxy's CA, or None when it cannot tell. Read-only.
pub fn trusted(state: &Path) -> Option<bool> {
    let certificate = certificate(state);
    if !certificate.is_file() {
        return None;
    }
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("/usr/bin/security");
        command.arg("verify-cert").arg("-c");
        command
    } else {
        let mut command = Command::new("openssl");
        command.arg("verify");
        command
    };
    command
        .arg(&certificate)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()
        .map(|status| status.success())
}

/// `caddy trust` against this proxy: the same private storage and admin socket (from the
/// generated config), so it installs Parcela's CA and never another Caddy's.
pub fn trust_command(state: &Path, caddy: &str) -> Command {
    let directory = router::proxy_dir(state);
    let mut command = Command::new(caddy);
    command
        .arg("trust")
        .arg("--config")
        .arg(directory.join("caddy.json"))
        .current_dir(&directory)
        .env("XDG_DATA_HOME", directory.join("data"))
        .env("XDG_CONFIG_HOME", directory.join("config"));
    command
}

pub fn install_trust(state: &Path, caddy: &str) -> Result<()> {
    let certificate = certificate(state);
    eprintln!(
        "Adding Parcela's certificate authority to the system trust store. It may ask for your password.\n  certificate  {}\n  undo         {caddy} untrust --cert {}",
        certificate.display(),
        certificate.display(),
    );
    let status = trust_command(state, caddy)
        .status()
        .with_context(|| format!("run {caddy} trust"))?;
    anyhow::ensure!(status.success(), "{caddy} trust failed ({status})");
    eprintln!("Trusted. Restart the browser if it still warns.");
    Ok(())
}

/// The first `up` in a terminal trusts a new CA, behind the system's own password prompt, so
/// HTTPS works without a separate `parcela trust`. Asked once per CA, never without a
/// terminal (agents, scripts, CI), and never with `PARCELA_NO_TRUST=1`.
pub fn offer_trust(state: &Path) {
    use std::io::IsTerminal;
    if std::env::var("PARCELA_NO_TRUST").is_ok_and(|value| !value.is_empty() && value != "0")
        || !std::io::stdin().is_terminal()
        || !std::io::stderr().is_terminal()
    {
        return;
    }
    let certificate = certificate(state);
    let asked = state.join("proxy/trust-asked.crt");
    let Ok(current) = std::fs::read(&certificate) else {
        return;
    };
    if std::fs::read(&asked).is_ok_and(|previous| previous == current)
        || trusted(state) != Some(false)
    {
        return;
    }
    // Recorded first, so a declined prompt is not asked again on every `up`.
    if crate::private::write_bytes(&asked, &current).is_err() {
        return;
    }
    if let Err(error) = install_trust(state, "caddy") {
        eprintln!(
            "parcela: the certificate authority is not trusted ({error:#}); run `parcela trust` to try again"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, path::Path};

    #[test]
    fn trust_runs_caddy_trust_with_the_proxys_private_config_and_storage() {
        let command = super::trust_command(Path::new("/state"), "/bin/caddy");
        assert_eq!(command.get_program(), "/bin/caddy");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["trust", "--config", "/state/proxy/caddy.json"]
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/state/proxy")));
        let envs: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
        assert_eq!(
            envs,
            [
                (
                    OsStr::new("XDG_CONFIG_HOME"),
                    Some(OsStr::new("/state/proxy/config"))
                ),
                (
                    OsStr::new("XDG_DATA_HOME"),
                    Some(OsStr::new("/state/proxy/data"))
                ),
            ]
        );
        assert_eq!(
            super::certificate(Path::new("/state")),
            Path::new("/state/proxy/data/pki/authorities/local/root.crt")
        );
    }
}

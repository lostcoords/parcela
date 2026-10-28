use anyhow::{Context, Result, ensure};
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    flag,
};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub struct Signals {
    value: Arc<AtomicUsize>,
    registrations: Vec<signal_hook::SigId>,
}
impl Signals {
    pub fn new() -> Result<Self> {
        let value = Arc::new(AtomicUsize::new(0));
        let mut registrations = Vec::new();
        for signal in [SIGINT, SIGTERM, SIGHUP] {
            registrations.push(flag::register_usize(
                signal,
                value.clone(),
                signal as usize,
            )?);
        }
        Ok(Self {
            value,
            registrations,
        })
    }
    pub fn received(&self) -> usize {
        self.value.load(Ordering::Relaxed)
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        for id in &self.registrations {
            signal_hook::low_level::unregister(*id);
        }
    }
}

pub struct OwnedChild {
    child: Child,
    ended: bool,
}
impl OwnedChild {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        Self::spawn_with_stderr(command, Stdio::inherit())
    }
    pub fn spawn_with_stderr(command: &mut Command, stderr: impl Into<Stdio>) -> Result<Self> {
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(stderr);
        Ok(Self {
            child: command.spawn().context("launch foreground child")?,
            ended: false,
        })
    }
    /// Starts `command` in its own process group with stdout and stderr in `log`.
    pub fn spawn_logged(command: &mut Command, log: std::fs::File) -> Result<Self> {
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        Ok(Self {
            child: command
                .spawn()
                .with_context(|| format!("launch {}", command.get_program().to_string_lossy()))?,
            ended: false,
        })
    }
    /// Takes over a child spawned elsewhere; dropping it stops its process group.
    pub fn from_child(child: Child) -> Self {
        Self {
            child,
            ended: false,
        }
    }
    pub fn detach(mut self) {
        self.ended = true;
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn poll(&mut self) -> Result<Option<ExitStatus>> {
        let result = self.child.try_wait()?;
        self.ended |= result.is_some();
        Ok(result)
    }
    pub fn signal(&mut self, signal: i32, group: bool) -> Result<()> {
        ensure!(self.poll()?.is_none(), "child already exited");
        let pid = self.child.id() as i32;
        // SAFETY: an unreaped retained live child cannot have its PID reused.
        let result = unsafe { libc::kill(if group { -pid } else { pid }, signal) };
        ensure!(
            result == 0,
            "cannot signal retained child: {}",
            std::io::Error::last_os_error()
        );
        Ok(())
    }
    /// SIGKILL to the whole group and reap at once, for a child nothing waits on.
    pub fn kill_now(mut self) {
        if !self.ended && self.poll().is_ok_and(|status| status.is_none()) {
            // SAFETY: the unreaped direct child still owns its PID/group identity.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.wait();
        }
        self.ended = true;
    }
    pub fn stop(&mut self, signal: i32) -> Result<ExitStatus> {
        if let Some(status) = self.poll()? {
            return Ok(status);
        }
        // SAFETY: the unreaped direct child still owns its PID/group identity.
        unsafe {
            libc::kill(-(self.child.id() as i32), signal);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.poll()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                // SAFETY: poll just confirmed that our unreaped child is alive.
                unsafe {
                    libc::kill(-(self.child.id() as i32), libc::SIGKILL);
                }
                let status = self.child.wait()?;
                self.ended = true;
                return Ok(status);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.stop(SIGTERM);
        }
    }
}
pub fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(SIGTERM))
}

/// Where `program` resolves: itself when it is a path, else the first match on PATH.
pub fn on_path(program: &str) -> Option<std::path::PathBuf> {
    if program.contains('/') {
        return std::path::Path::new(program)
            .is_file()
            .then(|| program.into());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

/// Capture a bounded diagnostic command with a deadline and a retained child identity.
pub fn capture(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
) -> Result<std::process::Output> {
    use std::{io::Read, os::fd::AsRawFd};
    command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = OwnedChild {
        child: command.spawn().context("launch backend control command")?,
        ended: false,
    };
    let mut stdout = child.child.stdout.take().context("missing stdout pipe")?;
    let mut stderr = child.child.stderr.take().context("missing stderr pipe")?;
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        // SAFETY: both descriptors remain owned by their pipe handles through capture.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        ensure!(
            flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == 0,
            "cannot bound backend output reads"
        );
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    let deadline = Instant::now() + timeout;
    let mut exited = None;
    let status = loop {
        for (pipe, bytes) in [
            (&mut stdout as &mut dyn Read, &mut out),
            (&mut stderr as &mut dyn Read, &mut err),
        ] {
            let mut buffer = [0u8; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        ensure!(bytes.len() + count <= limit, "backend response too large");
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error.into()),
                }
            }
        }
        if let Some(status) = exited {
            break status;
        }
        exited = child.poll()?;
        if exited.is_some() {
            continue;
        }
        ensure!(
            Instant::now() < deadline,
            "backend control command timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(std::process::Output {
        status,
        stdout: out,
        stderr: err,
    })
}
pub fn version(binary: &str, argument: &str) -> Result<String> {
    let output = capture(Command::new(binary).arg(argument), Duration::from_secs(3), 65536)
        .with_context(|| format!("{binary} unavailable; supply an explicit backend binary (ordinary commands never install tools)"))?;
    ensure!(
        output.status.success(),
        "backend version probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
/// Verify the listener itself, not merely an HTTP answer at an allocated port.
pub fn listener_identity(port: u16, roots: &[u32]) -> Result<Vec<u32>> {
    Listeners::inspect([port])?.owned(port, roots)
}

/// Every LISTEN socket on a set of ports, from one lsof call.
/// lsof is a diagnosed prerequisite for this initial Unix TCP implementation.
#[derive(Debug)]
pub struct Listeners(Vec<Listener>);
#[derive(Debug)]
struct Listener {
    pid: u32,
    port: u16,
    name: String,
    /// The loopback address that reaches this socket, or None outside loopback and wildcards.
    loopback: Option<IpAddr>,
}
impl Listeners {
    pub fn inspect(ports: impl IntoIterator<Item = u16>) -> Result<Self> {
        // One -i per port, which lsof ORs: lsof 4.99.7 aborts on a comma-separated port list.
        let ports: Vec<_> = ports
            .into_iter()
            .map(|port| format!("-iTCP:{port}"))
            .collect();
        if ports.is_empty() {
            return Ok(Self(Vec::new()));
        }
        let output = Command::new(lsof())
            .arg("-nP")
            .args(&ports)
            .args(["-sTCP:LISTEN", "-Fptn"])
            .output()
            .context("lsof is required to verify TCP listener ownership")?;
        if output.stdout.is_empty() && output.status.code() == Some(1) {
            return Ok(Self(Vec::new()));
        }
        ensure!(output.status.success(), "lsof listener inspection failed");
        Self::parse(&String::from_utf8(output.stdout)?)
    }
    fn parse(text: &str) -> Result<Self> {
        let mut pid = 0;
        let mut family = None;
        let mut found = Vec::new();
        for line in text.lines() {
            if let Some(value) = line.strip_prefix('p') {
                pid = value.parse()?;
            } else if line.starts_with('f') {
                family = None;
            } else if let Some(value) = line.strip_prefix('t') {
                family = Some(value);
            } else if let Some(name) = line.strip_prefix('n') {
                let (host, port) = name
                    .rsplit_once(':')
                    .with_context(|| format!("unexpected lsof listener {name}"))?;
                let loopback = match (host, family) {
                    ("*", Some("IPv4")) => Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
                    ("*", Some("IPv6")) => Some(IpAddr::V6(Ipv6Addr::LOCALHOST)),
                    _ => match host.trim_start_matches('[').trim_end_matches(']').parse() {
                        Ok(IpAddr::V4(Ipv4Addr::LOCALHOST | Ipv4Addr::UNSPECIFIED)) => {
                            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
                        }
                        Ok(IpAddr::V6(Ipv6Addr::LOCALHOST | Ipv6Addr::UNSPECIFIED)) => {
                            Some(IpAddr::V6(Ipv6Addr::LOCALHOST))
                        }
                        _ => None,
                    },
                };
                found.push(Listener {
                    pid,
                    port: port
                        .parse()
                        .with_context(|| format!("unexpected lsof listener {name}"))?,
                    name: name.into(),
                    loopback,
                });
            }
        }
        Ok(Self(found))
    }
    /// The sockets on `port`. Each must be loopback (`127.0.0.1`, `::1`) or a wildcard
    /// (`0.0.0.0`, `::`, `*`). Wildcard binds are accepted without a warning: the application
    /// chose that exposure, and Parcela still routes only through loopback.
    pub fn on(&self, port: u16) -> Result<Vec<(u32, IpAddr)>> {
        self.0
            .iter()
            .filter(|listener| listener.port == port)
            .map(|listener| {
                let loopback = listener.loopback.with_context(|| {
                    format!(
                        "backend must listen on loopback or a wildcard at port {port}, found {}",
                        listener.name
                    )
                })?;
                Ok((listener.pid, loopback))
            })
            .collect()
    }
    /// The live listeners on `port`, which must all belong to the process tree of one of `roots`.
    pub fn owned(&self, port: u16, roots: &[u32]) -> Result<Vec<u32>> {
        let mut found = Vec::new();
        for (pid, _) in self.on(port)? {
            let Some(identity) = Identity::read(pid)? else {
                continue;
            };
            let owned = descendant(pid, roots)?;
            if !identity.alive() {
                continue;
            }
            ensure!(
                owned,
                "allocated endpoint was acquired by a foreign listener; refusing route registration"
            );
            found.push(pid);
        }
        found.sort_unstable();
        found.dedup();
        Ok(found)
    }
    /// The loopback address that reaches the backend on `port`, preferring IPv4.
    pub fn address(&self, port: u16) -> Result<SocketAddr> {
        let listeners = self.on(port)?;
        let ip = if listeners.is_empty() || listeners.iter().any(|(_, ip)| ip.is_ipv4()) {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        };
        Ok(SocketAddr::new(ip, port))
    }
}

/// Who listens on `port`, for messages: "caddy (pid 123)", or "another process" when lsof
/// cannot see it.
pub fn port_holder(port: u16) -> String {
    let Ok(output) = Command::new(lsof())
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-Fpc"])
        .output()
    else {
        return "another process".into();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let pid = text.lines().find_map(|line| line.strip_prefix('p'));
    let command = text.lines().find_map(|line| line.strip_prefix('c'));
    match (pid, command) {
        (Some(pid), Some(command)) => format!("{command} (pid {pid})"),
        _ => "another process".into(),
    }
}
fn lsof() -> &'static str {
    if std::path::Path::new("/usr/sbin/lsof").exists() {
        "/usr/sbin/lsof"
    } else {
        "lsof"
    }
}

fn descendant(mut pid: u32, roots: &[u32]) -> Result<bool> {
    for _ in 0..32 {
        if roots.contains(&pid) {
            return Ok(true);
        }
        if pid <= 1 {
            return Ok(false);
        }
        let output = Command::new("ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .output()?;
        let parent = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u32>();
        let Ok(parent) = parent else {
            return Ok(false);
        };
        pid = parent;
    }
    Ok(false)
}

/// Signals the process, or with `group` its process group, only while `identity` still names
/// it: the pid is re-read just before the signal. A pid from disk is never signaled on its own.
pub fn signal_verified(identity: &Identity, signal: i32, group: bool) -> Result<bool> {
    if Identity::read(identity.pid)?.as_ref() != Some(identity) {
        return Ok(false);
    }
    let pid = identity.pid as i32;
    // SAFETY: kill has no pointers; the identity check above narrows pid reuse to the gap
    // between two system calls.
    Ok(unsafe { libc::kill(if group { -pid } else { pid }, signal) } == 0)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    pub pid: u32,
    birth: u64,
    boot: String,
}
impl Identity {
    pub fn read(pid: u32) -> Result<Option<Self>> {
        let Some(birth) = birth(pid)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            pid,
            birth,
            boot: boot_id()?,
        }))
    }
    pub fn alive(&self) -> bool {
        Self::read(self.pid).is_ok_and(|current| current.as_ref() == Some(self))
    }
    pub(crate) fn previous_boot(&self) -> Result<bool> {
        Ok(self.boot != boot_id()?)
    }
}
#[cfg(target_os = "linux")]
fn boot_id() -> Result<String> {
    Ok(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .to_owned())
}
#[cfg(target_os = "linux")]
fn birth(pid: u32) -> Result<Option<u64>> {
    let text = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let fields: Vec<_> = text
        .rsplit_once(')')
        .context("invalid process stat")?
        .1
        .split_whitespace()
        .collect();
    if fields
        .first()
        .is_none_or(|state| ["Z", "X"].contains(state))
    {
        return Ok(None);
    }
    Ok(Some(
        fields.get(19).context("missing process birth")?.parse()?,
    ))
}
#[cfg(target_os = "macos")]
fn boot_id() -> Result<String> {
    let name = std::ffi::CString::new("kern.bootsessionuuid")?;
    let mut bytes = [0u8; 128];
    let mut size = bytes.len();
    // SAFETY: the name and fixed output buffer remain owned through sysctlbyname.
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    ensure!(
        result == 0 && size <= bytes.len(),
        "cannot identify host boot"
    );
    Ok(std::str::from_utf8(&bytes[..size])?
        .trim_end_matches('\0')
        .to_owned())
}
#[cfg(target_os = "macos")]
fn birth(pid: u32) -> Result<Option<u64>> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    // SAFETY: PROC_PIDTBSDINFO (3) fills the supplied proc_bsdinfo buffer.
    let result =
        unsafe { libc::proc_pidinfo(pid as i32, 3, 0, info.as_mut_ptr().cast(), size as i32) };
    if result == 0 {
        return Ok(None);
    }
    ensure!(result as usize == size, "incomplete process birth identity");
    // SAFETY: proc_pidinfo reported that it initialized the entire buffer.
    let info = unsafe { info.assume_init() };
    if info.pbi_status == 5 {
        return Ok(None);
    }
    Ok(Some(
        info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
    ))
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn boot_id() -> Result<String> {
    anyhow::bail!("process identity requires macOS or Linux")
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn birth(_: u32) -> Result<Option<u64>> {
    anyhow::bail!("process identity requires macOS or Linux")
}

/// Every live process descending from a live root, the roots included. A pid can exit and be
/// reused between listing it and reading its identity, so each identity read is confirmed by a
/// second listing that still shows it under a parent in the tree.
pub fn tree(roots: &[Identity]) -> Result<Vec<Identity>> {
    let roots: Vec<&Identity> = roots.iter().filter(|root| root.alive()).collect();
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    let root_pids: std::collections::BTreeSet<u32> = roots.iter().map(|root| root.pid).collect();
    let pids = descendants(&parents()?, &root_pids);
    let identities: Vec<Identity> = pids
        .iter()
        .filter_map(|pid| Identity::read(*pid).transpose())
        .collect::<Result<_>>()?;
    let confirmed = parents()?;
    let members = descendants(&confirmed, &root_pids);
    Ok(identities
        .into_iter()
        .filter(|identity| members.contains(&identity.pid) && identity.alive())
        .collect())
}

/// Every process's parent, from one `ps` listing.
fn parents() -> Result<Vec<(u32, u32)>> {
    let output = capture(
        Command::new("ps").args(["-A", "-o", "pid=,ppid="]),
        Duration::from_secs(2),
        1_000_000,
    )?;
    ensure!(
        output.status.success(),
        "cannot inspect supervised process tree"
    );
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .collect())
}

/// `roots` and every pid descending from one of them in `pairs`.
fn descendants(
    pairs: &[(u32, u32)],
    roots: &std::collections::BTreeSet<u32>,
) -> std::collections::BTreeSet<u32> {
    let mut pids = roots.clone();
    loop {
        let before = pids.len();
        for (pid, parent) in pairs {
            if pids.contains(parent) {
                pids.insert(*pid);
            }
        }
        if pids.len() == before {
            return pids;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(text: &str, port: u16) -> Result<Vec<(u32, IpAddr)>> {
        Listeners::parse(text)?.on(port)
    }
    fn loopbacks(text: &str, port: u16) -> Vec<(u32, String)> {
        on(text, port)
            .unwrap()
            .into_iter()
            .map(|(pid, loopback)| (pid, loopback.to_string()))
            .collect()
    }

    #[test]
    fn ipv4_loopback_reaches_ipv4_loopback() {
        assert_eq!(
            loopbacks("p10\nf3\ntIPv4\nn127.0.0.1:3000\n", 3000),
            [(10, "127.0.0.1".into())]
        );
    }
    #[test]
    fn ipv6_loopback_reaches_ipv6_loopback() {
        assert_eq!(
            loopbacks("p10\nf3\ntIPv6\nn[::1]:3000\n", 3000),
            [(10, "::1".into())]
        );
    }
    #[test]
    fn localhost_bind_reports_both_families() {
        assert_eq!(
            loopbacks(
                "p10\nf3\ntIPv4\nn127.0.0.1:3000\nf4\ntIPv6\nn[::1]:3000\n",
                3000
            ),
            [(10, "127.0.0.1".into()), (10, "::1".into())]
        );
    }
    #[test]
    fn macos_wildcards_take_their_family_from_the_type_field() {
        assert_eq!(
            loopbacks("p10\nf3\ntIPv4\nn*:3000\np11\nf5\ntIPv6\nn*:3000\n", 3000),
            [(10, "127.0.0.1".into()), (11, "::1".into())]
        );
    }
    #[test]
    fn literal_wildcards_are_accepted() {
        assert_eq!(
            loopbacks(
                "p10\nf3\ntIPv4\nn0.0.0.0:3000\nf4\ntIPv6\nn[::]:3000\n",
                3000
            ),
            [(10, "127.0.0.1".into()), (10, "::1".into())]
        );
    }
    #[test]
    fn non_loopback_addresses_are_refused() {
        for name in ["192.168.1.20:3000", "[fe80::1]:3000", "127.0.0.2:3000"] {
            assert_eq!(
                on(&format!("p10\nf3\ntIPv4\nn{name}\n"), 3000)
                    .unwrap_err()
                    .to_string(),
                format!("backend must listen on loopback or a wildcard at port 3000, found {name}")
            );
        }
    }
    #[test]
    fn one_snapshot_answers_each_port_on_its_own() {
        let text = "p10\nf3\ntIPv4\nn127.0.0.1:3000\np11\nf4\ntIPv4\nn192.168.1.20:3001\nf5\ntIPv6\nn[::1]:3002\n";
        assert_eq!(loopbacks(text, 3000), [(10, "127.0.0.1".into())]);
        assert_eq!(loopbacks(text, 3002), [(11, "::1".into())]);
        assert!(loopbacks(text, 3003).is_empty());
        assert_eq!(
            on(text, 3001).unwrap_err().to_string(),
            "backend must listen on loopback or a wildcard at port 3001, found 192.168.1.20:3001"
        );
        let snapshot = Listeners::parse(text).unwrap();
        assert_eq!(snapshot.address(3002).unwrap().to_string(), "[::1]:3002");
        assert_eq!(
            snapshot.address(3003).unwrap().to_string(),
            "127.0.0.1:3003"
        );
    }
    #[test]
    fn untyped_wildcard_is_refused() {
        assert_eq!(
            on("p10\nf3\nn*:3000\n", 3000).unwrap_err().to_string(),
            "backend must listen on loopback or a wildcard at port 3000, found *:3000"
        );
    }
}

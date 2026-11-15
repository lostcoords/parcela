//! `parcela relay install`: URLs without a port. The system's service manager owns a real socket
//! on 127.0.0.1:443 and relays each connection, as the user, to the proxy's port: launchd starts
//! `/usr/bin/nc` on macOS, systemd starts `systemd-socket-proxyd` on Linux. No Parcela code runs
//! as root, and a busy 443 is a bind error, never a redirect that hides another program's
//! listener. A free 80 is relayed to the same port, where the proxy redirects plain HTTP to HTTPS.
//!
//! `parcela relay install --lan` is separate and macOS only: a pf redirect on one interface, from
//! 443 and a free 80 on its address to the proxy's port on the same address. Caddy listens there
//! only while a run is LAN-shared, and only for shared names, so the redirect never reaches
//! loopback routes. The rules load into an anchor under Apple's `com.apple/*`, so /etc/pf.conf
//! stays untouched.
use crate::proxy::frontdoor::FrontDoor;
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{net::Ipv4Addr, path::Path, process::Command, time::Duration};

pub const LABEL: &str = "org.parcela.https";
const PLIST: &str = "/Library/LaunchDaemons/org.parcela.https.plist";
const UNITS: &str = "/etc/systemd/system";
/// systemd units: a socket and its relay service per relayed port.
const HTTPS_UNIT: &str = "parcela-https";
const HTTP_UNIT: &str = "parcela-http";
const LAN_LABEL: &str = "org.parcela.lan";
const LAN_PLIST: &str = "/Library/LaunchDaemons/org.parcela.lan.plist";
const PF_RULES: &str = "/etc/pf.anchors/org.parcela.lan";
/// The stock /etc/pf.conf evaluates every `com.apple/*` anchor's redirects.
const PF_ANCHOR: &str = "com.apple/parcela";

fn user() -> Result<String> {
    // SAFETY: getpwuid returns a pointer into static storage or null; it is read at once.
    let name = unsafe {
        let entry = libc::getpwuid(libc::getuid());
        ensure!(!entry.is_null(), "cannot find the current user");
        std::ffi::CStr::from_ptr((*entry).pw_name)
    };
    let name = name.to_str().context("non-UTF-8 user name")?;
    ensure!(
        plain_name(name),
        "unsupported user name for the relay: {name:?}"
    );
    Ok(name.to_owned())
}

/// A user or interface name that goes into a plist or rule file as is: no markup or quoting.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn plist(user: &str, target: u16, http: bool) -> String {
    let socket = |port: u16| {
        format!(
            r#"<dict>
    <key>SockNodeName</key><string>127.0.0.1</string>
    <key>SockServiceName</key><string>{port}</string>
    <key>SockType</key><string>stream</string>
  </dict>"#
        )
    };
    let http = if http {
        format!("\n  <key>HTTP</key>{}", socket(80))
    } else {
        String::new()
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>UserName</key><string>{user}</string>
  <key>ProgramArguments</key>
  <array><string>/usr/bin/nc</string><string>127.0.0.1</string><string>{target}</string></array>
  <key>inetdCompatibility</key><dict><key>Wait</key><false/></dict>
  <key>Sockets</key>
  <dict>
  <key>Listeners</key>{}{http}
  </dict>
</dict>
</plist>
"#,
        socket(443)
    )
}

/// `systemd-socket-proxyd` units relaying `port` on loopback to `target`, as `user`.
fn units(name: &str, user: &str, port: u16, target: u16, proxyd: &str) -> (String, String) {
    let socket = format!(
        "[Unit]\nDescription=Parcela relay for 127.0.0.1:{port}\n\n[Socket]\nListenStream=127.0.0.1:{port}\n\n[Install]\nWantedBy=sockets.target\n"
    );
    let service = format!(
        "[Unit]\nDescription=Parcela relay for 127.0.0.1:{port} to :{target}\nRequires={name}.socket\nAfter={name}.socket\n\n[Service]\nUser={user}\nExecStart={proxyd} 127.0.0.1:{target}\nNoNewPrivileges=yes\nPrivateTmp=yes\n"
    );
    (socket, service)
}

fn proxyd() -> Result<&'static str> {
    [
        "/usr/lib/systemd/systemd-socket-proxyd",
        "/lib/systemd/systemd-socket-proxyd",
    ]
    .into_iter()
    .find(|path| Path::new(path).exists())
    .context("the relay needs systemd-socket-proxyd, part of systemd")
}

/// Where the relay is installed, for messages.
pub fn location() -> String {
    if cfg!(target_os = "macos") {
        PLIST.into()
    } else {
        format!("{UNITS}/{HTTPS_UNIT}.socket")
    }
}

/// Whether the installed relay already owns port 80.
fn relays_http() -> bool {
    if cfg!(target_os = "macos") {
        std::fs::read_to_string(PLIST).is_ok_and(|plist| takes_http(&plist))
    } else {
        Path::new(&format!("{UNITS}/{HTTP_UNIT}.socket")).exists()
    }
}

/// The proxy port the installed relay forwards to, read from its service file. It is fixed at
/// install, and only the state directory that installed it has its proxy there.
pub fn target() -> Option<u16> {
    if cfg!(target_os = "macos") {
        let plist = std::fs::read_to_string(PLIST).ok()?;
        target_in(&plist, "<string>127.0.0.1</string><string>", '<')
    } else {
        let unit = std::fs::read_to_string(format!("{UNITS}/{HTTPS_UNIT}.service")).ok()?;
        target_in(&unit, "-proxyd 127.0.0.1:", '\n')
    }
}

fn target_in(text: &str, before: &str, end: char) -> Option<u16> {
    let (_, rest) = text.split_once(before)?;
    rest.split(end).next()?.trim().parse().ok()
}

/// The installed relay: whether it exists and whether it also takes port 80.
pub fn status() -> (bool, bool) {
    let installed = installed();
    (installed, installed && relays_http())
}

fn takes_http(plist: &str) -> bool {
    plist.contains("<string>80</string>")
}

/// Who holds `port` on loopback, if anyone.
fn busy(port: u16) -> Option<String> {
    let holder = crate::process::port_holder(port);
    (holder != "another process" || std::net::TcpStream::connect(("127.0.0.1", port)).is_ok())
        .then_some(holder)
}

pub fn installed() -> bool {
    Path::new(&location()).exists()
}

fn sudo(script: &str, args: &[&str]) -> Result<()> {
    let status = Command::new("/usr/bin/sudo")
        .args(["/bin/sh", "-c", script, "sh"])
        .args(args)
        .status()
        .context("run sudo")?;
    ensure!(status.success(), "sudo failed ({status}); nothing changed");
    Ok(())
}

/// Install the relay to `front_door.https` and return the front door that uses it.
pub fn install(state: &Path, front_door: FrontDoor) -> Result<FrontDoor> {
    ensure!(
        cfg!(any(target_os = "macos", target_os = "linux")),
        "`parcela relay install` supports macOS and Linux"
    );
    ensure!(front_door.https != 443, "the proxy already listens on 443");
    if !installed()
        && let Some(holder) = busy(443)
    {
        bail!("port 443 is already in use by {holder}; Parcela never takes it over");
    }
    // Port 80 is optional: a busy one only loses the http:// redirect.
    let http = if installed() && relays_http() {
        true
    } else if let Some(holder) = busy(80) {
        crate::paint::Paint::note(&format!(
            "port 80 is in use by {holder}; http:// URLs will not redirect"
        ));
        false
    } else {
        true
    };
    let user = user()?;
    eprintln!(
        "Installing {} so URLs need no port. It may ask for your password.\n  undo  parcela relay uninstall",
        location()
    );
    let staging = state.join("proxy/relay");
    crate::state::private::directory(&staging)?;
    let staged = |name: &str, text: &str| -> Result<String> {
        let file = staging.join(name);
        crate::state::private::write_bytes(&file, text.as_bytes())?;
        Ok(file.to_str().context("non-UTF-8 state path")?.to_owned())
    };
    let result = if cfg!(target_os = "macos") {
        let file = staged(
            "org.parcela.https.plist",
            &plist(&user, front_door.https, http),
        )?;
        sudo(
            r#"install -m 644 -o root -g wheel "$1" "$2" && { launchctl bootout system/"$3" 2>/dev/null; launchctl bootstrap system "$2"; }"#,
            &[&file, PLIST, LABEL],
        )
    } else {
        let proxyd = proxyd()?;
        let mut files = Vec::new();
        let mut names = vec![HTTPS_UNIT];
        if http {
            names.push(HTTP_UNIT);
        }
        for name in &names {
            let port = if *name == HTTPS_UNIT { 443 } else { 80 };
            let (socket, service) = units(name, &user, port, front_door.https, proxyd);
            files.push(staged(&format!("{name}.socket"), &socket)?);
            files.push(staged(&format!("{name}.service"), &service)?);
        }
        let sockets: Vec<String> = names.iter().map(|name| format!("{name}.socket")).collect();
        let mut args: Vec<&str> = vec![UNITS];
        args.extend(files.iter().map(String::as_str));
        sudo(
            &format!(
                r#"dir="$1"; shift; install -m 644 -o root -g root "$@" "$dir"/ && systemctl daemon-reload && systemctl enable --now {}"#,
                sockets.join(" ")
            ),
            &args,
        )
    };
    let _ = std::fs::remove_dir_all(&staging);
    result?;
    Ok(FrontDoor {
        relay: true,
        ..front_door
    })
}

/// Remove the relay and the LAN redirect, and return the front door without the relay.
pub fn uninstall(front_door: FrontDoor) -> Result<FrontDoor> {
    if lan_installed() {
        uninstall_lan()?;
    }
    if cfg!(target_os = "macos") && installed() {
        sudo(
            r#"launchctl bootout system/"$1" 2>/dev/null; rm -f "$2""#,
            &[LABEL, PLIST],
        )?;
    } else if cfg!(target_os = "linux") && installed() {
        sudo(
            r#"cd "$1" && systemctl disable --now parcela-https.socket parcela-http.socket 2>/dev/null; rm -f parcela-https.socket parcela-https.service parcela-http.socket parcela-http.service && systemctl daemon-reload"#,
            &[UNITS],
        )?;
    }
    Ok(FrontDoor {
        relay: false,
        ..front_door
    })
}

/// The installed LAN redirect: connections from other devices to 443, and to 80 when `http`, on
/// `interface`'s address go to `target` on that address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LanRedirect {
    pub interface: String,
    pub target: u16,
    pub http: bool,
}

impl LanRedirect {
    fn ports(&self) -> impl Iterator<Item = u16> {
        [443].into_iter().chain(self.http.then_some(80))
    }

    /// Whether other devices reach the proxy on `https` at `ip` through port 443.
    pub fn reaches(&self, ip: Ipv4Addr, https: u16, interfaces: &[(String, Ipv4Addr)]) -> bool {
        self.target == https
            && interfaces
                .iter()
                .any(|(name, address)| *name == self.interface && *address == ip)
    }
}

/// Plain `rdr`, not `rdr pass`, so the person's own pf filter rules still apply.
fn pf_rules(redirect: &LanRedirect) -> String {
    let LanRedirect {
        interface, target, ..
    } = redirect;
    redirect
        .ports()
        .map(|port| {
            format!(
                "rdr on {interface} inet proto tcp from any to ({interface}) port {port} -> ({interface}) port {target}\n"
            )
        })
        .collect()
}

fn parse_pf(rules: &str) -> Option<LanRedirect> {
    let interface = rules.strip_prefix("rdr on ")?.split(' ').next()?;
    Some(LanRedirect {
        interface: interface.into(),
        target: target_in(rules, &format!("port 443 -> ({interface}) port "), '\n')?,
        http: rules.contains(" port 80 -> "),
    })
}

/// Loads [`PF_RULES`] at boot. `-E` takes a reference on pf, which the stock system leaves off.
fn lan_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LAN_LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>/sbin/pfctl</string><string>-E</string><string>-a</string><string>{PF_ANCHOR}</string><string>-f</string><string>{PF_RULES}</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#
    )
}

/// The installed LAN redirect, read from its rule file.
pub fn lan() -> Option<LanRedirect> {
    parse_pf(&std::fs::read_to_string(PF_RULES).ok()?)
}

fn lan_installed() -> bool {
    Path::new(PF_RULES).exists() || Path::new(LAN_PLIST).exists()
}

/// Whether pf accepts `rules` in the anchor, without loading them; the error carries pfctl's own.
fn check_pf(rules: &str) -> Result<()> {
    let output = Command::new("/sbin/pfctl")
        .args(["-n", "-a", PF_ANCHOR, "-f", rules])
        .output()
        .context("run pfctl")?;
    ensure!(
        output.status.success(),
        "pf rejected the LAN redirect; nothing changed\n{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

/// Whether something already accepts connections on `ip`:`port`. A redirect would hide it.
fn answers(ip: Ipv4Addr, port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&(ip, port).into(), Duration::from_millis(300)).is_ok()
}

/// Install the LAN redirect on `interface`, or on the one with the detected LAN address, to
/// `front_door.https`.
pub fn install_lan(
    state: &Path,
    front_door: FrontDoor,
    interface: Option<&str>,
) -> Result<LanRedirect> {
    ensure!(
        cfg!(target_os = "macos"),
        "`parcela relay install --lan` supports only macOS; elsewhere .local URLs keep the proxy's port"
    );
    ensure!(front_door.https != 443, "the proxy already listens on 443");
    let interfaces = crate::share::interfaces();
    let interface = match interface {
        Some(name) => name.to_owned(),
        None => {
            let ip = crate::share::lan_address()?;
            interfaces
                .iter()
                .find(|(_, address)| *address == ip)
                .map(|(name, _)| name.clone())
                .with_context(|| format!("no interface holds {ip}; pick one with --interface"))?
        }
    };
    ensure!(
        plain_name(&interface),
        "unsupported interface name: {interface:?}"
    );
    let ip = interfaces
        .iter()
        .find(|(name, address)| *name == interface && crate::share::private_network(*address))
        .map(|(_, address)| *address)
        .with_context(|| {
            format!("{interface} has no private IPv4 address; pick another with --interface")
        })?;
    if answers(ip, 443) {
        bail!("something already answers on {ip}:443; Parcela never takes it over");
    }
    let http = !answers(ip, 80);
    if !http {
        crate::paint::Paint::note(&format!(
            "something answers on {ip}:80; http:// .local URLs will not redirect"
        ));
    }
    let redirect = LanRedirect {
        interface,
        target: front_door.https,
        http,
    };
    eprintln!(
        "Installing {PF_RULES} so shared .local URLs need no port. It may ask for your password.\n  undo  parcela relay uninstall"
    );
    let staging = state.join("proxy/relay");
    crate::state::private::directory(&staging)?;
    let staged = |name: &str, text: &str| -> Result<String> {
        let file = staging.join(name);
        crate::state::private::write_bytes(&file, text.as_bytes())?;
        Ok(file.to_str().context("non-UTF-8 state path")?.to_owned())
    };
    let result = (|| {
        let rules = staged("org.parcela.lan.pf", &pf_rules(&redirect))?;
        let plist = staged("org.parcela.lan.plist", &lan_plist())?;
        check_pf(&rules)?;
        // A rules file left behind makes .local URLs drop the port, so any failure removes both.
        sudo(
            r#"{ install -m 644 -o root -g wheel "$1" "$2" && /sbin/pfctl -a "$5" -f "$2" && install -m 644 -o root -g wheel "$3" "$4" && { launchctl bootout system/"$6" 2>/dev/null; launchctl bootstrap system "$4"; }; } || { launchctl bootout system/"$6" 2>/dev/null; /sbin/pfctl -a "$5" -F all 2>/dev/null; rm -f "$2" "$4"; exit 1; }"#,
            &[&rules, PF_RULES, &plist, LAN_PLIST, PF_ANCHOR, LAN_LABEL],
        )
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result?;
    Ok(redirect)
}

/// Flushing the anchor leaves pf on until reboot; with the daemon gone, nothing turns it on again.
fn uninstall_lan() -> Result<()> {
    sudo(
        r#"launchctl bootout system/"$1" 2>/dev/null; /sbin/pfctl -a "$2" -F all 2>/dev/null; rm -f "$3" "$4""#,
        &[LAN_LABEL, PF_ANCHOR, LAN_PLIST, PF_RULES],
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_relay_runs_nc_as_the_user_on_a_launchd_socket() {
        let plist = super::plist("you", 1443, false);
        assert_eq!(
            super::target_in(&plist, "<string>127.0.0.1</string><string>", '<'),
            Some(1443)
        );
        assert!(!plist.contains("<string>80</string>"), "{plist}");
        for part in [
            "<key>UserName</key><string>you</string>",
            "<string>/usr/bin/nc</string><string>127.0.0.1</string><string>1443</string>",
            "<key>SockNodeName</key><string>127.0.0.1</string>",
            "<key>SockServiceName</key><string>443</string>",
        ] {
            assert!(plist.contains(part), "{part}\n{plist}");
        }
    }

    #[test]
    fn the_relay_also_takes_port_80_when_asked() {
        let plist = super::plist("you", 1443, true);
        assert!(plist.contains("<key>HTTP</key><dict>"), "{plist}");
        assert!(
            plist.contains("<key>SockServiceName</key><string>80</string>"),
            "{plist}"
        );
        assert_eq!(plist.matches("<string>1443</string>").count(), 1, "{plist}");
    }

    #[test]
    fn a_relay_without_port_80_is_told_apart() {
        assert!(super::takes_http(&super::plist("you", 1443, true)));
        assert!(!super::takes_http(&super::plist("you", 1443, false)));
    }

    #[test]
    fn the_linux_relay_runs_socket_proxyd_as_the_user_on_a_systemd_socket() {
        let (socket, service) = super::units(
            "parcela-https",
            "you",
            443,
            1443,
            "/usr/lib/systemd/systemd-socket-proxyd",
        );
        assert!(socket.contains("ListenStream=127.0.0.1:443\n"), "{socket}");
        assert_eq!(
            super::target_in(&service, "-proxyd 127.0.0.1:", '\n'),
            Some(1443)
        );
        for part in [
            "User=you\n",
            "ExecStart=/usr/lib/systemd/systemd-socket-proxyd 127.0.0.1:1443\n",
            "Requires=parcela-https.socket\n",
        ] {
            assert!(service.contains(part), "{part}\n{service}");
        }
    }

    #[test]
    fn the_relay_refuses_a_user_name_that_could_change_the_plist() {
        for name in ["dav", "first.last", "_www", "a-b_1"] {
            assert!(super::plain_name(name), "{name}");
        }
        for name in ["", "-x", "a<b", "a&b", "a b", "root</string>", "ü"] {
            assert!(!super::plain_name(name), "{name}");
        }
    }

    fn redirect(http: bool) -> super::LanRedirect {
        super::LanRedirect {
            interface: "en0".into(),
            target: 1443,
            http,
        }
    }

    #[test]
    fn the_pf_redirect_sends_443_on_the_interface_to_the_proxy_port_there() {
        let rules = super::pf_rules(&redirect(false));
        assert_eq!(
            rules,
            "rdr on en0 inet proto tcp from any to (en0) port 443 -> (en0) port 1443\n"
        );
        assert_eq!(super::parse_pf(&rules), Some(redirect(false)));
        let rules = super::pf_rules(&redirect(true));
        assert!(
            rules.contains(
                "rdr on en0 inet proto tcp from any to (en0) port 80 -> (en0) port 1443\n"
            ),
            "{rules}"
        );
        assert_eq!(super::parse_pf(&rules), Some(redirect(true)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pfctl_accepts_the_generated_rules_and_rejects_others_with_its_reason() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("lan.pf");
        let path = file.to_str().unwrap();
        std::fs::write(&file, super::pf_rules(&redirect(true))).unwrap();
        super::check_pf(path).unwrap();
        std::fs::write(&file, "rdr on en0 nonsense\n").unwrap();
        let error = super::check_pf(path).unwrap_err().to_string();
        assert!(error.contains("syntax error"), "{error}");
    }

    #[test]
    fn the_pf_daemon_loads_the_anchor_and_turns_pf_on_at_boot() {
        let plist = super::lan_plist();
        for part in [
            "<key>Label</key><string>org.parcela.lan</string>",
            "<string>/sbin/pfctl</string><string>-E</string><string>-a</string><string>com.apple/parcela</string><string>-f</string><string>/etc/pf.anchors/org.parcela.lan</string>",
            "<key>RunAtLoad</key><true/>",
        ] {
            assert!(plist.contains(part), "{part}\n{plist}");
        }
        assert!(!plist.contains("UserName"), "{plist}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_pf_daemon_is_a_valid_plist() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("lan.plist");
        std::fs::write(&file, super::lan_plist()).unwrap();
        let status = std::process::Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(&file)
            .output()
            .unwrap()
            .status;
        assert!(status.success());
    }

    #[test]
    fn the_lan_redirect_reaches_only_its_proxy_on_its_interface() {
        let ip = "192.168.1.20".parse().unwrap();
        let interfaces = [
            ("lo0".to_owned(), "127.0.0.1".parse().unwrap()),
            ("en0".to_owned(), ip),
        ];
        assert!(redirect(true).reaches(ip, 1443, &interfaces));
        assert!(!redirect(true).reaches(ip, 2443, &interfaces));
        assert!(!redirect(true).reaches("10.0.0.5".parse().unwrap(), 1443, &interfaces));
        let elsewhere = [("en1".to_owned(), ip)];
        assert!(!redirect(true).reaches(ip, 1443, &elsewhere));
    }
}

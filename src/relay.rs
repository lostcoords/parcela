//! `parcela relay install`: URLs without a port. The system's service manager owns a real socket
//! on 127.0.0.1:443 and relays each connection, as the user, to the proxy's port: launchd starts
//! `/usr/bin/nc` on macOS, systemd starts `systemd-socket-proxyd` on Linux. No Parcela code runs
//! as root, and a busy 443 is a bind error, never a redirect that hides another program's
//! listener. A free 80 is relayed to the same port, where the proxy redirects plain HTTP to HTTPS.
use crate::frontdoor::FrontDoor;
use anyhow::{Context, Result, bail, ensure};
use std::{path::Path, process::Command};

pub const LABEL: &str = "org.parcela.https";
const PLIST: &str = "/Library/LaunchDaemons/org.parcela.https.plist";
const UNITS: &str = "/etc/systemd/system";
/// systemd units: a socket and its relay service per relayed port.
const HTTPS_UNIT: &str = "parcela-https";
const HTTP_UNIT: &str = "parcela-http";

fn user() -> Result<String> {
    // SAFETY: getpwuid returns a pointer into static storage or null; it is read at once.
    let name = unsafe {
        let entry = libc::getpwuid(libc::getuid());
        ensure!(!entry.is_null(), "cannot find the current user");
        std::ffi::CStr::from_ptr((*entry).pw_name)
    };
    let name = name.to_str().context("non-UTF-8 user name")?;
    ensure!(
        plain_user(name),
        "unsupported user name for the relay: {name:?}"
    );
    Ok(name.to_owned())
}

/// A name that goes into the plist as is: no XML markup and no launchd surprises.
fn plain_user(name: &str) -> bool {
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
        eprintln!("parcela: port 80 is in use by {holder}; http:// URLs will not redirect");
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
    crate::private::directory(&staging)?;
    let staged = |name: &str, text: &str| -> Result<String> {
        let file = staging.join(name);
        crate::private::write_bytes(&file, text.as_bytes())?;
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

pub fn uninstall(front_door: FrontDoor) -> Result<FrontDoor> {
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

#[cfg(test)]
mod tests {
    #[test]
    fn the_relay_runs_nc_as_the_user_on_a_launchd_socket() {
        let plist = super::plist("you", 1443, false);
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
            assert!(super::plain_user(name), "{name}");
        }
        for name in ["", "-x", "a<b", "a&b", "a b", "root</string>", "ü"] {
            assert!(!super::plain_user(name), "{name}");
        }
    }
}

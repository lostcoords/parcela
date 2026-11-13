//! `parcela autostart`: at login, start again the environments that were running when the machine
//! went down. A per-user service runs `parcela autostart run` once: a LaunchAgent on macOS, a
//! systemd user unit on Linux. It never needs root and starts nothing that `down` stopped.
use crate::{
    environment::lifecycle,
    state::registry::{Registry, SessionStatus},
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

const LABEL: &str = "org.parcela.autostart";
const UNIT: &str = "parcela-autostart.service";

fn home() -> Result<PathBuf> {
    Ok(std::env::var_os("HOME").context("HOME missing")?.into())
}

/// Where the per-user service file lives.
pub fn location() -> Result<PathBuf> {
    Ok(if cfg!(target_os = "macos") {
        home()?.join(format!("Library/LaunchAgents/{LABEL}.plist"))
    } else {
        let config = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(path) => PathBuf::from(path),
            None => home()?.join(".config"),
        };
        config.join("systemd/user").join(UNIT)
    })
}

pub fn enabled() -> bool {
    location().is_ok_and(|path| path.exists())
}

/// What the service runs, with the PATH of the shell that enabled it, so Caddy and the projects'
/// commands resolve at login as they do now.
struct Launch {
    executable: String,
    state: String,
    config: Option<String>,
    path: String,
    log: String,
}

impl Launch {
    fn new(state: &Path) -> Result<Self> {
        let text = |path: &Path| -> Result<String> {
            ensure!(path.is_absolute(), "{} must be absolute", path.display());
            let text = path.to_str().context("non-UTF-8 path")?;
            ensure!(
                !text
                    .chars()
                    .any(|c| matches!(c, '<' | '>' | '&' | '"' | '\n' | '%')),
                "unsupported character in {text}"
            );
            Ok(text.to_owned())
        };
        let config = match std::env::var_os("PARCELA_CONFIG") {
            Some(path) => Some(text(Path::new(&path))?),
            None => None,
        };
        let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
        ensure!(
            !path
                .chars()
                .any(|c| matches!(c, '<' | '>' | '&' | '"' | '\n' | '%')),
            "unsupported character in PATH"
        );
        Ok(Self {
            executable: text(&std::env::current_exe()?.canonicalize()?)?,
            state: text(state)?,
            config,
            path,
            log: text(&state.join("autostart.log"))?,
        })
    }

    fn plist(&self) -> String {
        let config = self.config.as_ref().map_or_else(String::new, |config| {
            format!("<key>PARCELA_CONFIG</key><string>{config}</string>")
        });
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>--state-dir</string><string>{}</string><string>autostart</string><string>run</string></array>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>{}</string>{config}</dict>
  <key>RunAtLoad</key><true/>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
            self.executable, self.state, self.path, self.log, self.log
        )
    }

    fn unit(&self) -> String {
        let config = self.config.as_ref().map_or_else(String::new, |config| {
            format!("Environment=\"PARCELA_CONFIG={config}\"\n")
        });
        format!(
            "[Unit]\nDescription=Start Parcela environments left running\nAfter=network-online.target\n\n[Service]\nType=oneshot\nEnvironment=\"PATH={}\"\n{config}ExecStart=\"{}\" --state-dir \"{}\" autostart run\nStandardOutput=append:{}\nStandardError=append:{}\n\n[Install]\nWantedBy=default.target\n",
            self.path, self.executable, self.state, self.log, self.log
        )
    }
}

pub fn enable(state: &Path) -> Result<()> {
    let launch = Launch::new(state)?;
    let file = location()?;
    let parent = file.parent().context("service file needs a directory")?;
    std::fs::create_dir_all(parent)?;
    if cfg!(target_os = "macos") {
        std::fs::write(&file, launch.plist())?;
    } else if cfg!(target_os = "linux") {
        std::fs::write(&file, launch.unit())?;
        run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        run(Command::new("systemctl").args(["--user", "enable", UNIT]))?;
    } else {
        bail!("`parcela autostart` supports macOS and Linux");
    }
    Ok(())
}

pub fn disable() -> Result<()> {
    let file = location()?;
    if !file.exists() {
        return Ok(());
    }
    if cfg!(target_os = "linux") {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", UNIT])
            .status();
    } else if cfg!(target_os = "macos") {
        // Loaded since the last login; a job that already ran unloads quietly.
        let _ = Command::new("launchctl")
            .arg("bootout")
            .arg(format!("gui/{}", unsafe { libc::getuid() }))
            .arg(&file)
            .stderr(std::process::Stdio::null())
            .status();
    }
    std::fs::remove_file(&file)?;
    if cfg!(target_os = "linux") {
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
    }
    Ok(())
}

fn run(command: &mut Command) -> Result<()> {
    let status = command.status().context("run systemctl")?;
    ensure!(status.success(), "systemctl failed ({status})");
    Ok(())
}

/// An environment to start again, with the sharing flags its last run had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: String,
    pub flags: Vec<String>,
}

/// The sharing flags that bring a run back as it was. The LAN address is found again, since it
/// may have changed across the reboot.
fn flags(share: &crate::share::Share) -> Vec<String> {
    let mut flags = Vec::new();
    if share.lan.is_some() {
        flags.push("--lan".into());
    }
    if share.tailscale.is_some() {
        flags.push("--tailscale".into());
    }
    for domain in &share.domains {
        flags.extend(["--domain".into(), domain.name.clone()]);
    }
    if let Some(domain) = share.domains.first() {
        flags.extend(["--domain-port".into(), domain.port.to_string()]);
    }
    // A run that shared nothing stays unshared, even if the host has sharing defaults now.
    if flags.is_empty() {
        flags.push("--no-share".into());
    }
    flags
}

/// The environments to start again: those whose last run was neither stopped with `down` nor is
/// still running, and whose checkout still exists.
pub fn pending(registry: &Registry) -> Result<Vec<Pending>> {
    let workspaces = registry.list(None)?;
    let mut pending = Vec::new();
    for session in registry.sessions()? {
        if !matches!(
            session.status,
            SessionStatus::Ready | SessionStatus::Starting
        ) {
            continue;
        }
        let Some(workspace) = workspaces
            .iter()
            .find(|workspace| workspace.id == session.environment_id)
        else {
            continue;
        };
        if !workspace.source.exists() {
            continue;
        }
        let status = lifecycle::status(registry, workspace)?;
        if matches!(status["state"].as_str(), Some("ready" | "starting")) {
            continue;
        }
        let share =
            crate::environment::record::Record::read(&registry.runtime_path(workspace), &session)
                .map(|record| record.share)
                .unwrap_or_default();
        pending.push(Pending {
            id: workspace.id.clone(),
            flags: flags(&share),
        });
    }
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(config: Option<&str>) -> Launch {
        Launch {
            executable: "/Users/you/.local/bin/parcela".into(),
            state: "/Users/you/.local/state/parcela".into(),
            config: config.map(str::to_owned),
            path: "/opt/homebrew/bin:/usr/bin:/bin".into(),
            log: "/Users/you/.local/state/parcela/autostart.log".into(),
        }
    }

    #[test]
    fn the_launch_agent_runs_autostart_with_the_enabling_path() {
        let plist = launch(None).plist();
        for part in [
            "<string>/Users/you/.local/bin/parcela</string><string>--state-dir</string><string>/Users/you/.local/state/parcela</string><string>autostart</string><string>run</string>",
            "<key>PATH</key><string>/opt/homebrew/bin:/usr/bin:/bin</string>",
            "<key>RunAtLoad</key><true/>",
        ] {
            assert!(plist.contains(part), "{part}\n{plist}");
        }
        assert!(!plist.contains("PARCELA_CONFIG"));
        assert!(
            launch(Some("/tmp/c.yaml"))
                .plist()
                .contains("<key>PARCELA_CONFIG</key><string>/tmp/c.yaml</string>")
        );
    }

    #[test]
    fn a_run_comes_back_with_the_sharing_it_had() {
        use crate::share::{Domain, Share};
        assert_eq!(flags(&Share::default()), ["--no-share"]);
        let share = Share {
            lan: Some("192.168.1.20".parse().unwrap()),
            domains: vec![
                Domain {
                    name: "tower.example.com".into(),
                    port: 1480,
                },
                Domain {
                    name: "dev.example.org".into(),
                    port: 1480,
                },
            ],
            ..Share::default()
        };
        assert_eq!(
            flags(&share),
            [
                "--lan",
                "--domain",
                "tower.example.com",
                "--domain",
                "dev.example.org",
                "--domain-port",
                "1480"
            ]
        );
    }

    #[test]
    fn the_user_unit_runs_once_at_login() {
        let unit = launch(Some("/tmp/c.yaml")).unit();
        for part in [
            "Type=oneshot\n",
            "ExecStart=\"/Users/you/.local/bin/parcela\" --state-dir \"/Users/you/.local/state/parcela\" autostart run\n",
            "Environment=\"PARCELA_CONFIG=/tmp/c.yaml\"\n",
            "WantedBy=default.target\n",
        ] {
            assert!(unit.contains(part), "{part}\n{unit}");
        }
    }
}

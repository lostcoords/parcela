//! Settings that belong to this machine rather than a project, in
//! `$XDG_CONFIG_HOME/parcela/config.yaml` or `$PARCELA_CONFIG`. `parcela config` edits them.
use crate::{config::Manifest, share::Request};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    net::Ipv4Addr,
    ops::RangeInclusive,
    path::{Path, PathBuf},
};

/// Every key `parcela config set` accepts.
pub const KEYS: [&str; 9] = [
    "sharing.domain",
    "sharing.domain_port",
    "sharing.lan",
    "sharing.lan_ip",
    "sharing.tailscale",
    "telemetry",
    "updates",
    "worktrees.directory",
    "ports.backend",
];

/// Where backend ports come from unless `ports.backend` says otherwise. Below every OS
/// ephemeral range (Linux 32768+, macOS and Windows 49152+), so a process binding port 0 cannot
/// take an allocated port before the backend binds it. It holds no WHATWG bad ports and stays
/// clear of rb-portless's 4000-4999 range.
pub const BACKEND_PORTS: RangeInclusive<u16> = 20000..=29999;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Host {
    pub sharing: Sharing,
    /// Anonymous usage data; unset means on.
    pub telemetry: Option<bool>,
    /// Daily release checks; unset means on.
    pub updates: Option<bool>,
    pub worktrees: Worktrees,
    pub ports: Ports,
}

/// How `up` and `restart` share every run on this machine unless `--no-share` says otherwise.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sharing {
    /// Serve `<name>.<domain>` for a front proxy on this machine.
    pub domain: Option<String>,
    pub domain_port: Option<u16>,
    /// Also serve `.local` names on the LAN.
    pub lan: Option<bool>,
    /// The LAN address when LAN sharing is on, instead of the detected one.
    pub lan_ip: Option<Ipv4Addr>,
    /// Also serve on the tailnet with `tailscale serve`.
    pub tailscale: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Worktrees {
    /// Where `parcela create` puts new checkouts, as `<directory>/<repository>-<name>`.
    pub directory: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ports {
    /// Backend port range, like `20000-29999`.
    pub backend: Option<String>,
}

impl Sharing {
    pub fn validate(&self) -> Result<()> {
        if let Some(domain) = &self.domain {
            crate::share::domain_name(domain)?;
        }
        ensure!(
            self.domain_port.is_none() || self.domain.is_some(),
            "sharing.domain_port requires sharing.domain"
        );
        ensure!(
            self.domain_port.is_none_or(|port| port >= 1024),
            "sharing.domain_port must be between 1024 and 65535"
        );
        Ok(())
    }
}

/// The host configuration file, which need not exist.
pub fn path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PARCELA_CONFIG") {
        ensure!(!path.is_empty(), "PARCELA_CONFIG must not be empty");
        return Ok(path.into());
    }
    let directory = if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(path);
        ensure!(path.is_absolute(), "XDG_CONFIG_HOME must be absolute");
        path
    } else {
        home()?.join(".config")
    };
    Ok(directory.join("parcela/config.yaml"))
}

fn home() -> Result<PathBuf> {
    Ok(std::env::var_os("HOME")
        .context("HOME missing; set PARCELA_CONFIG")?
        .into())
}

impl Host {
    /// This machine's settings.
    pub fn current() -> Result<Self> {
        Self::load(&path()?)
    }

    /// The settings at `path`, or none when it does not exist.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        Self::parse(&text).with_context(|| format!("invalid {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let host: Self = serde_saphyr::from_str(text)?;
        host.validate()?;
        Ok(host)
    }

    fn validate(&self) -> Result<()> {
        self.sharing.validate()?;
        if let Some(directory) = &self.worktrees.directory {
            directory_path(directory)?;
        }
        if let Some(range) = &self.ports.backend {
            port_range(range)?;
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .context("config file needs a parent directory")?;
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        let temporary = parent.join(format!(".config.{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&temporary, self.to_yaml())
            .and_then(|()| fs::rename(&temporary, path))
            .inspect_err(|_| drop(fs::remove_file(&temporary)))
            .with_context(|| format!("write {}", path.display()))
    }

    /// One `key: value` line per set key, grouped under its section. Strings are JSON-quoted,
    /// which YAML reads as double-quoted scalars.
    fn to_yaml(&self) -> String {
        let mut text = String::from("# Parcela host settings; edit with `parcela config`.\n");
        let mut section = "";
        for key in KEYS {
            let value = self.get(key).expect("known key");
            if value.is_null() {
                continue;
            }
            match key.split_once('.') {
                Some((parent, field)) => {
                    if parent != section {
                        text.push_str(&format!("{parent}:\n"));
                        section = parent;
                    }
                    text.push_str(&format!("  {field}: {value}\n"));
                }
                None => {
                    section = "";
                    text.push_str(&format!("{key}: {value}\n"));
                }
            }
        }
        text
    }

    /// The value of `key`, null when unset.
    pub fn get(&self, key: &str) -> Result<Value> {
        let sharing = &self.sharing;
        Ok(match key {
            "sharing.domain" => json!(sharing.domain),
            "sharing.domain_port" => json!(sharing.domain_port),
            "sharing.lan" => json!(sharing.lan),
            "sharing.lan_ip" => json!(sharing.lan_ip),
            "sharing.tailscale" => json!(sharing.tailscale),
            "telemetry" => json!(self.telemetry),
            "updates" => json!(self.updates),
            "worktrees.directory" => json!(self.worktrees.directory),
            "ports.backend" => json!(self.ports.backend),
            _ => bail!(unknown(key)),
        })
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let sharing = &mut self.sharing;
        match key {
            "sharing.domain" => sharing.domain = Some(crate::share::domain_name(value)?),
            "sharing.domain_port" => {
                let port = value
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port >= 1024)
                    .with_context(|| format!("{key} {value}: use a port between 1024 and 65535"))?;
                sharing.domain_port = Some(port);
            }
            "sharing.lan" => sharing.lan = Some(boolean(key, value)?),
            "sharing.lan_ip" => {
                sharing.lan_ip = Some(
                    value
                        .parse()
                        .with_context(|| format!("{key} {value}: use an IPv4 address"))?,
                );
            }
            "sharing.tailscale" => sharing.tailscale = Some(boolean(key, value)?),
            "telemetry" => self.telemetry = Some(boolean(key, value)?),
            "updates" => self.updates = Some(boolean(key, value)?),
            "worktrees.directory" => {
                directory_path(value)?;
                self.worktrees.directory = Some(value.trim_end_matches('/').to_owned());
            }
            "ports.backend" => {
                let range = port_range(value)?;
                self.ports.backend = Some(format!("{}-{}", range.start(), range.end()));
            }
            _ => bail!(unknown(key)),
        }
        Ok(())
    }

    /// Clear `key`. Clearing the domain also clears its port, which needs it.
    pub fn unset(&mut self, key: &str) -> Result<()> {
        let sharing = &mut self.sharing;
        match key {
            "sharing.domain" => (sharing.domain, sharing.domain_port) = (None, None),
            "sharing.domain_port" => sharing.domain_port = None,
            "sharing.lan" => sharing.lan = None,
            "sharing.lan_ip" => sharing.lan_ip = None,
            "sharing.tailscale" => sharing.tailscale = None,
            "telemetry" => self.telemetry = None,
            "updates" => self.updates = None,
            "worktrees.directory" => self.worktrees.directory = None,
            "ports.backend" => self.ports.backend = None,
            _ => bail!(unknown(key)),
        }
        Ok(())
    }

    /// Each key with its value, unset keys included.
    pub fn entries(&self) -> Vec<(&'static str, Value)> {
        KEYS.iter()
            .map(|key| (*key, self.get(key).expect("known key")))
            .collect()
    }

    /// Add this machine's sharing defaults to a run of `manifest`. A project without an HTTP
    /// service has nothing to share, so it gets none.
    pub fn share(&self, request: &mut Request, manifest: &Manifest) {
        if manifest.services.is_empty() {
            return;
        }
        let sharing = &self.sharing;
        if request.domain.is_none() {
            request.domain = sharing.domain.clone();
        }
        request.domain_port = request.domain_port.or(sharing.domain_port);
        request.lan |= sharing.lan == Some(true);
        request.tailscale |= sharing.tailscale == Some(true);
        if request.lan || request.lan_ip.is_some() {
            request.lan_ip = request.lan_ip.or(sharing.lan_ip);
        }
    }

    /// The directory `parcela create` puts new checkouts in, when set.
    pub fn worktrees_directory(&self) -> Result<Option<PathBuf>> {
        self.worktrees
            .directory
            .as_deref()
            .map(directory_path)
            .transpose()
    }

    pub fn backend_ports(&self) -> RangeInclusive<u16> {
        self.ports
            .backend
            .as_deref()
            .and_then(|range| port_range(range).ok())
            .unwrap_or(BACKEND_PORTS)
    }
}

fn boolean(key: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => bail!("{key} {value}: use true or false"),
    }
}

/// An absolute path, or one under `~/`.
fn directory_path(value: &str) -> Result<PathBuf> {
    let path = match value.strip_prefix('~') {
        Some("") => home()?,
        Some(rest) if rest.starts_with('/') => home()?.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(value),
    };
    ensure!(
        path.is_absolute(),
        "worktrees.directory {value}: use an absolute path or one under ~/"
    );
    Ok(path)
}

fn port_range(value: &str) -> Result<RangeInclusive<u16>> {
    let range = value
        .split_once('-')
        .and_then(|(start, end)| Some((start.trim().parse().ok()?, end.trim().parse().ok()?)))
        .filter(|(start, end): &(u16, u16)| *start >= 1024 && start <= end)
        .with_context(|| {
            format!("ports.backend {value}: use a range like 20000-29999, from port 1024 up")
        })?;
    Ok(range.0..=range.1)
}

fn unknown(key: &str) -> String {
    format!("unknown key {key}; use one of: {}", KEYS.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(services: &str) -> Manifest {
        Manifest::parse(&format!("version: 1\nproject: app\nservices: {services}\n")).unwrap()
    }

    #[test]
    fn round_trips_through_yaml() {
        let mut host = Host::default();
        for (key, value) in [
            ("sharing.domain", "Tower.Example.com."),
            ("sharing.domain_port", "1481"),
            ("sharing.lan", "yes"),
            ("sharing.lan_ip", "192.168.1.20"),
            ("sharing.tailscale", "false"),
            ("telemetry", "off"),
            ("updates", "true"),
            ("worktrees.directory", "/srv/my worktrees: #1/"),
            ("ports.backend", " 30000 - 30999"),
        ] {
            host.set(key, value).unwrap();
        }
        let text = host.to_yaml();
        assert!(
            text.contains("sharing:\n  domain: \"tower.example.com\"\n"),
            "{text}"
        );
        let parsed = Host::parse(&text).unwrap();
        assert_eq!(
            serde_json::to_value(parsed.entries()).unwrap(),
            json!([
                ["sharing.domain", "tower.example.com"],
                ["sharing.domain_port", 1481],
                ["sharing.lan", true],
                ["sharing.lan_ip", "192.168.1.20"],
                ["sharing.tailscale", false],
                ["telemetry", false],
                ["updates", true],
                ["worktrees.directory", "/srv/my worktrees: #1"],
                ["ports.backend", "30000-30999"],
            ])
        );
        assert_eq!(parsed.backend_ports(), 30000..=30999);
        let empty = Host::parse(&Host::default().to_yaml()).unwrap();
        assert!(empty.entries().iter().all(|(_, value)| value.is_null()));
    }

    #[test]
    fn rejects_bad_keys_and_values() {
        let mut host = Host::default();
        assert!(host.set("sharing.funnel", "true").is_err());
        assert!(host.set("sharing.domain", "localhost").is_err());
        assert!(host.set("sharing.domain_port", "80").is_err());
        assert!(host.set("sharing.lan", "maybe").is_err());
        assert!(host.set("sharing.lan_ip", "::1").is_err());
        assert!(host.set("worktrees.directory", "worktrees").is_err());
        assert!(host.set("ports.backend", "500-600").is_err());
        assert!(host.set("ports.backend", "30000-20000").is_err());
        assert!(Host::parse("sharing:\n  domain_port: 1481\n").is_err());
        assert!(Host::parse("sharing:\n  funnel: true\n").is_err());
        assert!(Host::parse("ports:\n  backend: lots\n").is_err());
    }

    #[test]
    fn worktrees_directory_expands_home() {
        let mut host = Host::default();
        host.set("worktrees.directory", "~/worktrees").unwrap();
        assert_eq!(
            host.worktrees_directory().unwrap().unwrap(),
            home().unwrap().join("worktrees")
        );
    }

    #[test]
    fn sharing_defaults_add_to_the_request() {
        let host = Host::parse(
            "sharing:\n  domain: tower.example.com\n  lan: true\n  lan_ip: 10.0.0.2\n  tailscale: true\n",
        )
        .unwrap();
        let web = manifest("{web: {command: serve, http: {healthcheck: /}}}");
        let mut request = Request {
            domain: Some("other.example.com".into()),
            ..Request::default()
        };
        host.share(&mut request, &web);
        assert_eq!(request.domain.as_deref(), Some("other.example.com"));
        assert!(request.lan && request.tailscale);
        assert_eq!(request.lan_ip, Some(Ipv4Addr::new(10, 0, 0, 2)));

        let mut request = Request::default();
        host.share(&mut request, &manifest("{worker: {command: work}}"));
        assert!(!request.any(), "a project without a URL is never shared");

        let address_only = Host::parse("sharing:\n  lan_ip: 10.0.0.2\n").unwrap();
        let mut request = Request::default();
        address_only.share(&mut request, &web);
        assert!(
            !request.any(),
            "an address alone never turns LAN sharing on"
        );
    }

    #[test]
    fn unsetting_the_domain_clears_its_port() {
        let mut host =
            Host::parse("sharing:\n  domain: a.example.com\n  domain_port: 1481\n").unwrap();
        host.unset("sharing.domain").unwrap();
        assert!(host.sharing.domain.is_none() && host.sharing.domain_port.is_none());
    }
}

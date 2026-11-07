//! Settings that belong to this machine rather than a project, in
//! `$XDG_CONFIG_HOME/parcela/config.yaml` or `$PARCELA_CONFIG`. `parcela config` edits them.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Every key `parcela config` accepts.
pub const KEYS: [&str; 2] = ["sharing.domain", "sharing.domain_port"];

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Host {
    pub sharing: Sharing,
}

/// The domain `up` and `restart` share every environment under, for a front proxy on this machine.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sharing {
    pub domain: Option<String>,
    pub domain_port: Option<u16>,
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
        PathBuf::from(std::env::var_os("HOME").context("HOME missing; set PARCELA_CONFIG")?)
            .join(".config")
    };
    Ok(directory.join("parcela/config.yaml"))
}

impl Host {
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
        host.sharing.validate()?;
        Ok(host)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.sharing.validate()?;
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

    fn to_yaml(&self) -> String {
        let mut text = String::from("# Parcela host settings; edit with `parcela config`.\n");
        let Sharing {
            domain,
            domain_port,
        } = &self.sharing;
        if domain.is_some() || domain_port.is_some() {
            text.push_str("sharing:\n");
        }
        if let Some(domain) = domain {
            text.push_str(&format!("  domain: {domain}\n"));
        }
        if let Some(port) = domain_port {
            text.push_str(&format!("  domain_port: {port}\n"));
        }
        text
    }

    /// The sharing defaults for a run of `manifest`: none for a project without a URL to share.
    pub fn sharing_for(&self, manifest: &crate::config::Manifest) -> Sharing {
        if manifest.services.is_empty() {
            Sharing::default()
        } else {
            self.sharing.clone()
        }
    }

    /// The value of `key`, null when unset.
    pub fn get(&self, key: &str) -> Result<Value> {
        Ok(match key {
            "sharing.domain" => json!(self.sharing.domain),
            "sharing.domain_port" => json!(self.sharing.domain_port),
            _ => bail!(unknown(key)),
        })
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "sharing.domain" => {
                self.sharing.domain = Some(crate::share::domain_name(value)?);
            }
            "sharing.domain_port" => {
                let port = value
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port >= 1024)
                    .with_context(|| {
                        format!("sharing.domain_port {value}: use a port between 1024 and 65535")
                    })?;
                self.sharing.domain_port = Some(port);
            }
            _ => bail!(unknown(key)),
        }
        Ok(())
    }

    /// Clear `key`. Clearing the domain also clears its port, which needs it.
    pub fn unset(&mut self, key: &str) -> Result<()> {
        match key {
            "sharing.domain" => self.sharing = Sharing::default(),
            "sharing.domain_port" => self.sharing.domain_port = None,
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
}

fn unknown(key: &str) -> String {
    format!("unknown key {key}; use one of: {}", KEYS.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_yaml() {
        let mut host = Host::default();
        host.set("sharing.domain", "Tower.Example.com.").unwrap();
        host.set("sharing.domain_port", "1481").unwrap();
        let parsed = Host::parse(&host.to_yaml()).unwrap();
        assert_eq!(parsed.get("sharing.domain").unwrap(), "tower.example.com");
        assert_eq!(parsed.get("sharing.domain_port").unwrap(), 1481);
        assert!(
            Host::parse(&Host::default().to_yaml())
                .unwrap()
                .sharing
                .domain
                .is_none()
        );
    }

    #[test]
    fn rejects_bad_keys_and_values() {
        let mut host = Host::default();
        assert!(host.set("sharing.lan", "true").is_err());
        assert!(host.set("sharing.domain", "localhost").is_err());
        assert!(host.set("sharing.domain_port", "80").is_err());
        assert!(Host::parse("sharing:\n  domain_port: 1481\n").is_err());
        assert!(Host::parse("sharing:\n  lan: true\n").is_err());
    }

    #[test]
    fn projects_without_a_url_are_not_shared() {
        let host = Host::parse("sharing:\n  domain: tower.example.com\n").unwrap();
        let worker = crate::config::Manifest::parse(
            "version: 1\nproject: jobs\nservices: {worker: {command: work}}\n",
        )
        .unwrap();
        let web = crate::config::Manifest::parse(
            "version: 1\nproject: web\nservices: {web: {command: serve, http: {healthcheck: /}}}\n",
        )
        .unwrap();
        assert!(host.sharing_for(&worker).domain.is_none());
        assert_eq!(
            host.sharing_for(&web).domain.as_deref(),
            Some("tower.example.com")
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

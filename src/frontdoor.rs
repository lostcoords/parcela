//! The port browsers reach Parcela's proxy on, saved in `<state>/proxy/frontdoor.json`.
use crate::private;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The HTTPS port used until the person picks another one.
pub const DEFAULT_HTTPS_PORT: u16 = 1443;
/// Overrides [`DEFAULT_HTTPS_PORT`] while no profile has been saved.
pub const PORT_ENV: &str = "PARCELA_HTTPS_PORT";

/// Files from 0.3 and earlier also carry a `mode`, which is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontDoor {
    pub https: u16,
    /// Set by `parcela relay install`: the system relays 443 to `https`, so URLs have no port.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relay: bool,
}

fn path(state: &Path) -> PathBuf {
    state.join("proxy/frontdoor.json")
}

impl FrontDoor {
    /// The saved port, or the default when none has been saved yet.
    pub fn load(state: &Path) -> Result<Self> {
        let file = path(state);
        if std::fs::symlink_metadata(&file).is_err() {
            return Self::default_port();
        }
        private::read_json(&file)
            .with_context(|| format!("unreadable proxy port file {}", file.display()))
    }

    fn default_port() -> Result<Self> {
        let https = match std::env::var(PORT_ENV) {
            Ok(value) => value
                .parse()
                .with_context(|| format!("{PORT_ENV} must be a port number, got {value:?}"))?,
            Err(_) => DEFAULT_HTTPS_PORT,
        };
        Ok(Self::on(https))
    }

    /// The proxy on `https`, without the 443 relay.
    pub fn on(https: u16) -> Self {
        Self {
            https,
            relay: false,
        }
    }

    pub fn save(&self, state: &Path) -> Result<()> {
        private::write_json(&path(state), self)
    }

    /// URLs leave out the port when it is HTTPS's own, directly or through the relay.
    pub fn url(&self, host: &str) -> String {
        match self.https {
            _ if self.relay => format!("https://{host}"),
            443 => format!("https://{host}"),
            port => format!("https://{host}:{port}"),
        }
    }

    /// One phrase for people: where the proxy answers.
    pub fn describe(&self) -> String {
        if self.relay {
            format!("HTTPS on :443, relayed to :{}", self.https)
        } else {
            format!("HTTPS on :{}", self.https)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FrontDoor;

    #[test]
    fn urls_carry_the_port_unless_it_is_443() {
        assert_eq!(
            FrontDoor::on(1443).url("notes.localhost"),
            "https://notes.localhost:1443"
        );
        assert_eq!(
            FrontDoor::on(443).url("notes.localhost"),
            "https://notes.localhost"
        );
        let relayed = FrontDoor {
            https: 1443,
            relay: true,
        };
        assert_eq!(relayed.url("notes.localhost"), "https://notes.localhost");
    }

    #[test]
    fn saved_as_the_https_port() {
        assert_eq!(
            serde_json::to_value(FrontDoor::on(1443)).unwrap(),
            serde_json::json!({"https": 1443})
        );
    }
}

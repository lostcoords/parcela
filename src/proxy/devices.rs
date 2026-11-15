//! Device pairing for LAN shares. Every request to a `.local` name passes Caddy's `forward_auth`
//! to the dashboard socket, which answers from this store: a device whose cookie names an approved
//! record reaches the app as this machine would, since Caddy drops the headers that name the
//! device; any other gets a page asking the person at this machine to approve it. Approval happens only on loopback,
//! in the dashboard or the CLI. The cookie belongs to one `.local` name, since browsers keep no
//! cookie across single-label `.local` domains, so each name a device opens is approved once.
use crate::state::private::{self, Lock};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const COOKIE: &str = "parcela_device";
/// Older pending requests make room for new ones, so an unpaired device cannot grow the store.
const MAX_PENDING: usize = 20;
const COOKIE_MAX_AGE: u32 = 400 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Standing {
    Pending,
    Approved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: String,
    pub standing: Standing,
    /// The `.local` name whose cookie holds the token.
    pub host: String,
    pub user_agent: String,
    pub address: String,
    pub requested_at_unix_ms: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at_unix_ms: Option<u128>,
    token_sha256: String,
}

impl Device {
    /// Everything but the token's hash, for the CLI and the dashboard.
    pub fn public(&self) -> Value {
        json!({
            "id": self.id,
            "standing": self.standing,
            "host": self.host,
            "user_agent": self.user_agent,
            "address": self.address,
            "requested_at_unix_ms": self.requested_at_unix_ms,
            "decided_at_unix_ms": self.decided_at_unix_ms,
        })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    devices: Vec<Device>,
}

fn file(state: &Path) -> PathBuf {
    state.join("proxy/devices.json")
}

fn lock(state: &Path) -> Result<Lock> {
    private::directory(&state.join("proxy"))?;
    Lock::wait(&state.join("proxy/devices.lock"))
}

fn load(state: &Path) -> Result<Store> {
    let path = file(state);
    if std::fs::symlink_metadata(&path).is_err() {
        return Ok(Store::default());
    }
    private::read_json(&path).with_context(|| format!("unreadable device list {}", path.display()))
}

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn random_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn list(state: &Path) -> Result<Vec<Device>> {
    Ok(load(state)?.devices)
}

/// What the person decided about a device in the dashboard or the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approve,
    /// Forgets the device; its next page load asks again.
    Revoke,
}

impl Decision {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "approve" => Some(Self::Approve),
            "revoke" => Some(Self::Revoke),
            _ => None,
        }
    }
}

/// Apply `decision` to the device `id` and return it as it was decided.
pub fn decide(state: &Path, id: &str, decision: Decision) -> Result<Device> {
    let _lock = lock(state)?;
    let mut store = load(state)?;
    let Some(index) = store.devices.iter().position(|device| device.id == id) else {
        bail!("no device {id}; `parcela devices` lists them");
    };
    let decided = match decision {
        Decision::Revoke => store.devices.remove(index),
        Decision::Approve => {
            let device = &mut store.devices[index];
            device.standing = Standing::Approved;
            device.decided_at_unix_ms = Some(crate::proxy::opens::now_unix_ms());
            device.clone()
        }
    };
    private::write_json(&file(state), &store)?;
    Ok(decided)
}

/// Forget every device and return how many there were.
pub fn revoke_all(state: &Path) -> Result<usize> {
    let _lock = lock(state)?;
    let store = load(state)?;
    private::write_json(&file(state), &Store::default())?;
    Ok(store.devices.len())
}

/// One request at a LAN name, as Caddy describes it to the gate.
#[derive(Debug, Default)]
pub struct Ask<'a> {
    pub host: &'a str,
    pub cookie: Option<&'a str>,
    /// A top-level page load, the only request that may ask for approval.
    pub navigation: bool,
    pub user_agent: &'a str,
    pub address: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Gate {
    Allow,
    /// `token` is set when this request created the pending record, for its cookie.
    Wait {
        device: String,
        token: Option<String>,
    },
    Refuse,
}

/// The value of the pairing cookie in a `Cookie` header.
pub fn cookie_token(header: &str) -> Option<&str> {
    header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == COOKIE && !value.is_empty()).then_some(value)
    })
}

/// The gate's answer for `ask`, adding a pending record for an unknown device's page load.
fn gate(store: &mut Store, ask: &Ask, now: u128) -> Gate {
    let known = ask.cookie.map(hash).and_then(|hashed| {
        store
            .devices
            .iter()
            .find(|device| device.token_sha256 == hashed && device.host == ask.host)
    });
    match known {
        Some(device) => match device.standing {
            Standing::Approved => Gate::Allow,
            Standing::Pending => Gate::Wait {
                device: device.id.clone(),
                token: None,
            },
        },
        None if ask.navigation => {
            let pending = store
                .devices
                .iter()
                .filter(|device| device.standing == Standing::Pending)
                .count();
            if pending >= MAX_PENDING
                && let Some(oldest) = store
                    .devices
                    .iter()
                    .position(|device| device.standing == Standing::Pending)
            {
                store.devices.remove(oldest);
            }
            let id = loop {
                let id = Uuid::new_v4().simple().to_string()[..8].to_owned();
                if store.devices.iter().all(|device| device.id != id) {
                    break id;
                }
            };
            let token = random_token();
            store.devices.push(Device {
                id: id.clone(),
                standing: Standing::Pending,
                host: ask.host.to_owned(),
                user_agent: ask.user_agent.chars().take(300).collect(),
                address: ask.address.chars().take(64).collect(),
                requested_at_unix_ms: now,
                decided_at_unix_ms: None,
                token_sha256: hash(&token),
            });
            Gate::Wait {
                device: id,
                token: Some(token),
            }
        }
        None => Gate::Refuse,
    }
}

/// The gate's HTTP answer: status, extra headers and an HTML or text body. Only a 2xx lets Caddy
/// continue to the app.
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub mime: &'static str,
    pub body: Vec<u8>,
    /// The nonce of the page's one script, for its policy.
    pub nonce: Option<String>,
}

pub fn answer(state: &Path, ask: &Ask) -> Result<Answer> {
    let quick = load(state).map(|mut store| gate(&mut store, ask, 0))?;
    let decision = match quick {
        Gate::Wait { token: Some(_), .. } => {
            let _lock = lock(state)?;
            let mut store = load(state)?;
            let decision = gate(&mut store, ask, crate::proxy::opens::now_unix_ms());
            private::write_json(&file(state), &store)?;
            decision
        }
        decision => decision,
    };
    let machine = machine();
    Ok(match decision {
        Gate::Allow => Answer {
            status: 200,
            headers: Vec::new(),
            mime: "text/plain",
            body: Vec::new(),
            nonce: None,
        },
        Gate::Wait { device, token } => {
            let nonce = Uuid::new_v4().simple().to_string();
            Answer {
            status: 403,
            headers: token
                .map(|token| {
                    (
                        "Set-Cookie",
                        format!(
                            "{COOKIE}={token}; Path=/; Max-Age={COOKIE_MAX_AGE}; Secure; HttpOnly; SameSite=Lax"
                        ),
                    )
                })
                .into_iter()
                .chain([(PAIRING_HEADER, "pending".to_owned())])
                .collect(),
            mime: "text/html; charset=utf-8",
            body: page(
                &format!("Pair with {machine}"),
                &banner(&machine),
                "Pair this device",
                &format!(
                    "<p>Waiting for approval on {machine}. Open the Parcela dashboard there and approve device <strong>{device}</strong>. This page continues by itself.</p>",
                    machine = escape(&machine)
                ),
                Some(&nonce),
            ),
            nonce: Some(nonce),
        }},
        Gate::Refuse => Answer {
            status: 403,
            headers: Vec::new(),
            mime: "text/plain",
            body: format!(
                "This device is not paired with {machine}. Open the page to pair it. Certificate: {CERTIFICATE_PATH}\n"
            )
            .into_bytes(),
            nonce: None,
        },
    })
}

/// This machine's name, as people see it on the network.
fn machine() -> String {
    let mut buffer = [0u8; 256];
    // SAFETY: gethostname writes at most `buffer.len()` bytes into the buffer it is given.
    let named = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } == 0;
    let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(0);
    let name = String::from_utf8_lossy(&buffer[..end]);
    match name.trim_end_matches(".local") {
        name if named && !name.is_empty() => name.to_owned(),
        _ => "this computer".into(),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const CERTIFICATE_PATH: &str = "/_parcela/certificate";
/// Tells the waiting page's script that the device still waits; anything else is worth a reload.
const PAIRING_HEADER: &str = "Parcela-Pairing";
/// Asks again every 3s without reloading, so the certificate steps stay open while the person
/// reads them. Without scripts the page reloads itself instead.
const WAITING_SCRIPT: &str = "setInterval(()=>fetch(location.href,{method:'HEAD',cache:'no-store'}).then(r=>{if(r.headers.get('Parcela-Pairing')!=='pending')location.reload()},()=>{}),3000)";

/// The policy of the pairing and certificate pages: inline styles, and the waiting page's one
/// script, by its nonce.
pub fn policy(nonce: Option<&str>) -> String {
    let script = nonce.map_or_else(String::new, |nonce| {
        format!("script-src 'nonce-{nonce}'; connect-src 'self'; ")
    });
    format!(
        "default-src 'none'; {script}style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"
    )
}

/// Always on the waiting page, since a page cannot tell whether its own certificate is trusted.
fn banner(machine: &str) -> String {
    format!(
        "<section style=\"display:grid;gap:12px;padding:16px;border-radius:16px;background:#22344a\">\
<p><strong>This device doesn't trust {machine} yet</strong>, so your browser warns you.</p>\
<a href=\"/_parcela/ca.crt\" download style=\"display:block;padding:12px 16px;border-radius:999px;background:#8cc8ff;color:#0d1a26;font-weight:600;text-align:center;text-decoration:none\">Download the certificate</a>\
<p><a href=\"{CERTIFICATE_PATH}\">How to install it, and its fingerprint</a></p></section>",
        machine = escape(machine)
    )
}

/// `body` is HTML; callers escape what they put in it.
/// With `nonce`, the page waits for approval and carries `WAITING_SCRIPT` under it.
fn page(title: &str, banner: &str, heading: &str, body: &str, nonce: Option<&str>) -> Vec<u8> {
    let refresh = nonce.map_or_else(String::new, |nonce| {
        format!(
            "<noscript><meta http-equiv=\"refresh\" content=\"3\"></noscript><script nonce=\"{nonce}\">{WAITING_SCRIPT}</script>"
        )
    });
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{refresh}<title>{title}</title><style>p{{margin:0}}a{{color:#8cc8ff}}</style></head><body style=\"margin:0;min-height:100vh;display:grid;place-items:center;font:16px/1.5 system-ui,sans-serif;background:#16212b;color:#e8edf2\"><main style=\"max-width:32rem;padding:24px;display:grid;gap:12px\">{banner}<h1 style=\"font-size:1.25rem;margin:0\">{heading}</h1>{body}</main></body></html>",
        title = escape(title),
        heading = escape(heading),
    )
    .into_bytes()
}

/// What a LAN name answers at `/_parcela/certificate` and `/_parcela/ca.crt`, before any pairing:
/// how to trust this proxy's root certificate, and the certificate itself. Never the key.
pub fn certificate(state: &Path, path: &str) -> Result<Answer> {
    let der = crate::proxy::certificate_der(state)?;
    Ok(match path {
        "/_parcela/ca.crt" => Answer {
            status: 200,
            headers: vec![(
                "Content-Disposition",
                "attachment; filename=\"parcela-ca.crt\"".into(),
            )],
            mime: "application/x-x509-ca-cert",
            body: der,
            nonce: None,
        },
        CERTIFICATE_PATH => Answer {
            status: 200,
            headers: Vec::new(),
            mime: "text/html; charset=utf-8",
            body: certificate_page(&machine(), &crate::proxy::fingerprint(&der)),
            nonce: None,
        },
        _ => Answer {
            status: 404,
            headers: Vec::new(),
            mime: "text/plain",
            body: b"Not found\n".to_vec(),
            nonce: None,
        },
    })
}

fn certificate_page(machine: &str, fingerprint: &str) -> Vec<u8> {
    let machine = escape(machine);
    page(
        &format!("Trust {machine}"),
        "",
        "Install this computer's certificate",
        &format!(
            "<p>Pages from {machine} are signed by its own certificate authority. Trust it on this device and the \"not secure\" warning goes away.</p>\
<p><a href=\"/_parcela/ca.crt\">Download the certificate</a></p>\
<p>Before you trust it, check that its SHA-256 fingerprint matches what <code>parcela trust --fingerprint</code> prints on {machine}:<br><code style=\"overflow-wrap:anywhere\">{fingerprint}</code></p>\
<p><strong>iPhone and iPad:</strong> download it and allow the profile. Open Settings › General › VPN &amp; Device Management, tap the downloaded profile, named Caddy Local Authority, and install it. Then open Settings › General › About › Certificate Trust Settings and turn on full trust for it.</p>\
<p><strong>Android:</strong> download it. Open Settings › Security › Encryption &amp; credentials › Install a certificate › CA certificate and choose the downloaded file. Menu names vary by maker.</p>\
<p><strong>Desktop browsers:</strong> download it and add it to the system's trusted roots: Keychain Access on macOS, Certificate Manager on Windows. Firefox keeps its own list under Settings › Privacy &amp; Security › Certificates.</p>\
<p><strong>What this allows:</strong> whoever holds this computer's Parcela key can then make certificates for any website that this device accepts, so they could read and change its HTTPS traffic. Trust it only on devices you own, and remove it when you are done: on iPhone, delete the profile in VPN &amp; Device Management; on Android, remove it under Encryption &amp; credentials › User credentials or Trusted credentials › User.</p>"
        ),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask<'a>(host: &'a str, cookie: Option<&'a str>, navigation: bool) -> Ask<'a> {
        Ask {
            host,
            cookie,
            navigation,
            user_agent: "Phone",
            address: "192.168.1.30",
        }
    }

    fn approve(store: &mut Store, id: &str) {
        store
            .devices
            .iter_mut()
            .find(|device| device.id == id)
            .unwrap()
            .standing = Standing::Approved;
    }

    #[test]
    fn each_waiting_page_allows_only_its_own_script() {
        let directory = tempfile::tempdir().unwrap();
        let waiting = || answer(directory.path(), &ask("shop.local", None, true)).unwrap();
        let (first, second) = (waiting(), waiting());
        let nonce = first.nonce.clone().unwrap();
        assert_ne!(Some(&nonce), second.nonce.as_ref());
        assert!(
            String::from_utf8(first.body)
                .unwrap()
                .contains(&format!("<script nonce=\"{nonce}\">"))
        );
        assert!(policy(Some(&nonce)).contains(&format!("script-src 'nonce-{nonce}'")));
        assert!(!policy(None).contains("script-src"));
    }

    #[test]
    fn an_unknown_device_waits_and_only_its_page_load_asks() {
        let mut store = Store::default();
        assert_eq!(
            gate(&mut store, &ask("shop.local", None, false), 1),
            Gate::Refuse
        );
        assert!(store.devices.is_empty());
        let Gate::Wait {
            device,
            token: Some(token),
        } = gate(&mut store, &ask("shop.local", None, true), 1)
        else {
            panic!("a page load asks for approval");
        };
        assert_eq!(store.devices.len(), 1);
        assert_eq!(
            gate(&mut store, &ask("shop.local", Some(&token), true), 2),
            Gate::Wait {
                device: device.clone(),
                token: None
            },
            "a reload waits on the same request"
        );
        assert_eq!(store.devices.len(), 1);
        approve(&mut store, &device);
        assert_eq!(
            gate(&mut store, &ask("shop.local", Some(&token), false), 3),
            Gate::Allow
        );
    }

    #[test]
    fn a_token_is_stored_hashed_and_holds_only_for_its_host() {
        let mut store = Store::default();
        let Gate::Wait {
            device,
            token: Some(token),
        } = gate(&mut store, &ask("shop.local", None, true), 1)
        else {
            panic!("a page load asks for approval");
        };
        approve(&mut store, &device);
        let saved = serde_json::to_string(&store).unwrap();
        assert!(!saved.contains(&token), "{saved}");
        assert!(saved.contains(&hash(&token)), "{saved}");
        assert_eq!(token.len(), 64);
        assert!(
            !store.devices[0]
                .public()
                .to_string()
                .contains(&hash(&token))
        );
        assert!(matches!(
            gate(&mut store, &ask("api-shop.local", Some(&token), true), 2),
            Gate::Wait { token: Some(_), .. }
        ));
        assert_eq!(
            gate(&mut store, &ask("shop.local", Some("forged"), false), 2),
            Gate::Refuse
        );
    }

    #[test]
    fn pending_requests_are_capped_and_the_oldest_make_room() {
        let mut store = Store::default();
        let Gate::Wait { device, .. } = gate(&mut store, &ask("shop.local", None, true), 1) else {
            panic!("a page load asks for approval");
        };
        for _ in 0..MAX_PENDING + 5 {
            gate(&mut store, &ask("shop.local", None, true), 3);
        }
        let pending = store
            .devices
            .iter()
            .filter(|device| device.standing == Standing::Pending)
            .count();
        assert_eq!(pending, MAX_PENDING);
        assert!(store.devices.iter().all(|pending| pending.id != device));
    }

    #[test]
    fn the_pairing_cookie_is_found_among_others() {
        assert_eq!(cookie_token("a=1; parcela_device=abc; b=2"), Some("abc"));
        assert_eq!(cookie_token("parcela_device="), None);
        assert_eq!(cookie_token("xparcela_device=abc"), None);
    }

    #[test]
    fn decisions_persist_in_the_state_directory() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path();
        let answer = answer(state, &ask("shop.local", None, true)).unwrap();
        assert_eq!(answer.status, 403);
        let (_, cookie) = &answer.headers[0];
        assert!(
            cookie.ends_with("; Path=/; Max-Age=34560000; Secure; HttpOnly; SameSite=Lax"),
            "{cookie}"
        );
        let token = cookie_token(cookie.split(';').next().unwrap()).unwrap();
        let id = list(state).unwrap()[0].id.clone();
        assert!(decide(state, "nope", Decision::Approve).is_err());
        decide(state, &id, Decision::Approve).unwrap();
        let allowed = super::answer(state, &ask("shop.local", Some(token), false)).unwrap();
        assert_eq!((allowed.status, allowed.headers), (200, Vec::new()));
        decide(state, &id, Decision::Revoke).unwrap();
        assert!(list(state).unwrap().is_empty());
        assert_eq!(
            super::answer(state, &ask("shop.local", Some(token), false))
                .unwrap()
                .status,
            403
        );
        super::answer(state, &ask("shop.local", None, true)).unwrap();
        assert_eq!(revoke_all(state).unwrap(), 1);
    }
}

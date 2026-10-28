//! Opening a run beyond this machine, only when `up` asks: `.local` names on the LAN through
//! Caddy, `tailscale serve` on the tailnet, and names under the person's own domain for a front
//! proxy they run. Publishers are processes of the run's graph, so they stop with it.
use crate::{config::Manifest, process::on_path, registry::Endpoint};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::Path,
    process::Command,
    time::Duration,
};

/// What `up --lan`, `up --tailscale` and `up --domain` asked for.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub lan: bool,
    /// The address for `--lan` instead of the detected one.
    pub lan_ip: Option<Ipv4Addr>,
    pub tailscale: bool,
    pub domain: Option<String>,
    /// The loopback port for `--domain` instead of [`DOMAIN_PORT`].
    pub domain_port: Option<u16>,
}

impl Request {
    pub fn any(&self) -> bool {
        self.lan || self.lan_ip.is_some() || self.tailscale || self.domain.is_some()
    }
}

/// How one run is shared, recorded with the run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Share {
    /// The LAN address the `.local` names point at. Caddy also listens there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan: Option<Ipv4Addr>,
    /// Caddy's port on that address. The 443 relay serves loopback only, so this is the proxy's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan_port: Option<u16>,
    /// The mDNS publisher, `dns-sd` or `avahi-publish`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mdns: Option<String>,
    /// The tailscale binary and this machine's tailnet name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<Tailnet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<Domain>,
}

/// Names under the person's own domain, `notes.dev.example.com`. Caddy answers them in plain HTTP
/// on loopback only; the person's front proxy holds the certificate and the public listener.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Domain {
    pub name: String,
    pub port: u16,
}

impl Domain {
    /// `notes.localhost` on this machine is `notes.<domain>` behind the front proxy.
    pub fn host(&self, hostname: &str) -> String {
        format!("{}.{}", hostname.trim_end_matches(".localhost"), self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tailnet {
    pub binary: String,
    pub host: String,
    /// The tailnet HTTPS port of each service.
    pub ports: BTreeMap<String, u16>,
}

/// Processes Parcela adds to the graph start with this, so no project service may.
const PREFIX: &str = "parcela-";
const TAILNET_PORTS: [u16; 9] = [443, 8443, 8444, 8445, 8446, 8447, 8448, 8449, 8450];
/// Where Caddy answers `--domain` names, so a front proxy can be configured once.
pub const DOMAIN_PORT: u16 = 1480;

impl Share {
    /// Check the request can be met now and choose addresses and ports for `endpoints`.
    pub fn plan(
        state: &Path,
        request: &Request,
        endpoints: &[Endpoint],
        https_port: u16,
    ) -> Result<Self> {
        let mut share = Self::default();
        if request.lan || request.lan_ip.is_some() {
            let ip = match request.lan_ip {
                Some(ip) => {
                    ensure!(
                        present(ip),
                        "--lan-ip {ip}: no network interface has that address"
                    );
                    ip
                }
                None => lan_address()?,
            };
            ensure!(
                private_network(ip),
                "{ip} is a public address; --lan only serves a private network"
            );
            let mdns = ["dns-sd", "avahi-publish"]
                .into_iter()
                .find_map(on_path)
                .context("--lan needs dns-sd (macOS) or avahi-publish (Linux) on PATH")?;
            probe_lan(state, ip, https_port)?;
            share.lan = Some(ip);
            share.lan_port = Some(https_port);
            share.mdns = Some(mdns.to_str().context("non-UTF-8 mDNS path")?.into());
        }
        if request.tailscale {
            share.tailscale = Some(Tailnet::plan(endpoints)?);
        }
        if let Some(name) = &request.domain {
            let name = domain_name(name)?;
            let port = request.domain_port.unwrap_or(DOMAIN_PORT);
            ensure!(
                port != https_port,
                "--domain-port {port} is the proxy's HTTPS port; pick another"
            );
            probe_domain(state, port)?;
            share.domain = Some(Domain { name, port });
        }
        Ok(share)
    }

    pub fn is_empty(&self) -> bool {
        self.lan.is_none() && self.tailscale.is_none() && self.domain.is_none()
    }

    /// The URLs other devices use for `endpoint`.
    pub fn urls(&self, endpoint: &Endpoint) -> Vec<String> {
        let mut urls = Vec::new();
        if self.lan.is_some() {
            let host = local_host(&endpoint.hostname);
            urls.push(match self.lan_port {
                Some(443) => format!("https://{host}"),
                Some(port) => format!("https://{host}:{port}"),
                None => endpoint.public_url.replacen(&endpoint.hostname, &host, 1),
            });
        }
        if let Some(tailnet) = &self.tailscale
            && let Some(port) = tailnet.ports.get(&endpoint.service)
        {
            urls.push(match port {
                443 => format!("https://{}", tailnet.host),
                port => format!("https://{}:{port}", tailnet.host),
            });
        }
        if let Some(domain) = &self.domain {
            urls.push(format!("https://{}", domain.host(&endpoint.hostname)));
        }
        urls
    }

    /// `PARCELA_SHARE_HOSTS`: every hostname other devices send, for a dev server's host check.
    pub fn hosts(&self, endpoints: &[Endpoint]) -> String {
        let mut hosts = Vec::new();
        if self.lan.is_some() {
            hosts.extend(
                endpoints
                    .iter()
                    .map(|endpoint| local_host(&endpoint.hostname)),
            );
        }
        if let Some(tailnet) = &self.tailscale {
            hosts.push(tailnet.host.clone());
        }
        if let Some(domain) = &self.domain {
            hosts.extend(
                endpoints
                    .iter()
                    .map(|endpoint| domain.host(&endpoint.hostname)),
            );
        }
        hosts.join(",")
    }

    /// The publisher processes to add to the graph, by name.
    pub fn processes(
        &self,
        manifest: &Manifest,
        endpoints: &[Endpoint],
        https_port: u16,
    ) -> Result<BTreeMap<String, String>> {
        let mut processes = BTreeMap::new();
        if self.is_empty() {
            return Ok(processes);
        }
        if let Some(name) = manifest
            .processes
            .keys()
            .find(|name| name.starts_with(PREFIX))
        {
            bail!("service {name} cannot be shared: names starting with {PREFIX} are Parcela's");
        }
        for endpoint in endpoints {
            if let (Some(ip), Some(mdns)) = (self.lan, &self.mdns) {
                let host = local_host(&endpoint.hostname);
                let label = host.trim_end_matches(".local");
                let command = if mdns.ends_with("dns-sd") {
                    format!(
                        "{} -P {label} _https._tcp local {https_port} {host} {ip}",
                        quote(mdns)
                    )
                } else {
                    format!("{} -a -R {host} {ip}", quote(mdns))
                };
                processes.insert(format!("{PREFIX}lan-{}", endpoint.service), command);
            }
            if let Some(tailnet) = &self.tailscale {
                let port = tailnet.ports[&endpoint.service];
                processes.insert(
                    format!("{PREFIX}tailscale-{}", endpoint.service),
                    format!(
                        "{} serve --https={port} http://127.0.0.1:{}",
                        quote(&tailnet.binary),
                        endpoint.backend_port
                    ),
                );
            }
        }
        Ok(processes)
    }
}

impl Tailnet {
    fn plan(endpoints: &[Endpoint]) -> Result<Self> {
        let binary = on_path("tailscale").context(
            "--tailscale needs the tailscale CLI on PATH: https://tailscale.com/download",
        )?;
        let status = tailscale_json(&binary, &["status", "--json"])?;
        ensure!(
            status["BackendState"] == "Running",
            "tailscale is not connected; run `tailscale up`"
        );
        let host = status["Self"]["DNSName"]
            .as_str()
            .unwrap_or_default()
            .trim_end_matches('.')
            .to_owned();
        ensure!(
            !host.is_empty(),
            "tailscale reports no name for this machine; turn on MagicDNS"
        );
        ensure!(
            has_capability(&status["Self"], "https"),
            "tailscale HTTPS certificates are off; turn on HTTPS at https://login.tailscale.com/admin/dns"
        );
        let mut used = BTreeSet::new();
        used_ports(
            &tailscale_json(&binary, &["serve", "status", "--json"])?,
            &mut used,
        );
        // A process outside `tailscale serve`, such as a host-network proxy, can listen on the
        // tailnet address itself. Serve would then retry its bind forever and answer TLS errors.
        let addresses: Vec<IpAddr> = status["Self"]["TailscaleIPs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|ip| ip.as_str()?.parse().ok())
            .collect();
        let mut free = TAILNET_PORTS
            .into_iter()
            .chain(TAILNET_PORTS[TAILNET_PORTS.len() - 1] + 1..=u16::MAX)
            .filter(|port| !used.contains(port) && !listened(&addresses, *port));
        let ports = endpoints
            .iter()
            .map(|endpoint| {
                let port = free.next().context("no free tailnet HTTPS port")?;
                Ok((endpoint.service.clone(), port))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            binary: binary.to_str().context("non-UTF-8 tailscale path")?.into(),
            host,
            ports,
        })
    }
}

/// `notes.localhost` on this machine is `notes.local` on the LAN.
pub fn local_host(hostname: &str) -> String {
    format!("{}.local", hostname.trim_end_matches(".localhost"))
}

/// The address of the interface the default route uses. Connecting a UDP socket sends nothing.
fn lan_address() -> Result<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let address = socket
        .connect((Ipv4Addr::new(192, 0, 2, 1), 9))
        .and_then(|()| socket.local_addr())
        .ok()
        .and_then(|address| match address.ip() {
            std::net::IpAddr::V4(ip) => Some(ip),
            std::net::IpAddr::V6(_) => None,
        })
        .filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
        .context("no LAN address; connect to a network first")?;
    // Tailscale's 100.64.0.0/10: the default route goes through an exit node, not the LAN.
    ensure!(
        !(address.octets()[0] == 100 && address.octets()[1] & 0xc0 == 64),
        "the default route goes through Tailscale ({address}), not a LAN; use --tailscale"
    );
    Ok(address)
}

/// A lowercase domain of at least two DNS labels. Each environment's name becomes one more label,
/// so a single wildcard certificate covers them all.
pub(crate) fn domain_name(value: &str) -> Result<String> {
    let name = value.trim_end_matches('.').to_ascii_lowercase();
    ensure!(
        name.contains('.'),
        "--domain {value}: use a domain you control, such as dev.example.com"
    );
    for label in name.split('.') {
        crate::config::dns_label(label).with_context(|| format!("--domain {value}"))?;
    }
    ensure!(
        !["localhost", "local"]
            .iter()
            .any(|suffix| name.ends_with(&format!(".{suffix}"))),
        "--domain {value}: .localhost and .local names are Parcela's own"
    );
    Ok(name)
}

/// Refuse a loopback port someone else holds. The proxy's own domain listener is fine.
fn probe_domain(state: &Path, port: u16) -> Result<()> {
    if TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
        || crate::router::listening(state, "domain", &format!("127.0.0.1:{port}"))
    {
        return Ok(());
    }
    bail!(
        "port {port} on 127.0.0.1 is already in use by {}; pick another with --domain-port",
        crate::process::port_holder(port)
    )
}

/// RFC 1918 and link-local addresses. A public address would serve the app to the internet.
fn private_network(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local()
}

/// Refuse a LAN port someone else holds. The proxy's own LAN listener is fine.
fn probe_lan(state: &Path, ip: Ipv4Addr, port: u16) -> Result<()> {
    if TcpListener::bind((ip, port)).is_ok()
        || crate::router::listening(state, "lan", &format!("{ip}:{port}"))
    {
        return Ok(());
    }
    bail!(
        "port {port} on {ip} is already in use by {}",
        crate::process::port_holder(port)
    )
}

/// Whether an interface still has `ip`. Caddy cannot listen on an address that left.
pub fn present(ip: Ipv4Addr) -> bool {
    let mut list = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list`, which is walked read-only and freed once below.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return false;
    }
    let mut found = false;
    let mut entry = list;
    while !entry.is_null() {
        // SAFETY: `entry` is a node of the list getifaddrs returned.
        let interface = unsafe { &*entry };
        if !interface.ifa_addr.is_null()
            && i32::from(unsafe { (*interface.ifa_addr).sa_family }) == libc::AF_INET
        {
            // SAFETY: an AF_INET address is a sockaddr_in.
            let address = unsafe { &*(interface.ifa_addr as *const libc::sockaddr_in) };
            found |= Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr)) == ip;
        }
        entry = interface.ifa_next;
    }
    // SAFETY: `list` came from getifaddrs and is freed once.
    unsafe { libc::freeifaddrs(list) };
    found
}

fn tailscale_json(binary: &Path, args: &[&str]) -> Result<Value> {
    let mut command = Command::new(binary);
    command.args(args);
    let output = crate::process::capture(&mut command, Duration::from_secs(10), 4 << 20)?;
    ensure!(
        output.status.success(),
        "tailscale {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("unreadable output from tailscale {}", args.join(" ")))
}

fn has_capability(node: &Value, name: &str) -> bool {
    let listed = node["Capabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    let mapped = node["CapMap"]
        .as_object()
        .into_iter()
        .flat_map(|map| map.keys().map(String::as_str));
    listed.chain(mapped).any(|capability| {
        let capability = capability.to_ascii_lowercase();
        capability == name || capability.ends_with(&format!("/{name}"))
    })
}

/// Whether another process listens on `port` at one of `addresses`. It connects rather than binds:
/// without root, binding 443 fails with permission denied before the kernel checks who holds it.
fn listened(addresses: &[IpAddr], port: u16) -> bool {
    addresses.iter().any(|ip| {
        TcpStream::connect_timeout(&SocketAddr::new(*ip, port), Duration::from_millis(300)).is_ok()
    })
}

/// Every port in a `tailscale serve status --json`, background and foreground.
fn used_ports(value: &Value, used: &mut BTreeSet<u16>) {
    let Some(object) = value.as_object() else {
        return;
    };
    for (key, value) in object {
        let ports: Vec<&str> = match key.as_str() {
            "TCP" => value
                .as_object()
                .into_iter()
                .flat_map(|map| map.keys().map(String::as_str))
                .collect(),
            "Web" => value
                .as_object()
                .into_iter()
                .flat_map(|map| map.keys().filter_map(|host| host.rsplit(':').next()))
                .collect(),
            _ => Vec::new(),
        };
        used.extend(
            ports
                .into_iter()
                .filter_map(|port| port.parse::<u16>().ok()),
        );
        used_ports(value, used);
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn endpoint(service: &str, hostname: &str, port: u16) -> Endpoint {
        Endpoint {
            service: service.into(),
            hostname: hostname.into(),
            backend_port: port,
            public_url: format!("https://{hostname}:1443"),
        }
    }

    fn shared() -> Share {
        Share {
            lan: Some(Ipv4Addr::new(192, 168, 1, 20)),
            lan_port: Some(1443),
            mdns: Some("/usr/bin/dns-sd".into()),
            tailscale: Some(Tailnet {
                binary: "/usr/local/bin/tailscale".into(),
                host: "mb.tail1.ts.net".into(),
                ports: BTreeMap::from([("web".into(), 443), ("api".into(), 8443)]),
            }),
            domain: Some(Domain {
                name: "dev.example.com".into(),
                port: DOMAIN_PORT,
            }),
        }
    }

    #[test]
    fn lan_refuses_public_addresses() {
        for ip in ["192.168.1.20", "10.0.0.5", "172.16.4.1", "169.254.10.2"] {
            assert!(private_network(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["203.0.113.7", "8.8.8.8", "172.32.0.1", "100.100.1.1"] {
            assert!(!private_network(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn tailnet_ports_skip_addresses_another_process_holds() {
        let loopback = [IpAddr::V4(Ipv4Addr::LOCALHOST)];
        let holder = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = holder.local_addr().unwrap().port();
        assert!(listened(&loopback, port));
        drop(holder);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while listened(&loopback, port) {
            assert!(
                std::time::Instant::now() < deadline,
                "closed test listener still accepts connections"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn urls_and_hosts_cover_lan_tailnet_and_domain() {
        let web = endpoint("web", "notes.localhost", 24001);
        let api = endpoint("api", "api-notes.localhost", 24002);
        let share = shared();
        assert_eq!(
            share.urls(&web),
            [
                "https://notes.local:1443",
                "https://mb.tail1.ts.net",
                "https://notes.dev.example.com"
            ]
        );
        assert_eq!(
            share.urls(&api),
            [
                "https://api-notes.local:1443",
                "https://mb.tail1.ts.net:8443",
                "https://api-notes.dev.example.com"
            ]
        );
        assert_eq!(
            share.hosts(&[web, api]),
            "notes.local,api-notes.local,mb.tail1.ts.net,notes.dev.example.com,api-notes.dev.example.com"
        );
        assert_eq!(Share::default().hosts(&[]), "");
    }

    #[test]
    fn publishers_run_per_service() {
        let manifest = Manifest::parse(
            "version: 1\nproject: notes\nservices:\n  web:\n    command: serve\n    http: {healthcheck: /}\n",
        )
        .unwrap();
        let processes = shared()
            .processes(
                &manifest,
                &[endpoint("web", "notes.localhost", 24001)],
                1443,
            )
            .unwrap();
        assert_eq!(
            processes["parcela-lan-web"],
            "'/usr/bin/dns-sd' -P notes _https._tcp local 1443 notes.local 192.168.1.20"
        );
        assert_eq!(
            processes["parcela-tailscale-web"],
            "'/usr/local/bin/tailscale' serve --https=443 http://127.0.0.1:24001"
        );
    }

    #[test]
    fn project_services_cannot_take_publisher_names() {
        let manifest = Manifest::parse(
            "version: 1\nproject: notes\nservices:\n  web:\n    command: serve\n    http: {healthcheck: /}\n  parcela-lan-web:\n    command: serve\n",
        )
        .unwrap();
        let error = shared()
            .processes(&manifest, &[], 1443)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("names starting with parcela- are Parcela's"),
            "{error}"
        );
        assert!(
            Share::default()
                .processes(&manifest, &[], 1443)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn serve_status_ports_include_foreground_sessions() {
        let mut used = BTreeSet::new();
        used_ports(
            &json!({"TCP": {"443": {"HTTPS": true}}, "Foreground": {"abc": {"TCP": {"8447": {}}, "Web": {"mb.ts.net:8448": {}}}}}),
            &mut used,
        );
        assert_eq!(used, BTreeSet::from([443, 8447, 8448]));
    }

    #[test]
    fn domains_are_lowercase_names_parcela_does_not_own() {
        assert_eq!(domain_name("Dev.Example.com.").unwrap(), "dev.example.com");
        for bad in [
            "example",
            "dev..example.com",
            "-dev.example.com",
            "dev.localhost",
            "mb.local",
            "dev_x.example.com",
        ] {
            assert!(domain_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn https_capability_is_found_in_either_list() {
        assert!(has_capability(&json!({"Capabilities": ["https"]}), "https"));
        assert!(has_capability(
            &json!({"CapMap": {"https://tailscale.com/cap/https": null}}),
            "https"
        ));
        assert!(!has_capability(&json!({"Capabilities": ["ssh"]}), "https"));
    }
}

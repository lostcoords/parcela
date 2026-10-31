use crate::{
    frontdoor::FrontDoor,
    http,
    private::{self, Lock},
    process::{OwnedChild, Signals, exit_code, version},
    registry::{Endpoint, Registry, Session, SessionStatus},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const LEASE_MS: u64 = 3000;
/// The Parcela release that started a proxy, shown to people.
pub const VERSION: &str = env!("PARCELA_VERSION");
/// What a proxy and the commands that load routes into it must agree on: the proxy's own behavior
/// and the shape of its Caddy configuration. Releases share a running proxy while this matches.
/// Bump it only when that changes.
pub const PROTOCOL: u32 = 5;
/// Paths on every loopback app host that Parcela answers for that environment instead of the app.
pub const ENVIRONMENT_PATHS: &str = "/_parcela/*";

/// The running proxy: its ownership token, the front door it serves and the release that started
/// it. Unknown fields are ignored, so a newer proxy's record stays readable by older releases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Router {
    pub token: String,
    pub front_door: FrontDoor,
    pub version: String,
    pub protocol: u32,
}
impl Router {
    pub fn url(&self, host: &str) -> String {
        self.front_door.url(host)
    }
    /// Whether Caddy itself listens on `port`, so no backend may use it.
    pub fn listens_on(&self, port: u16) -> bool {
        self.front_door.https == port
    }
    fn id(&self) -> String {
        format!("parcela-{}", self.token)
    }
}
pub(crate) fn proxy_dir(state: &Path) -> PathBuf {
    state.join("proxy")
}
fn socket(state: &Path) -> Result<PathBuf> {
    crate::sockets::proxy_admin(state)
}
/// The verified running proxy. Fails when none answers on Parcela's private admin socket.
pub fn current(state: &Path) -> Result<Router> {
    let file = proxy_dir(state).join("router.json");
    let record: Value = private::read_json(&file)
        .context("the Parcela proxy is not running; start it with `parcela proxy start`")?;
    let router: Router = serde_json::from_value(record.clone()).map_err(|_| {
        anyhow::anyhow!(
            "the proxy record {} is from Parcela {}; run `parcela proxy restart`",
            file.display(),
            record["version"]
        )
    })?;
    ensure!(
        Uuid::parse_str(&router.token).is_ok(),
        "malformed router ownership record"
    );
    verify(state, &router)?;
    Ok(router)
}
fn verify(state: &Path, router: &Router) -> Result<()> {
    let (status, bytes) =
        http::admin(&socket(state)?, "GET", &format!("/id/{}", router.id()), &[])?;
    ensure!(
        status == 200,
        "Caddy ownership token does not match; refusing configuration takeover"
    );
    let object: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        object["@id"] == router.id(),
        "Caddy ownership marker was changed"
    );
    Ok(())
}
pub fn sync(registry: &Registry, router: &Router) -> Result<()> {
    let state = registry.state_path();
    let _lock = Lock::wait(&proxy_dir(state).join("updates.lock"))?;
    verify(state, router)?;
    let routes = routes(registry, router)?;
    renew_leases(registry, router, &routes)?;
    let mut config = render(registry, router, &routes, true)?;
    // A LAN or domain listener Caddy could not open never takes the loopback routes down with it.
    if FAILED_SHARE
        .lock()
        .is_ok_and(|failed| failed.as_ref() == Some(&config))
    {
        config = render(registry, router, &routes, false)?;
    }
    let cached: Result<Value> = private::read_json(&proxy_dir(state).join("loaded.json"));
    if cached.is_ok_and(|cached| cached == config) {
        return Ok(());
    }
    if let Err(error) = load(state, &config) {
        let servers = &config["apps"]["http"]["servers"];
        ensure!(
            servers.get("lan").is_some() || servers.get("domain").is_some(),
            error
        );
        eprintln!(
            "parcela: LAN and domain sharing are off; Caddy could not listen there: {error:#}"
        );
        if let Ok(mut failed) = FAILED_SHARE.lock() {
            *failed = Some(config);
        }
        config = render(registry, router, &routes, false)?;
        load(state, &config)?;
    }
    private::write_json(&proxy_dir(state).join("loaded.json"), &config)?;
    if let Ok(mut evidence) =
        private::read_json::<ProxyEvidence>(&proxy_dir(state).join("owner.json"))
    {
        ensure!(
            evidence.child.alive() && evidence.router.token == router.token,
            "proxy recovery evidence changed"
        );
        evidence.socket = private::Identity::read(&socket(state)?)?;
        private::write_json(&proxy_dir(state).join("owner.json"), &evidence)?;
    }
    Ok(())
}
/// The last configuration whose LAN or domain listener Caddy refused.
static FAILED_SHARE: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);

fn load(state: &Path, config: &Value) -> Result<()> {
    let (status, response) = http::admin(
        &socket(state)?,
        "POST",
        "/load",
        &serde_json::to_vec(config)?,
    )?;
    ensure!(
        status == 200,
        "Caddy rejected owned route update: {}",
        String::from_utf8_lossy(&response)
    );
    Ok(())
}

/// Whether the loaded proxy configuration's `server` already listens on `address`.
pub fn listening(state: &Path, server: &str, address: &str) -> bool {
    private::read_json::<Value>(&proxy_dir(state).join("loaded.json")).is_ok_and(|config| {
        config["apps"]["http"]["servers"][server]["listen"]
            .as_array()
            .is_some_and(|listen| listen.iter().any(|listened| listened == address))
    })
}
fn lease_directory(state: &Path, router: &Router, session: &Session) -> PathBuf {
    proxy_dir(state)
        .join("leases")
        .join(&router.token)
        .join(&session.token)
}
pub fn route_published(registry: &Registry, router: &Router, session: &Session) -> bool {
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return false;
    };
    let file =
        lease_directory(registry.state_path(), router, session).join(now.as_secs().to_string());
    std::fs::read(file).is_ok_and(|bytes| bytes == session.token.as_bytes())
}
/// A ready session this proxy may route: its live owner still holds the lock and every endpoint
/// listens inside the recorded supervisor tree. `dials` pairs each endpoint with its address.
struct Route {
    session: Session,
    dials: Vec<(Endpoint, std::net::SocketAddr)>,
    /// The LAN address of a run started with `--lan`.
    lan: Option<Ipv4Addr>,
    /// The front proxy's domain of a run started with `--domain`.
    domain: Option<crate::share::Domain>,
    /// Services that also take every subdomain of their host.
    wildcard: std::collections::BTreeSet<String>,
}
/// Every routable session, checked once per update with one listener snapshot.
fn routes(registry: &Registry, router: &Router) -> Result<Vec<Route>> {
    let mut candidates = Vec::new();
    for session in registry.sessions()? {
        if session.status != SessionStatus::Ready
            || session.router_token.as_ref() != Some(&router.token)
        {
            continue;
        }
        let path = registry
            .state_path()
            .join("environments")
            .join(&session.environment_id);
        if let Ok(record) = crate::owner::Record::read(&path, &session)
            && record.owner_holds(&path)
        {
            let endpoints = session.endpoints(record.manifest.default_service.as_deref());
            let wildcard = record
                .manifest
                .services
                .iter()
                .filter(|(_, service)| service.wildcard)
                .map(|(name, _)| name.clone())
                .collect();
            candidates.push((
                session,
                record.roots(),
                endpoints,
                record.share.lan,
                record.share.domain.clone(),
                wildcard,
            ));
        }
    }
    let listeners = crate::process::Listeners::inspect(candidates.iter().flat_map(
        |(_, _, endpoints, _, _, _)| endpoints.iter().map(|endpoint| endpoint.backend_port),
    ))?;
    let mut routes = Vec::new();
    for (session, roots, endpoints, lan, domain, wildcard) in candidates {
        let owned = endpoints.iter().all(|endpoint| {
            listeners
                .owned(endpoint.backend_port, &roots)
                .is_ok_and(|pids| !pids.is_empty())
        });
        if owned {
            let dials = endpoints
                .into_iter()
                .map(|endpoint| {
                    let address = listeners.address(endpoint.backend_port)?;
                    Ok((endpoint, address))
                })
                .collect::<Result<_>>()?;
            routes.push(Route {
                session,
                dials,
                lan,
                domain,
                wildcard,
            });
        }
    }
    Ok(routes)
}
fn renew_leases(registry: &Registry, router: &Router, routes: &[Route]) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    for session in registry.sessions()? {
        if session.router_token.as_ref() != Some(&router.token) {
            continue;
        }
        let directory = lease_directory(registry.state_path(), router, &session);
        let active = routes
            .iter()
            .any(|route| route.session.token == session.token);
        if active {
            private::directory(&directory)?;
            for tick in now..now + LEASE_MS / 1000 {
                let file = directory.join(tick.to_string());
                if !file.exists() {
                    private::write_bytes(&file, session.token.as_bytes())?;
                }
            }
        }
        if directory.exists() {
            private::existing_directory(&directory)?;
            for entry in std::fs::read_dir(&directory)? {
                let entry = entry?;
                if let Ok(tick) = entry.file_name().to_string_lossy().parse::<u64>()
                    && (!active || tick < now)
                {
                    use std::os::unix::fs::MetadataExt;
                    let metadata = std::fs::symlink_metadata(entry.path())?;
                    ensure!(
                        metadata.is_file() && metadata.mode() & 0o077 == 0,
                        "foreign lease artifact preserved"
                    );
                    ensure!(
                        std::fs::read(entry.path())? == session.token.as_bytes(),
                        "foreign lease artifact preserved"
                    );
                    std::fs::remove_file(entry.path())?;
                }
            }
        }
    }
    Ok(())
}
pub fn configuration(registry: &Registry, router: &Router) -> Result<Value> {
    render(registry, router, &routes(registry, router)?, true)
}
/// The whole Caddy configuration. With `shared`, runs started with `--lan` also answer on their
/// LAN address under `.local` names, on a server of their own so LAN clients reach nothing else,
/// and runs started with `--domain` answer their names in plain HTTP on a loopback server, for
/// the person's own front proxy to terminate TLS in front of it.
fn render(registry: &Registry, router: &Router, ready: &[Route], shared: bool) -> Result<Value> {
    let state = registry.state_path();
    let dashboard = format!("unix/{}", crate::sockets::dashboard(state)?.display());
    let workspaces = registry.list(None)?;
    let mut routes = Vec::new();
    let mut hosts = std::collections::BTreeSet::new();
    for workspace in &workspaces {
        hosts.extend(registry.hosts(&workspace.id)?.into_values());
    }
    if router.protocol >= 4 {
        ensure!(
            !hosts.contains(crate::dashboard::HOST),
            "{} is reserved for the Parcela dashboard; rename the conflicting environment before starting this proxy",
            crate::dashboard::HOST
        );
        hosts.insert(crate::dashboard::HOST.to_owned());
        routes.push(json!({"match":[{"host":[crate::dashboard::HOST]}], "handle":[{"handler":"reverse_proxy", "upstreams":[{"dial":dashboard.clone()}],
            "headers":{"request":{"delete":["X-Parcela-Environment", "X-Parcela-Hostname", "X-Parcela-Access"]}}}], "terminal":true}));
    }
    let hosts: Vec<_> = hosts.into_iter().collect();
    for host in &hosts {
        crate::config::dns_label(
            host.strip_suffix(".localhost")
                .context("invalid owned hostname")?,
        )?;
    }
    let mut lan_routes = Vec::new();
    let mut lan_listen = std::collections::BTreeSet::new();
    let mut lan_hosts = Vec::new();
    let mut wildcards = Vec::new();
    let mut domain_routes = Vec::new();
    let mut domain_listen = std::collections::BTreeSet::new();
    for Route {
        session,
        dials,
        lan: lan_address,
        domain,
        wildcard,
    } in ready
    {
        let workspace = workspaces
            .iter()
            .find(|workspace| workspace.id == session.environment_id)
            .context("session source missing from registry")?;
        let reserved = registry.hosts(&workspace.id)?;
        for (endpoint, address) in dials {
            ensure!(
                reserved.values().any(|host| host == &endpoint.hostname),
                "unreserved service route refused"
            );
            ensure!(
                !router.listens_on(endpoint.backend_port),
                "proxy loop rejected"
            );
            let leased = json!({"root":lease_directory(state, router, session), "try_files":["{time.now.unix}"]});
            let handle = json!([{
                "handler":"reverse_proxy", "upstreams":[{"dial":address.to_string()}],
                "headers":{"request":{"set":{"X-Parcela-Environment":[session.environment_id], "X-Parcela-Generation":[session.generation.to_string()]}}}
            }]);
            // Only loopback names reach Parcela's own paths; on LAN and domain names they are the
            // app's. Caddy always sets `X-Parcela-Access`, which the dashboard requires.
            let environment = (router.protocol >= 5).then(|| json!([{
                "handler":"reverse_proxy", "upstreams":[{"dial":dashboard}],
                "headers":{"request":{"set":{"X-Parcela-Environment":[session.environment_id], "X-Parcela-Hostname":[endpoint.hostname], "X-Parcela-Access":["local"]}}}
            }]));
            let mut matched = vec![endpoint.hostname.clone()];
            if wildcard.contains(&endpoint.service) {
                // Subdomains of an environment's host are never another environment's: those
                // join their labels with hyphens, `api-shop.localhost`.
                let tenants = format!("*.{}", endpoint.hostname);
                matched.push(tenants.clone());
                wildcards.push(tenants);
            }
            if let Some(environment) = environment {
                routes.push(json!({"match":[{"host":matched, "path":[ENVIRONMENT_PATHS], "file":leased}], "handle":environment, "terminal": true}));
            }
            routes.push(json!({"@id": format!("run-{}-{}", session.token, endpoint.service), "match":[{"host":matched, "file":leased}], "handle":handle, "terminal": true}));
            if let Some(ip) = lan_address.filter(|ip| shared && crate::share::present(*ip)) {
                let host = crate::share::local_host(&endpoint.hostname);
                lan_listen.insert(format!("{ip}:{}", router.front_door.https));
                lan_routes.push(json!({"match":[{"host":[host], "file":leased}], "handle":handle, "terminal": true}));
                lan_hosts.push(host);
            }
            if let Some(domain) = domain.as_ref().filter(|_| shared) {
                let host = domain.host(&endpoint.hostname);
                let mut matched = vec![host.clone()];
                if wildcard.contains(&endpoint.service) {
                    matched.push(format!("*.{host}"));
                }
                domain_listen.insert(format!("127.0.0.1:{}", domain.port));
                domain_routes.push(json!({"match":[{"host":matched, "file":leased}], "handle":handle, "terminal": true}));
            }
        }
    }
    if !hosts.is_empty() {
        routes.push(json!({"match":[{"host":hosts}], "handle":[{"handler":"static_response","status_code":503,"body":"Parcela environment unavailable\n"}], "terminal":true}));
    }
    routes.push(json!({"@id":router.id(), "handle":[{"handler":"static_response", "status_code":404, "body":"No ready Parcela route\n"}], "terminal":true}));
    let https_port = router.front_door.https;
    // Plain HTTP on the HTTPS port, including port 80 through the macOS relay, gets a 308 to
    // https:// with the same host and path.
    let wrappers = json!([{"wrapper":"http_redirect"}, {"wrapper":"tls"}]);
    let mut servers = json!({
        "https":{"listen":[format!("127.0.0.1:{https_port}")], "protocols":["h1","h2"],
            "listener_wrappers":wrappers, "tls_connection_policies":[{}], "automatic_https":{"disable_redirects":true}, "routes":routes}
    });
    if !lan_routes.is_empty() {
        lan_routes.push(json!({"handle":[{"handler":"static_response", "status_code":404, "body":"No shared Parcela route\n"}], "terminal":true}));
        servers["lan"] = json!({"listen":lan_listen, "protocols":["h1","h2"],
            "listener_wrappers":wrappers, "tls_connection_policies":[{}], "automatic_https":{"disable_redirects":true}, "routes":lan_routes});
    }
    if !domain_routes.is_empty() {
        domain_routes.push(json!({"handle":[{"handler":"static_response", "status_code":404, "body":"No shared Parcela route\n"}], "terminal":true}));
        // The front proxy is the client here, so its X-Forwarded-Proto and -Host reach the app,
        // which then builds https:// links to the name the browser asked for.
        servers["domain"] = json!({"listen":domain_listen, "protocols":["h1"],
            "trusted_proxies":{"source":"static", "ranges":["127.0.0.1/32", "::1/128"]},
            "automatic_https":{"disable":true}, "routes":domain_routes});
    }
    let http_app = json!({"https_port": https_port, "servers": servers});
    let mut config = json!({
        "admin":{"listen":format!("unix/{}", socket(state)?.display()), "config":{"persist":false}},
        "storage":{"module":"file_system", "root":proxy_dir(state).join("data")},
        "apps":{
            "pki":{"certificate_authorities":{"local":{"install_trust":false}}},
            "http":http_app
        }
    });
    if !hosts.is_empty() {
        let subjects: Vec<_> = hosts.iter().chain(&lan_hosts).chain(&wildcards).collect();
        config["apps"]["tls"] = json!({"automation":{"policies":[{"subjects":subjects, "issuers":[{"module":"internal"}]}]}});
    }
    Ok(config)
}
/// Run Caddy in the foreground for `front_door` until a signal arrives. Saves the profile once
/// Caddy answers, so the profile always names the last proxy that started.
pub fn run(registry: &Registry, binary: &str, front_door: FrontDoor) -> Result<i32> {
    let backend = version(binary, "version")?;
    ensure!(
        backend.starts_with("v2."),
        "unsupported Caddy version: {backend}"
    );
    let state = registry.state_path();
    let directory = proxy_dir(state);
    private::directory(&directory)?;
    // Read-only status probes briefly acquire this same lock before a daemon owns it.
    let _lifetime = Lock::wait(&directory.join("owner.lock"))?;
    let _update = Lock::wait(&directory.join("updates.lock"))?;
    let admin_socket = socket(state)?;
    crate::sockets::prepare(&admin_socket)?;
    let mut recovered = None;
    if admin_socket.exists() {
        let evidence: ProxyEvidence = private::read_json(&directory.join("owner.json"))
            .context("existing admin socket has no recovery evidence; no takeover performed")?;
        ensure!(
            evidence.socket.matches(&admin_socket),
            "admin socket was replaced; no takeover performed"
        );
        if evidence.child.alive() {
            verify(state, &evidence.router)?;
            recovered = Some(evidence.child);
        } else {
            ensure!(
                std::os::unix::net::UnixStream::connect(&admin_socket).is_err(),
                "unknown live admin socket preserved"
            );
            evidence.socket.remove_socket(&admin_socket)?;
        }
    }
    if recovered.is_none() {
        probe_listener(front_door.https)?;
    }
    let router = Router {
        token: Uuid::new_v4().to_string(),
        front_door,
        version: VERSION.into(),
        protocol: PROTOCOL,
    };
    let config = configuration(registry, &router)?;
    let _dashboard = crate::dashboard::Server::start(state, front_door)?;
    let config_path = directory.join("caddy.json");
    private::write_json(&config_path, &config)?;
    private::directory(&directory.join("data"))?;
    private::directory(&directory.join("config"))?;
    let mut command = Command::new(binary);
    command
        .args(["run", "--config"])
        .arg(&config_path)
        .current_dir(&directory)
        .env("XDG_DATA_HOME", directory.join("data"))
        .env("XDG_CONFIG_HOME", directory.join("config"));
    let signals = Signals::new()?;
    let mut child = if recovered.is_none() {
        Some(OwnedChild::spawn(&mut command)?)
    } else {
        None
    };
    if recovered.is_some() {
        let (status, response) = http::admin(
            &admin_socket,
            "POST",
            "/load",
            &serde_json::to_vec(&config)?,
        )?;
        ensure!(
            status == 200,
            "owned Caddy rejected recovery: {}",
            String::from_utf8_lossy(&response)
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.as_mut().map(OwnedChild::poll).transpose()?.flatten() {
            anyhow::bail!("Caddy exited during startup ({})", exit_code(status));
        }
        if signals.received() != 0 {
            return stop_proxy(
                &mut child,
                recovered.as_ref(),
                state,
                signals.received() as i32,
            );
        }
        if verify(state, &router).is_ok() {
            break;
        }
        ensure!(Instant::now() < deadline, "Caddy admin startup timed out");
        thread::sleep(Duration::from_millis(50));
    }
    let identity = if let Some(identity) = &recovered {
        identity.clone()
    } else {
        crate::process::Identity::read(child.as_ref().context("proxy child missing")?.id())?
            .context("proxy birth identity unavailable")?
    };
    // Saved before router.json, which is what makes this proxy read as running.
    front_door.save(state)?;
    private::write_json(
        &directory.join("owner.json"),
        &ProxyEvidence {
            daemon: crate::process::Identity::read(std::process::id())?
                .context("proxy birth identity unavailable")?,
            child: identity.clone(),
            socket: private::Identity::read(&admin_socket)?,
            router: router.clone(),
        },
    )?;
    private::write_json(&directory.join("router.json"), &router)?;
    private::write_json(&directory.join("loaded.json"), &config)?;
    drop(_update);
    eprintln!(
        "parcela: Caddy {backend} serving {} ({}); CA trust unchanged",
        router.url("<name>.localhost"),
        front_door.describe()
    );
    let mut refresh = Instant::now();
    let result = loop {
        if !identity.alive() {
            break Err(anyhow::anyhow!(
                "owned Caddy exited; applications remain independently supervised"
            ));
        }
        if let Some(status) = child.as_mut().map(OwnedChild::poll).transpose()?.flatten() {
            break Ok(exit_code(status));
        }
        if signals.received() != 0 {
            break stop_proxy(
                &mut child,
                recovered.as_ref(),
                state,
                signals.received() as i32,
            );
        }
        if Instant::now() >= refresh {
            if let Err(error) = sync(registry, &router) {
                eprintln!("parcela: route lease renewal failed; routes expire: {error:#}");
            }
            refresh = Instant::now() + Duration::from_millis(500);
        }
        thread::sleep(Duration::from_millis(100));
    };
    // Caddy normally unlinks its socket itself; no delete/recovery of unknown sockets.
    result
}

/// Who owns the running proxy: the `parcela proxy run` process, its Caddy child and admin socket.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProxyEvidence {
    pub daemon: crate::process::Identity,
    pub child: crate::process::Identity,
    pub socket: private::Identity,
    pub router: Router,
}

/// Refuse a busy port, naming who holds it. Parcela never takes a listener over or falls back.
pub fn probe_listener(port: u16) -> Result<()> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => anyhow::bail!(
            "this system only lets root listen on port {port}; Parcela runs without root, so pick a port of 1024 or higher (the default is 1443)"
        ),
        Err(_) => {}
    }
    anyhow::bail!(
        "port {port} is already in use by {}; choose another port with `parcela proxy start --port <n>`",
        crate::process::port_holder(port)
    )
}

fn stop_proxy(
    child: &mut Option<OwnedChild>,
    peer: Option<&crate::process::Identity>,
    state: &Path,
    signal: i32,
) -> Result<i32> {
    if let Some(child) = child {
        return Ok(exit_code(child.stop(signal)?));
    }
    let peer = peer.context("proxy identity missing")?;
    stop_caddy(state, &current(state)?, peer)?;
    Ok(0)
}
/// Ask the owned Caddy to exit through its private admin socket, then wait until it is gone.
pub(crate) fn stop_caddy(
    state: &Path,
    router: &Router,
    caddy: &crate::process::Identity,
) -> Result<()> {
    ensure!(caddy.alive(), "owned Caddy identity unavailable");
    verify(state, router)?;
    let (status, _) = http::admin(&socket(state)?, "POST", "/stop", &[])?;
    ensure!(status == 200, "owned Caddy rejected stop");
    let deadline = Instant::now() + Duration::from_secs(10);
    while caddy.alive() {
        ensure!(Instant::now() < deadline, "owned Caddy stop incomplete");
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

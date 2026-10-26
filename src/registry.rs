use crate::config::Manifest;
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

const VERSION: i64 = 4;
pub const NOT_STARTED: &str = "this checkout has not been started; start it with `parcela up`";
const SESSION_COLUMNS: &str =
    "environment_id, token, router_token, generation, backend_port, status, kind, public_url";
const COLUMNS: &str = "id, project_id, source, role, label, hostname, generation, vcs";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Primary,
    Preview,
}
impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Preview => "preview",
        }
    }
}

/// Where a recorded run is in its lifecycle. Stored and printed in lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Starting,
    Ready,
    Stopping,
    Stopped,
    Interrupted,
}
impl SessionStatus {
    const ALL: [Self; 5] = [
        Self::Starting,
        Self::Ready,
        Self::Stopping,
        Self::Stopped,
        Self::Interrupted,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Interrupted => "interrupted",
        }
    }
    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == text)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Workspace {
    pub id: String,
    pub project_id: String,
    pub source: PathBuf,
    pub role: Role,
    pub label: String,
    pub hostname: String,
    pub generation: i64,
    pub vcs: String,
}

fn metadata_if_present(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("inspect {}", path.display())),
    }
}

fn private_directory(path: &Path, create: bool) -> Result<()> {
    if metadata_if_present(path)?.is_none() && create {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .with_context(|| format!("create private state directory {}", path.display()))?;
    }
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect state directory {}", path.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "state directory must be a real directory, not a symlink"
    );
    // SAFETY: getuid takes no pointers and has no failure case.
    ensure!(
        metadata.uid() == unsafe { libc::getuid() },
        "state directory must belong to the current user"
    );
    ensure!(
        metadata.mode() & 0o077 == 0,
        "state directory must be private (chmod 700 {})",
        path.display()
    );
    Ok(())
}

pub struct Registry {
    connection: Connection,
    state: PathBuf,
}
impl Registry {
    /// Only adoption creates state. Inspection of absent state returns no records.
    pub fn open(state: &Path, create: bool) -> Result<Option<Self>> {
        let db = state.join("registry.sqlite3");
        if !create && metadata_if_present(state)?.is_none() {
            return Ok(None);
        }
        private_directory(state, create)?;
        if !create && metadata_if_present(&db)?.is_none() {
            return Ok(None);
        }
        if create {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&db)
            {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error).context("create private registry file"),
            }
        }
        if let Some(metadata) = metadata_if_present(&db)? {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "registry must be a regular file, not a symlink"
            );
            // SAFETY: getuid takes no pointers and has no failure case.
            ensure!(
                metadata.uid() == unsafe { libc::getuid() } && metadata.mode() & 0o077 == 0,
                "registry must be owned by the current user and private (mode 600)"
            );
        }
        let flags = if create {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let mut connection = Connection::open_with_flags(&db, flags).context("open registry")?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        if create {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let version: i64 =
                transaction.pragma_query_value(None, "user_version", |r| r.get(0))?;
            if version == 0 {
                let objects: i64 =
                    transaction
                        .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
                ensure!(
                    objects == 0,
                    "refusing to initialize a nonempty foreign database"
                );
                transaction.execute_batch("CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE);
                    CREATE TABLE workspaces (
                        id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id),
                        source TEXT NOT NULL UNIQUE, role TEXT NOT NULL CHECK(role IN ('primary','preview')),
                        label TEXT NOT NULL, hostname TEXT NOT NULL UNIQUE,
                        generation INTEGER NOT NULL CHECK(generation > 0), vcs TEXT NOT NULL,
                        UNIQUE(project_id, label));
                    CREATE UNIQUE INDEX one_primary ON workspaces(project_id) WHERE role = 'primary';
                    CREATE TABLE sessions (
                        environment_id TEXT PRIMARY KEY REFERENCES workspaces(id),
                        token TEXT NOT NULL, router_token TEXT NOT NULL, generation INTEGER NOT NULL,
                        backend_port INTEGER NOT NULL, status TEXT NOT NULL,
                        kind TEXT NOT NULL, public_url TEXT NOT NULL,
                        endpoints TEXT NOT NULL DEFAULT '[]', ports TEXT NOT NULL DEFAULT '{}');
                    CREATE TABLE workspace_hosts (hostname TEXT PRIMARY KEY, environment_id TEXT NOT NULL REFERENCES workspaces(id), service TEXT NOT NULL, UNIQUE(environment_id, service));
                    CREATE TABLE active_ports (port INTEGER PRIMARY KEY CHECK(port >= 1024 AND port <= 65535), environment_id TEXT NOT NULL REFERENCES sessions(environment_id));
                    PRAGMA user_version = 4;")?;
            } else {
                ensure!(
                    version == VERSION,
                    "unsupported registry schema {version}; no changes made"
                );
            }
            transaction.commit()?;
        } else {
            // One read transaction, so both values come from the same committed state.
            let snapshot = connection.transaction()?;
            let version: i64 = snapshot.pragma_query_value(None, "user_version", |r| r.get(0))?;
            let objects: i64 =
                snapshot.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
            snapshot.commit()?;
            // A concurrent first `up` has created the file but not yet committed its schema.
            if version == 0 && objects == 0 {
                return Ok(None);
            }
            ensure!(version == VERSION, "unsupported registry schema {version}");
        }
        Ok(Some(Self {
            connection,
            state: state.canonicalize()?,
        }))
    }

    pub fn adopt(
        &mut self,
        manifest: &Manifest,
        source: &Path,
        role: Role,
        label: &str,
    ) -> Result<Workspace> {
        manifest.validate()?;
        let hostname = manifest.hostname(role == Role::Primary, label)?;
        let source = source.canonicalize()?;
        ensure!(source.is_dir(), "source must be a checkout directory");
        let source_text = source
            .to_str()
            .context("non-UTF-8 checkout paths are not supported yet")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old_name: Option<String> = transaction
            .query_row(
                "SELECT name FROM projects WHERE id = ?1",
                [&manifest.project.id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(name) = old_name {
            ensure!(
                name == manifest.project.name,
                "project name changed; explicit rename is not implemented"
            );
        } else {
            transaction
                .execute(
                    "INSERT INTO projects (id, name) VALUES (?1, ?2)",
                    params![manifest.project.id, manifest.project.name],
                )
                .with_context(|| {
                    format!(
                        "project name {} already belongs to another project; set a different project name in parcela.yaml",
                        manifest.project.name
                    )
                })?;
        }
        let existing = transaction
            .query_row(
                &format!("SELECT {COLUMNS} FROM workspaces WHERE source = ?1"),
                [source_text],
                read_workspace,
            )
            .optional()?;
        let workspace = if let Some(workspace) = existing {
            ensure!(
                workspace.project_id == manifest.project.id
                    && workspace.role == role
                    && workspace.label == label
                    && workspace.hostname == hostname,
                "this checkout is already registered as {} {:?}; omit the naming flags to reuse it",
                workspace.role.as_str(),
                workspace.label
            );
            workspace
        } else {
            // Metadata only: jj wins in colocated repositories. No VCS commands or history mutations.
            let vcs = if source.join(".jj").is_dir() {
                "jj"
            } else if source.join(".git").exists() {
                "git"
            } else {
                "plain"
            };
            let workspace = Workspace {
                id: Uuid::new_v4().to_string(),
                project_id: manifest.project.id.clone(),
                source: source.clone(),
                role,
                label: label.to_owned(),
                hostname,
                generation: 1,
                vcs: vcs.to_owned(),
            };
            explain_conflict(&transaction, &workspace)?;
            transaction.execute("INSERT INTO workspaces (id, project_id, source, role, label, hostname, generation, vcs) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![workspace.id, workspace.project_id, source_text, role.as_str(), label, workspace.hostname, workspace.generation, vcs])
                .context("this checkout conflicts with another registered environment; choose another preview name with --name <name>")?;
            workspace
        };
        let hosts = manifest.service_hosts(&workspace.hostname)?;
        let old_hosts = {
            let mut query = transaction
                .prepare("SELECT service, hostname FROM workspace_hosts WHERE environment_id=?1")?;
            query
                .query_map([&workspace.id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<std::collections::BTreeMap<_, _>>>()?
        };
        if old_hosts != hosts {
            let active: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE environment_id=?1 AND status != 'stopped')", [&workspace.id], |row| row.get(0))?;
            ensure!(
                !active
                    || (old_hosts.len() == 1
                        && old_hosts.get("") == Some(&workspace.hostname)
                        && hosts.len() == 1),
                "service hostname changes require a stopped environment; stop it before starting the updated configuration"
            );
            transaction.execute(
                "DELETE FROM workspace_hosts WHERE environment_id=?1",
                [&workspace.id],
            )?;
            for (service, host) in hosts {
                if let Some(holder) = host_holder(&transaction, &host, &workspace.id)? {
                    bail!(
                        "{host} is already used by {holder}; choose another preview name with --name <name> or another subdomain for {service}"
                    );
                }
                transaction
                    .execute(
                        "INSERT INTO workspace_hosts VALUES (?1,?2,?3)",
                        params![host, workspace.id, service],
                    )
                    .context("service hostname is already reserved; nothing was changed")?;
            }
        }
        transaction.commit()?;
        Ok(workspace)
    }

    /// Forget a stopped environment's registration and release its hostnames and ports.
    /// Source and runtime data stay on disk; only registry rows are removed.
    pub fn forget(&mut self, environment: &str) -> Result<Workspace> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let workspace = transaction
            .query_row(
                &format!("SELECT {COLUMNS} FROM workspaces WHERE id = ?1"),
                [environment],
                read_workspace,
            )
            .optional()?
            .context("unknown environment; list environments with parcela ls")?;
        let status: Option<String> = transaction
            .query_row(
                "SELECT status FROM sessions WHERE environment_id=?1",
                [environment],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(status) = status.filter(|status| status != "stopped") {
            let name = match workspace.role {
                Role::Primary => &workspace.project_id,
                Role::Preview => &workspace.label,
            };
            bail!("{name} is {status}; stop it with `parcela down` before removing it");
        }
        transaction.execute(
            "DELETE FROM active_ports WHERE environment_id=?1",
            [environment],
        )?;
        transaction.execute(
            "DELETE FROM workspace_hosts WHERE environment_id=?1",
            [environment],
        )?;
        transaction.execute(
            "DELETE FROM sessions WHERE environment_id=?1",
            [environment],
        )?;
        transaction.execute("DELETE FROM workspaces WHERE id=?1", [environment])?;
        transaction.execute(
            "DELETE FROM projects WHERE id=?1 AND NOT EXISTS (SELECT 1 FROM workspaces WHERE project_id=?1)",
            [&workspace.project_id],
        )?;
        transaction.commit()?;
        Ok(workspace)
    }

    pub fn list(&self, project: Option<&str>) -> Result<Vec<Workspace>> {
        let mut statement = self.connection.prepare(&format!("SELECT {COLUMNS} FROM workspaces WHERE (?1 IS NULL OR project_id = ?1) ORDER BY project_id, role, label"))?;
        Ok(statement
            .query_map([project], read_workspace)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn current(&self, source: &Path, manifest: &Manifest) -> Result<Workspace> {
        let source = source.canonicalize()?;
        let source = source.to_str().context("non-UTF-8 paths are unsupported")?;
        let workspace = self
            .connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM workspaces WHERE source = ?1"),
                [source],
                read_workspace,
            )
            .optional()?;
        let Some(workspace) = workspace else {
            bail!(NOT_STARTED);
        };
        let hostname = manifest.hostname(workspace.role == Role::Primary, &workspace.label)?;
        ensure!(
            workspace.project_id == manifest.project.id && workspace.hostname == hostname,
            "parcela.yaml now gives this checkout {hostname}, but it was started as {}; forget the old one with `parcela -w {} rm`, then run `parcela up`",
            workspace.hostname,
            workspace.id
        );
        Ok(workspace)
    }

    pub fn state_path(&self) -> &Path {
        &self.state
    }

    fn version(&self) -> Result<i64> {
        Ok(self
            .connection
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    /// Check proposed URLs without changing the reservations of a running generation.
    pub fn check_hosts(&self, manifest: &Manifest, workspace: &Workspace) -> Result<()> {
        for (service, host) in manifest.service_hosts(&workspace.hostname)? {
            if let Some(holder) = host_holder(&self.connection, &host, &workspace.id)? {
                bail!("{host} is already used by {holder}; choose another subdomain for {service}");
            }
        }
        Ok(())
    }

    pub fn hosts(&self, environment: &str) -> Result<std::collections::BTreeMap<String, String>> {
        if self.version()? < 3 {
            let host = self.connection.query_row(
                "SELECT hostname FROM workspaces WHERE id=?1",
                [environment],
                |row| row.get::<_, String>(0),
            )?;
            return Ok(std::collections::BTreeMap::from([(String::new(), host)]));
        }
        let mut query = self.connection.prepare("SELECT service, hostname FROM workspace_hosts WHERE environment_id=?1 ORDER BY service")?;
        Ok(query
            .query_map([environment], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn session(&self, environment: &str) -> Result<Option<Session>> {
        let version = self.version()?;
        if version == 1 {
            return Ok(None);
        }
        let extra = session_extra(version);
        Ok(self
            .connection
            .query_row(
                &format!("SELECT {SESSION_COLUMNS}{extra} FROM sessions WHERE environment_id=?1"),
                [environment],
                read_session,
            )
            .optional()?)
    }

    pub fn sessions(&self) -> Result<Vec<Session>> {
        let version = self.version()?;
        if version == 1 {
            return Ok(Vec::new());
        }
        let extra = session_extra(version);
        let mut statement = self.connection.prepare(&format!(
            "SELECT {SESSION_COLUMNS}{extra} FROM sessions ORDER BY environment_id"
        ))?;
        Ok(statement
            .query_map([], read_session)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn begin_session(
        &mut self,
        workspace: &Workspace,
        router_token: &str,
        port: u16,
        kind: &str,
        public_url: &str,
    ) -> Result<Session> {
        self.begin_endpoints(
            workspace,
            router_token,
            port,
            kind,
            public_url,
            &[],
            &BTreeMap::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn begin_endpoints(
        &mut self,
        workspace: &Workspace,
        router_token: &str,
        port: u16,
        kind: &str,
        public_url: &str,
        endpoints: &[Endpoint],
        ports: &BTreeMap<String, u16>,
    ) -> Result<Session> {
        self.begin_run(
            workspace,
            Some((router_token, port, public_url)),
            kind,
            endpoints,
            ports,
        )
    }

    pub fn begin_processes(
        &mut self,
        workspace: &Workspace,
        ports: &BTreeMap<String, u16>,
    ) -> Result<Session> {
        self.begin_run(workspace, None, "up", &[], ports)
    }

    fn begin_run(
        &mut self,
        workspace: &Workspace,
        http: Option<(&str, u16, &str)>,
        kind: &str,
        endpoints: &[Endpoint],
        ports: &BTreeMap<String, u16>,
    ) -> Result<Session> {
        let (router_token, port, public_url) = match http {
            Some((token, port, url)) => (Some(token), Some(port), Some(url)),
            None => (None, None, None),
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reserved = {
            let mut query = transaction
                .prepare("SELECT service, hostname FROM workspace_hosts WHERE environment_id=?1")?;
            query
                .query_map([&workspace.id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<std::collections::BTreeMap<_, _>>>()?
        };
        let mut endpoint_ports = port.into_iter().collect::<std::collections::BTreeSet<_>>();
        if !endpoints.is_empty() {
            ensure!(
                endpoints
                    .iter()
                    .any(|endpoint| Some(endpoint.backend_port) == port
                        && Some(endpoint.public_url.as_str()) == public_url
                        && endpoint.hostname == workspace.hostname),
                "default endpoint aliases do not match"
            );
            endpoint_ports.clear();
            let mut services = std::collections::BTreeSet::new();
            for endpoint in endpoints {
                endpoint.validate()?;
                ensure!(
                    reserved.get(&endpoint.service) == Some(&endpoint.hostname)
                        || (endpoints.len() == 1 && reserved.get("") == Some(&endpoint.hostname)),
                    "service hostname not reserved; explicitly re-adopt the stopped environment"
                );
                ensure!(
                    endpoint_ports.insert(endpoint.backend_port)
                        && services.insert(&endpoint.service),
                    "duplicate service or backend endpoint"
                );
            }
        }
        for (name, port) in ports {
            crate::config::port_name(name)?;
            ensure!(*port >= 1024, "port {name} must be unprivileged");
        }
        let mut ports_leased = endpoint_ports;
        ensure!(
            ports.values().all(|port| ports_leased.insert(*port)),
            "duplicate service or reserved port"
        );
        ensure!(
            port.is_none_or(|port| port >= 1024),
            "backend endpoint must be unprivileged"
        );
        let previous: Option<String> = transaction
            .query_row(
                "SELECT status FROM sessions WHERE environment_id=?1",
                [&workspace.id],
                |r| r.get(0),
            )
            .optional()?;
        ensure!(
            previous.as_deref().is_none_or(|status| status == "stopped"),
            "this environment is already running or was interrupted; see `parcela status`"
        );
        transaction.execute(
            "UPDATE workspaces SET generation = generation + 1 WHERE id = ?1",
            [&workspace.id],
        )?;
        let generation = transaction.query_row(
            "SELECT generation FROM workspaces WHERE id = ?1",
            [&workspace.id],
            |r| r.get(0),
        )?;
        let session = Session {
            environment_id: workspace.id.clone(),
            token: Uuid::new_v4().to_string(),
            router_token: router_token.map(str::to_owned),
            generation,
            backend_port: port,
            status: SessionStatus::Starting,
            kind: kind.into(),
            public_url: public_url.map(str::to_owned),
            endpoints: endpoints.to_vec(),
            ports: ports.clone(),
        };
        // Schema 4's non-null HTTP aliases encode absent routing as empty strings and port 0.
        // Keep that storage encoding at the boundary; sessions and JSON use Option.
        transaction.execute("INSERT INTO sessions (environment_id, token, router_token, generation, backend_port, status, kind, public_url, endpoints, ports) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
            ON CONFLICT(environment_id) DO UPDATE SET token=excluded.token, router_token=excluded.router_token, generation=excluded.generation, backend_port=excluded.backend_port, status=excluded.status, kind=excluded.kind, public_url=excluded.public_url, endpoints=excluded.endpoints, ports=excluded.ports WHERE sessions.status='stopped'",
            params![session.environment_id, session.token, session.router_token.as_deref().unwrap_or(""), session.generation, port.unwrap_or(0), session.status.as_str(), kind, public_url.unwrap_or(""), serde_json::to_string(endpoints)?, serde_json::to_string(ports)?])
            .context("endpoint lease conflict; no foreign listener was changed")?;
        transaction.execute(
            "DELETE FROM active_ports WHERE environment_id=?1",
            [&workspace.id],
        )?;
        for port in ports_leased {
            transaction
                .execute(
                    "INSERT INTO active_ports VALUES (?1,?2)",
                    params![port, workspace.id],
                )
                .context("endpoint lease conflict; no foreign listener was changed")?;
        }
        transaction.commit()?;
        Ok(session)
    }

    pub fn session_status(&mut self, session: &Session, status: SessionStatus) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE sessions SET status=?1 WHERE environment_id=?2 AND token=?3 AND generation=?4 AND (status != 'stopped' OR ?1 = 'stopped')",
            params![
                status.as_str(),
                session.environment_id,
                session.token,
                session.generation
            ],
        )?;
        ensure!(
            changed == 1,
            "session ownership changed; refusing stale update"
        );
        if status == SessionStatus::Stopped {
            transaction.execute(
                "DELETE FROM active_ports WHERE environment_id=?1",
                [&session.environment_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn rebind_router(&mut self, session: &Session, token: &str) -> Result<()> {
        ensure!(Uuid::parse_str(token).is_ok(), "invalid router epoch");
        let changed = self.connection.execute("UPDATE sessions SET router_token=?1 WHERE environment_id=?2 AND token=?3 AND generation=?4 AND status IN ('ready','starting')", params![token, session.environment_id, session.token, session.generation])?;
        ensure!(
            changed == 1,
            "application ownership changed during route recovery"
        );
        Ok(())
    }
    pub fn runtime_path(&self, workspace: &Workspace) -> PathBuf {
        self.state.join("environments").join(&workspace.id)
    }
}

/// The checkout that already holds a hostname, other than `environment` itself.
fn host_holder(transaction: &Connection, host: &str, environment: &str) -> Result<Option<String>> {
    Ok(transaction
        .query_row(
            "SELECT w.source FROM workspaces w LEFT JOIN workspace_hosts h ON h.environment_id = w.id
             WHERE (w.hostname = ?1 OR h.hostname = ?1) AND w.id != ?2 LIMIT 1",
            params![host, environment],
            |row| row.get(0),
        )
        .optional()?)
}

/// Name the registration that blocks `workspace` and how to avoid it, before SQLite refuses it.
fn explain_conflict(transaction: &rusqlite::Transaction<'_>, workspace: &Workspace) -> Result<()> {
    if workspace.role == Role::Primary
        && let Some(source) = transaction
            .query_row(
                "SELECT source FROM workspaces WHERE project_id = ?1 AND role = 'primary'",
                [&workspace.project_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
    {
        bail!(
            "{} already has a primary checkout at {source}; start this one as a preview with --name <name>",
            workspace.project_id
        );
    }
    if let Some(source) = transaction
        .query_row(
            "SELECT source FROM workspaces WHERE project_id = ?1 AND label = ?2",
            params![workspace.project_id, workspace.label],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        bail!(
            "preview name {} is already used by {source}; choose another with --name <name>",
            workspace.label
        );
    }
    if let Some(holder) = host_holder(transaction, &workspace.hostname, &workspace.id)? {
        bail!(
            "{} is already used by {holder}; choose another preview name with --name <name>",
            workspace.hostname
        );
    }
    Ok(())
}

fn read_workspace(row: &rusqlite::Row<'_>) -> rusqlite::Result<Workspace> {
    let role: String = row.get(3)?;
    let role = match role.as_str() {
        "primary" => Role::Primary,
        "preview" => Role::Preview,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let id: String = row.get(0)?;
    // Never let malformed persisted identity become a runtime path traversal.
    if Uuid::parse_str(&id).is_err() || id.len() != 36 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let generation: i64 = row.get(6)?;
    if generation <= 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(Workspace {
        id,
        project_id: row.get(1)?,
        source: PathBuf::from(row.get::<_, String>(2)?),
        role,
        label: row.get(4)?,
        hostname: row.get(5)?,
        generation,
        vcs: row.get(7)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub service: String,
    pub hostname: String,
    pub backend_port: u16,
    pub public_url: String,
}
impl Endpoint {
    pub fn validate(&self) -> Result<()> {
        crate::config::dns_label(&self.service)?;
        crate::config::dns_label(
            self.hostname
                .strip_suffix(".localhost")
                .context("invalid service hostname")?,
        )?;
        ensure!(
            self.backend_port >= 1024,
            "backend endpoint must be unprivileged"
        );
        let tail = self
            .public_url
            .strip_prefix(&format!("https://{}", self.hostname))
            .context("invalid service URL")?;
        ensure!(
            tail.is_empty()
                || tail
                    .strip_prefix(':')
                    .is_some_and(|port| port.parse::<u16>().is_ok_and(|port| port > 0)),
            "invalid service URL port"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub environment_id: String,
    pub token: String,
    pub router_token: Option<String>,
    pub generation: i64,
    pub backend_port: Option<u16>,
    pub status: SessionStatus,
    pub kind: String,
    pub public_url: Option<String>,
    pub endpoints: Vec<Endpoint>,
    /// Named non-HTTP ports leased with the endpoints; kept after stop so the next run can reuse them.
    pub ports: BTreeMap<String, u16>,
}
impl Session {
    pub fn endpoints(&self, default_service: Option<&str>) -> Vec<Endpoint> {
        if self.endpoints.is_empty() {
            match (default_service, self.backend_port, self.public_url.as_ref()) {
                (Some(service), Some(port), Some(url)) => vec![Endpoint {
                    service: service.into(),
                    hostname: url
                        .trim_start_matches("https://")
                        .split(':')
                        .next()
                        .unwrap_or_default()
                        .into(),
                    backend_port: port,
                    public_url: url.clone(),
                }],
                _ => vec![],
            }
        } else {
            self.endpoints.clone()
        }
    }
    /// Every port this session leases: its HTTP endpoints and its named ports.
    pub fn leased_ports(&self) -> Vec<u16> {
        let endpoints: Vec<u16> = if self.endpoints.is_empty() {
            self.backend_port.into_iter().collect()
        } else {
            self.endpoints
                .iter()
                .map(|endpoint| endpoint.backend_port)
                .collect()
        };
        endpoints
            .into_iter()
            .chain(self.ports.values().copied())
            .collect()
    }
}
fn session_extra(version: i64) -> &'static str {
    match version {
        4.. => ", endpoints, ports",
        3 => ", endpoints, '{}'",
        _ => ", '[]', '{}'",
    }
}
fn read_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    let session = Session {
        environment_id: row.get(0)?,
        token: row.get(1)?,
        router_token: Some(row.get::<_, String>(2)?).filter(|token| !token.is_empty()),
        generation: row.get(3)?,
        backend_port: Some(row.get::<_, u16>(4)?).filter(|port| *port != 0),
        status: SessionStatus::parse(&row.get::<_, String>(5)?)
            .ok_or(rusqlite::Error::InvalidQuery)?,
        kind: row.get(6)?,
        public_url: Some(row.get::<_, String>(7)?).filter(|url| !url.is_empty()),
        endpoints: serde_json::from_str(&row.get::<_, String>(8)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        ports: serde_json::from_str(&row.get::<_, String>(9)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    };
    if [&session.environment_id, &session.token]
        .iter()
        .any(|id| Uuid::parse_str(id).is_err())
        || session.generation <= 0
        || session.backend_port.is_some_and(|port| port < 1024)
        || session
            .router_token
            .as_ref()
            .is_some_and(|token| Uuid::parse_str(token).is_err())
        || session.router_token.is_some() != session.backend_port.is_some()
        || session.public_url.is_some() != session.backend_port.is_some()
        || session
            .endpoints
            .iter()
            .any(|endpoint| endpoint.validate().is_err())
        || session
            .endpoints
            .iter()
            .map(|endpoint| &endpoint.service)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != session.endpoints.len()
        || session
            .ports
            .iter()
            .any(|(name, port)| crate::config::port_name(name).is_err() || *port < 1024)
        || session
            .leased_ports()
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != session.leased_ports().len()
        || (!session.endpoints.is_empty()
            && !session.endpoints.iter().any(|endpoint| {
                Some(endpoint.backend_port) == session.backend_port
                    && Some(&endpoint.public_url) == session.public_url.as_ref()
            }))
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(session)
}

//! Runs one generation's processes: services, hooks and sharing publishers. Each process gets its
//! own process group and log file. A process starts once its dependencies have started, answered
//! their readiness check or, for hooks, exited 0. Shutdown goes in reverse dependency order.
//!
//! The owner ticks the graph. If the owner dies, the processes keep running, and the next owner
//! adopts them by their recorded birth identity and signals them only while that identity holds.
use crate::{
    config::Manifest,
    process::{self, Identity, OwnedChild},
    registry::Endpoint,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::os::unix::process::CommandExt;
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Service,
    Hook,
    /// `--lan` and `--tailscale` publishers, retried when they fail.
    Publisher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Started,
    Healthy,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    Http {
        port: u16,
        host: String,
        path: String,
    },
    Exec(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    /// Run with `bash -c`, or `sh -c` where there is no bash.
    Shell(String),
    /// `parcela run -- argv`, run as is.
    Argv(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub kind: Kind,
    pub program: Program,
    pub working_dir: PathBuf,
    /// The process's whole environment from Parcela; inherited `PARCELA_` variables are removed.
    pub env: BTreeMap<String, String>,
    pub depends_on: Vec<(String, Condition)>,
    /// Checked until the service passes its explicit startup readiness check.
    pub probe: Option<Probe>,
    pub max_restarts: u32,
    /// Run when the graph stops, before the process is signaled, for anything the process
    /// started outside its own process group, like containers.
    pub stop: Option<String>,
}

/// The graph for one run of `up`. Commands reach the shell unexpanded, so `$PORT` is each HTTP
/// service's own variable. Hooks run after the services they depend on, and every other service
/// waits for the last hook. `env` is the run's environment; each HTTP service adds its `PORT`.
pub fn plan(
    manifest: &Manifest,
    endpoints: &[Endpoint],
    env: &BTreeMap<String, String>,
    source: &Path,
    runtime: &Path,
    setup: bool,
    shared: &BTreeMap<String, String>,
) -> Result<Vec<Node>> {
    let mut env = env.clone();
    env.remove("PORT");
    let condition = |name: &str| {
        let ready =
            manifest.services.contains_key(name) || manifest.processes[name].healthcheck.is_some();
        if ready {
            Condition::Healthy
        } else {
            Condition::Started
        }
    };
    let mut nodes = Vec::new();
    let mut last_hook = None;
    for (name, hook) in manifest.hooks.iter() {
        if name == "setup" && !setup {
            continue;
        }
        let mut depends_on: Vec<_> = hook
            .depends_on
            .iter()
            .map(|dependency| (dependency.clone(), condition(dependency)))
            .collect();
        if let Some(previous) = last_hook.replace(name) {
            depends_on.push((previous.into(), Condition::Completed));
        }
        // The exit code outlives this owner, so a successor that adopts the hook still learns it.
        let exit = hook_exit(runtime, name);
        let exit = exit
            .to_str()
            .context("non-UTF-8 runtime path")?
            .replace('\'', "'\\''");
        nodes.push(Node {
            name: name.into(),
            kind: Kind::Hook,
            program: Program::Shell(format!(
                "(\n{}\n)\nstatus=$?\nprintf %s \"$status\" > '{exit}'\nexit \"$status\"",
                hook.command
            )),
            working_dir: source.to_owned(),
            env: env.clone(),
            depends_on,
            probe: None,
            max_restarts: 0,
            stop: None,
        });
    }
    let before_hooks = manifest.hook_dependencies();
    for (name, process) in &manifest.processes {
        let mut environment = env.clone();
        let endpoint = endpoints.iter().find(|endpoint| &endpoint.service == name);
        if let Some(endpoint) = endpoint {
            environment.insert("PORT".into(), endpoint.backend_port.to_string());
        }
        for (key, value) in &process.env {
            let resolved = crate::runtime::resolve(key, value, &env)?;
            environment.insert(key.clone(), resolved);
        }
        let probe = match (endpoint, &process.healthcheck) {
            (Some(endpoint), _) => Some(Probe::Http {
                port: endpoint.backend_port,
                host: endpoint.hostname.clone(),
                path: manifest.services[name].readiness_path.clone(),
            }),
            (None, Some(healthcheck)) => Some(Probe::Exec(healthcheck.clone())),
            (None, None) => None,
        };
        let mut depends_on: Vec<_> = process
            .depends_on
            .iter()
            .map(|dependency| (dependency.clone(), condition(dependency)))
            .collect();
        if let Some(hook) = last_hook.filter(|_| !before_hooks.contains(name.as_str())) {
            depends_on.push((hook.into(), Condition::Completed));
        }
        nodes.push(Node {
            name: name.clone(),
            kind: Kind::Service,
            program: Program::Shell(process.command.clone()),
            working_dir: process
                .working_dir
                .as_ref()
                .map_or_else(|| source.to_owned(), |dir| source.join(dir)),
            env: environment,
            depends_on,
            probe,
            max_restarts: 0,
            stop: process.stop.clone(),
        });
    }
    // Concurrent `tailscale serve` changes can lose a race on the serve config, so a failed
    // publisher tries again.
    for (name, command) in shared {
        nodes.push(Node {
            name: name.clone(),
            kind: Kind::Publisher,
            program: Program::Shell(command.clone()),
            working_dir: source.to_owned(),
            env: env.clone(),
            depends_on: Vec::new(),
            probe: None,
            max_restarts: 10,
            stop: None,
        });
    }
    Ok(nodes)
}

/// The one process of `parcela run -- argv`.
pub fn literal(
    service: &str,
    argv: &[String],
    env: &BTreeMap<String, String>,
    source: &Path,
) -> Node {
    Node {
        name: service.into(),
        kind: Kind::Service,
        program: Program::Argv(argv.to_vec()),
        working_dir: source.to_owned(),
        env: env.clone(),
        depends_on: Vec::new(),
        probe: None,
        max_restarts: 0,
        stop: None,
    }
}

/// Where a hook writes its exit code when it finishes.
pub fn hook_exit(runtime: &Path, name: &str) -> PathBuf {
    runtime.join("hooks").join(format!("{name}.exit"))
}

/// A hook's code from its exit file; unknown for anything else.
fn hook_code(runtime: &Path, node: &Node) -> Option<i32> {
    if node.kind != Kind::Hook {
        return None;
    }
    std::fs::read_to_string(hook_exit(runtime, &node.name))
        .ok()?
        .parse()
        .ok()
}

/// What the owner records for each process that started, so a successor can adopt it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Child {
    /// None when the process exited before its identity could be read.
    pub identity: Option<Identity>,
    /// Known once this owner reaped it, or from a hook's exit file.
    pub exit: Option<i32>,
    pub restarts: u32,
    /// Its `stop` command succeeded, so a successor does not run it again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,
}

/// How long a process has between SIGTERM and SIGKILL.
const STOP_GRACE: Duration = Duration::from_secs(10);
/// How long an exec readiness check may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_PERIOD: Duration = Duration::from_secs(1);
const RESTART_BACKOFF: Duration = Duration::from_secs(1);
/// How long a `stop` command may take before its process group is killed.
const STOP_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

enum State {
    Pending,
    /// Waiting to restart.
    Backoff(Instant),
    Running(Running),
    /// The exit code, unknown for an adopted process that is gone.
    Exited(Option<i32>),
    /// A dependency stopped before it could satisfy this process.
    Skipped,
}
enum Running {
    Own(OwnedChild),
    Adopted,
}
/// A node's `stop` command.
enum Stop {
    Due,
    Running(OwnedChild, Instant),
    /// The exit code; `None` when it was killed or could not start.
    Done(Option<i32>),
}
struct Slot {
    state: State,
    identity: Option<Identity>,
    started: bool,
    healthy: bool,
    restarts: u32,
    next_probe: Instant,
    probe: Option<(OwnedChild, Instant)>,
    terminated: Option<Instant>,
    stop: Stop,
}

pub struct Supervisor {
    nodes: Vec<Node>,
    slots: Vec<Slot>,
    runtime: PathBuf,
    logs: PathBuf,
    shell: PathBuf,
    stopping: bool,
}
impl Supervisor {
    /// A graph whose processes have not started, in the environment's runtime directory. The
    /// previous run's logs are removed.
    pub fn new(nodes: Vec<Node>, runtime: &Path) -> Result<Self> {
        crate::logs::reset(&crate::logs::directory(runtime))?;
        Ok(Self::adopt(nodes, runtime, &BTreeMap::new()))
    }

    /// The graph of a run whose owner died. Recorded processes that are still alive are adopted;
    /// the others count as exited, a hook with the code from its exit file.
    pub fn adopt(nodes: Vec<Node>, runtime: &Path, children: &BTreeMap<String, Child>) -> Self {
        let now = Instant::now();
        let slots = nodes
            .iter()
            .map(|node| {
                let child = children.get(&node.name);
                let state = match child {
                    None => State::Pending,
                    Some(child) if child.exit.is_some() => State::Exited(child.exit),
                    Some(child) if child.identity.as_ref().is_some_and(Identity::alive) => {
                        State::Running(Running::Adopted)
                    }
                    Some(_) => State::Exited(hook_code(runtime, node)),
                };
                Slot {
                    state,
                    identity: child.and_then(|child| child.identity.clone()),
                    started: child.is_some(),
                    healthy: false,
                    restarts: child.map_or(0, |child| child.restarts),
                    next_probe: now,
                    probe: None,
                    terminated: None,
                    stop: if child.is_some_and(|child| child.stopped) {
                        Stop::Done(Some(0))
                    } else {
                        Stop::Due
                    },
                }
            })
            .collect();
        Self {
            nodes,
            slots,
            runtime: runtime.to_owned(),
            logs: crate::logs::directory(runtime),
            shell: process::on_path("bash").unwrap_or_else(|| "/bin/sh".into()),
            stopping: false,
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.nodes.iter().map(|node| node.name.as_str())
    }

    /// The processes that started, for the owner record.
    pub fn children(&self) -> BTreeMap<String, Child> {
        self.nodes
            .iter()
            .zip(&self.slots)
            .filter(|(_, slot)| slot.started)
            .map(|(node, slot)| {
                (
                    node.name.clone(),
                    Child {
                        identity: slot.identity.clone(),
                        exit: match slot.state {
                            State::Exited(code) => code,
                            _ => None,
                        },
                        restarts: slot.restarts,
                        stopped: matches!(slot.stop, Stop::Done(Some(0))),
                    },
                )
            })
            .collect()
    }

    /// The live processes this graph started, as roots of its process tree.
    pub fn roots(&self) -> Vec<u32> {
        self.slots
            .iter()
            .filter(|slot| matches!(slot.state, State::Running(_)))
            .filter_map(|slot| slot.identity.as_ref().map(|identity| identity.pid))
            .collect()
    }

    /// Advances the graph once: reaps exits, restarts publishers, checks readiness, starts what
    /// is unblocked and skips what never can start. Calls `started` right after each spawn, so the
    /// owner records a process before the next one starts. Returns whether the record changed.
    pub fn tick(&mut self, mut started: impl FnMut(&Self) -> Result<()>) -> Result<bool> {
        let mut changed = self.reap()?;
        if self.stopping {
            return Ok(self.stop_tick()? || changed);
        }
        let now = Instant::now();
        for slot in &mut self.slots {
            if let State::Backoff(until) = slot.state
                && now >= until
            {
                slot.state = State::Pending;
            }
        }
        self.probe()?;
        loop {
            let mut progressed = false;
            for index in 0..self.nodes.len() {
                if !matches!(self.slots[index].state, State::Pending) {
                    continue;
                }
                match self.blocked(index) {
                    Some(true) => {
                        self.slots[index].state = State::Skipped;
                        progressed = true;
                    }
                    Some(false) => {}
                    None => {
                        self.spawn(index)?;
                        started(self)?;
                        progressed = true;
                    }
                }
            }
            if !progressed {
                break;
            }
            changed = true;
        }
        Ok(changed)
    }

    /// `None` when every dependency is satisfied, `Some(true)` when one never can be.
    fn blocked(&self, index: usize) -> Option<bool> {
        let mut waiting = false;
        for (dependency, condition) in &self.nodes[index].depends_on {
            let Some(position) = self.position(dependency) else {
                return Some(true);
            };
            let slot = &self.slots[position];
            let terminal = matches!(slot.state, State::Exited(_) | State::Skipped);
            let (met, never) = match condition {
                Condition::Started => (slot.started, terminal && !slot.started),
                Condition::Healthy => (slot.healthy, terminal && !slot.healthy),
                Condition::Completed => (
                    matches!(slot.state, State::Exited(Some(0))),
                    terminal && !matches!(slot.state, State::Exited(Some(0))),
                ),
            };
            if never {
                return Some(true);
            }
            waiting |= !met;
        }
        waiting.then_some(false)
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.name == name)
    }

    fn spawn(&mut self, index: usize) -> Result<()> {
        let node = &self.nodes[index];
        let mut command = match &node.program {
            Program::Shell(script) => {
                let mut command = Command::new(&self.shell);
                command.arg("-c").arg(script);
                command
            }
            Program::Argv(argv) => {
                let mut command = Command::new(
                    argv.first()
                        .context("literal command argv required after --")?,
                );
                command.args(&argv[1..]);
                command
            }
        };
        command.current_dir(&node.working_dir);
        crate::runtime::command_env(&mut command, &node.env);
        let log = crate::logs::open(&self.logs, &node.name)?;
        let child = OwnedChild::spawn_logged(&mut command, log)
            .with_context(|| format!("start {}", node.name))?;
        let identity = Identity::read(child.id())?;
        let slot = &mut self.slots[index];
        slot.identity = identity;
        slot.started = true;
        slot.state = State::Running(Running::Own(child));
        Ok(())
    }

    /// Records the processes that exited since the last tick.
    fn reap(&mut self) -> Result<bool> {
        let mut changed = false;
        for (node, slot) in self.nodes.iter().zip(&mut self.slots) {
            let exit = match &mut slot.state {
                State::Running(Running::Own(child)) => {
                    child.poll()?.map(|status| Some(process::exit_code(status)))
                }
                State::Running(Running::Adopted) => {
                    (!slot.identity.as_ref().is_some_and(Identity::alive))
                        .then(|| hook_code(&self.runtime, node))
                }
                _ => None,
            };
            let Some(code) = exit else {
                continue;
            };
            changed = true;
            if let Some((child, _)) = slot.probe.take() {
                child.kill_now();
            }
            if !self.stopping && code != Some(0) && slot.restarts < node.max_restarts {
                slot.restarts += 1;
                slot.state = State::Backoff(Instant::now() + RESTART_BACKOFF);
                continue;
            }
            slot.state = State::Exited(code);
            if !self.stopping && node.kind == Kind::Publisher && code != Some(0) {
                eprintln!(
                    "parcela: {} stopped after {} restarts; that share is no longer published",
                    node.name, slot.restarts
                );
            }
            if !self.stopping && node.kind == Kind::Service {
                match code {
                    Some(code) => eprintln!("parcela: {} exited with code {code}", node.name),
                    None => eprintln!("parcela: {} exited", node.name),
                }
            }
        }
        Ok(changed)
    }

    /// Checks explicit readiness of running processes until each first passes.
    fn probe(&mut self) -> Result<()> {
        let now = Instant::now();
        for index in 0..self.nodes.len() {
            let awaited = self.nodes[index].probe.is_some()
                || self.nodes.iter().zip(&self.slots).any(|(node, slot)| {
                    matches!(slot.state, State::Pending)
                        && node.depends_on.iter().any(|(name, condition)| {
                            *name == self.nodes[index].name && *condition == Condition::Healthy
                        })
                });
            let node = &self.nodes[index];
            let slot = &mut self.slots[index];
            if slot.healthy || !awaited || !matches!(slot.state, State::Running(_)) {
                if let Some((child, _)) = slot.probe.take() {
                    child.kill_now();
                }
                continue;
            }
            if let Some((child, started)) = &mut slot.probe {
                if let Some(status) = child.poll()? {
                    slot.healthy = status.success();
                    slot.probe = None;
                } else if started.elapsed() >= PROBE_TIMEOUT
                    && let Some((child, _)) = slot.probe.take()
                {
                    child.kill_now();
                }
                continue;
            }
            if now < slot.next_probe {
                continue;
            }
            slot.next_probe = now + PROBE_PERIOD;
            match &node.probe {
                Some(Probe::Http { port, host, path }) => {
                    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, *port));
                    let v6 = SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, *port));
                    slot.healthy = crate::http::readiness(address, host, path)
                        .or_else(|_| crate::http::readiness(v6, host, path))
                        .unwrap_or(false);
                }
                Some(Probe::Exec(check)) => {
                    let mut command = Command::new(&self.shell);
                    command
                        .arg("-c")
                        .arg(check)
                        .current_dir(&node.working_dir)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .process_group(0);
                    crate::runtime::command_env(&mut command, &node.env);
                    let child =
                        OwnedChild::from_child(command.spawn().context("start a readiness check")?);
                    slot.probe = Some((child, now));
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Processes still awaiting startup, explicit readiness, or hook completion.
    pub fn waiting(&self) -> Vec<String> {
        self.nodes
            .iter()
            .zip(&self.slots)
            .filter_map(|(node, slot)| {
                let ready = match &slot.state {
                    State::Exited(Some(0)) => true,
                    State::Running(_) => {
                        node.kind != Kind::Hook && (node.probe.is_none() || slot.healthy)
                    }
                    _ => false,
                };
                (!ready).then(|| format!("{}: waiting for startup or healthcheck", node.name))
            })
            .collect()
    }

    /// Whether a hook is running. One still waiting for a dependency does not count, so the
    /// readiness timeout still covers a dependency that never becomes ready.
    pub fn hooks_running(&self) -> bool {
        self.nodes
            .iter()
            .zip(&self.slots)
            .any(|(node, slot)| node.kind == Kind::Hook && matches!(slot.state, State::Running(_)))
    }

    /// Why the run cannot become ready: a hook that failed or could not run, or, until the run
    /// was first ready (`starting`), an HTTP service that stopped, a process that failed, or one
    /// that could not start. A literal `run` reports its exit code through `finished` instead.
    pub fn failure(&self, starting: bool) -> Option<String> {
        self.nodes.iter().zip(&self.slots).find_map(|(node, slot)| {
            let dependency = || {
                node.depends_on
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .find(|name| {
                        self.position(name).is_some_and(|position| {
                            matches!(
                                self.slots[position].state,
                                State::Exited(_) | State::Skipped
                            )
                        })
                    })
                    .unwrap_or("a dependency")
                    .to_owned()
            };
            match (node.kind, &slot.state) {
                (Kind::Hook, State::Exited(Some(0))) => None,
                (Kind::Hook, State::Exited(Some(code))) => Some(format!(
                    "hooks.{} failed with exit code {code}; its output is above",
                    node.name
                )),
                (Kind::Hook, State::Exited(None)) => {
                    Some(format!("hooks.{} stopped before it finished", node.name))
                }
                (Kind::Hook, State::Skipped) => Some(format!(
                    "hooks.{} did not run because {} stopped",
                    node.name,
                    dependency()
                )),
                _ if !starting || matches!(node.program, Program::Argv(_)) => None,
                // A one-shot process, like an asset build, may finish during startup.
                (Kind::Service, State::Exited(Some(0) | None))
                    if !matches!(node.probe, Some(Probe::Http { .. })) =>
                {
                    None
                }
                (Kind::Service, State::Exited(code)) => Some(format!(
                    "{} exited{} before it was ready; its output is above",
                    node.name,
                    code.map(|code| format!(" with code {code}"))
                        .unwrap_or_default()
                )),
                (Kind::Service, State::Skipped) => Some(format!(
                    "{} did not start because {} stopped",
                    node.name,
                    dependency()
                )),
                _ => None,
            }
        })
    }

    /// The exit code once every process has exited and, while stopping, every `stop` command
    /// finished: a literal `run`'s own, else the first nonzero one.
    pub fn finished(&self) -> Option<i32> {
        if !self
            .slots
            .iter()
            .all(|slot| matches!(slot.state, State::Exited(_) | State::Skipped))
            || (self.stopping && (0..self.nodes.len()).any(|index| self.stop_due(index)))
        {
            return None;
        }
        let codes = self
            .nodes
            .iter()
            .zip(&self.slots)
            .filter_map(|(node, slot)| match slot.state {
                State::Exited(code) => Some((node, code.unwrap_or(1))),
                _ => None,
            });
        let mut first = 0;
        for (node, code) in codes {
            if matches!(node.program, Program::Argv(_)) {
                return Some(code);
            }
            if first == 0 {
                first = code;
            }
        }
        Some(first)
    }

    /// Stops starting processes and begins the ordered shutdown, which `tick` carries on.
    pub fn stop(&mut self) {
        self.stopping = true;
    }

    /// Whether a node still has to run its `stop` command: it has one, started and has not
    /// finished stopping.
    fn stop_due(&self, index: usize) -> bool {
        let slot = &self.slots[index];
        self.nodes[index].stop.is_some()
            && slot.started
            && matches!(slot.stop, Stop::Due | Stop::Running(..))
    }

    /// Why a `stop` command did not finish the shutdown, once the graph stopped.
    pub fn stop_failure(&self) -> Option<String> {
        self.nodes
            .iter()
            .zip(&self.slots)
            .find_map(|(node, slot)| match slot.stop {
                Stop::Done(Some(0)) | Stop::Due | Stop::Running(..) => None,
                Stop::Done(Some(code)) => Some(format!(
                    "services.{}.stop failed with exit code {code}; see parcela logs {}",
                    node.name, node.name
                )),
                Stop::Done(None) => Some(format!(
                    "services.{}.stop did not finish; see parcela logs {}",
                    node.name, node.name
                )),
            })
    }

    /// Starts, polls or times out a node's `stop` command. Returns whether it finished.
    fn stop_command(&mut self, index: usize) -> Result<bool> {
        let node = &self.nodes[index];
        let slot = &mut self.slots[index];
        if let Stop::Running(child, started) = &mut slot.stop {
            let code = match child.poll()? {
                Some(status) => Some(process::exit_code(status)),
                None if started.elapsed() < STOP_COMMAND_TIMEOUT => return Ok(false),
                None => None,
            };
            if let Stop::Running(child, _) = std::mem::replace(&mut slot.stop, Stop::Done(code))
                && code.is_none()
            {
                child.kill_now();
            }
            return Ok(true);
        }
        let mut command = Command::new(&self.shell);
        command
            .arg("-c")
            .arg(node.stop.as_deref().unwrap_or_default())
            .current_dir(&node.working_dir);
        crate::runtime::command_env(&mut command, &node.env);
        let spawned = crate::logs::open(&self.logs, &node.name)
            .and_then(|log| OwnedChild::spawn_logged(&mut command, log));
        slot.stop = match spawned {
            Ok(child) => Stop::Running(child, Instant::now()),
            Err(error) => {
                eprintln!(
                    "parcela: could not run services.{}.stop: {error:#}",
                    node.name
                );
                Stop::Done(None)
            }
        };
        Ok(matches!(slot.stop, Stop::Done(_)))
    }

    /// Stops each process once nothing that depends on it is still running or stopping: first
    /// its `stop` command, if it has one, then SIGTERM, and SIGKILL after the grace period. A
    /// process that already exited still gets its `stop` command. Returns whether the record
    /// changed.
    fn stop_tick(&mut self) -> Result<bool> {
        let mut changed = false;
        for slot in &mut self.slots {
            if matches!(slot.state, State::Pending | State::Backoff(_)) {
                slot.state = State::Skipped;
                changed = true;
            }
        }
        for index in 0..self.nodes.len() {
            let running = matches!(self.slots[index].state, State::Running(_));
            if !running && !self.stop_due(index) {
                continue;
            }
            let name = &self.nodes[index].name;
            let dependents_active = (0..self.nodes.len()).any(|other| {
                self.nodes[other]
                    .depends_on
                    .iter()
                    .any(|(dependency, _)| dependency == name)
                    && (matches!(self.slots[other].state, State::Running(_))
                        || self.stop_due(other))
            });
            if dependents_active {
                continue;
            }
            if self.stop_due(index) {
                changed |= self.stop_command(index)?;
                continue;
            }
            let slot = &mut self.slots[index];
            let signal = match slot.terminated {
                None => libc::SIGTERM,
                Some(sent) if sent.elapsed() >= STOP_GRACE => libc::SIGKILL,
                Some(_) => continue,
            };
            if signal == libc::SIGTERM {
                slot.terminated = Some(Instant::now());
            }
            match &mut slot.state {
                State::Running(Running::Own(child)) => {
                    let _ = child.signal(signal, true);
                }
                State::Running(Running::Adopted) => {
                    if let Some(identity) = &slot.identity {
                        process::signal_verified(identity, signal, true)?;
                    }
                }
                _ => {}
            }
        }
        Ok(changed)
    }

    /// `parcela status`'s process list: name, state and pid.
    pub fn list(&self) -> serde_json::Value {
        let processes: Vec<_> = self
            .nodes
            .iter()
            .zip(&self.slots)
            .map(|(node, slot)| {
                let state = match &slot.state {
                    State::Pending => "pending",
                    State::Backoff(_) => "restarting",
                    State::Running(_) if slot.terminated.is_some() => "stopping",
                    State::Running(_) => "running",
                    State::Exited(Some(0)) => "completed",
                    State::Exited(_) => "exited",
                    State::Skipped => "skipped",
                };
                let pid = match slot.state {
                    State::Running(_) => slot.identity.as_ref().map(|identity| identity.pid),
                    _ => None,
                };
                serde_json::json!({"name": node.name, "status": state, "pid": pid})
            })
            .collect();
        serde_json::json!({ "services": processes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, script: &str, depends_on: &[(&str, Condition)]) -> Node {
        Node {
            name: name.into(),
            kind: if name.starts_with("hook") {
                Kind::Hook
            } else {
                Kind::Service
            },
            program: Program::Shell(script.into()),
            working_dir: std::env::temp_dir(),
            env: BTreeMap::new(),
            depends_on: depends_on
                .iter()
                .map(|(name, condition)| ((*name).into(), *condition))
                .collect(),
            probe: None,
            max_restarts: 0,
            stop: None,
        }
    }

    fn run_until(supervisor: &mut Supervisor, done: impl Fn(&Supervisor) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(supervisor) {
            assert!(Instant::now() < deadline, "graph did not settle");
            supervisor.tick(|_| Ok(())).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn hooks_gate_services_and_a_failed_hook_skips_them() {
        let temp = tempfile::TempDir::new().unwrap();
        let logs = crate::logs::directory(temp.path());
        let mut supervisor = Supervisor::new(
            vec![
                node("hook-a", "echo prepared", &[]),
                node("hook-b", "exit 3", &[("hook-a", Condition::Completed)]),
                node("web", "sleep 30", &[("hook-b", Condition::Completed)]),
            ],
            temp.path(),
        )
        .unwrap();
        run_until(&mut supervisor, |s| s.finished().is_some());
        assert_eq!(
            supervisor.failure(true).unwrap(),
            "hooks.hook-b failed with exit code 3; its output is above"
        );
        assert_eq!(supervisor.list()["services"][2]["status"], "skipped");
        assert_eq!(crate::logs::tail(&logs, "hook-a", 5).unwrap(), ["prepared"]);
    }

    #[test]
    fn a_healthy_dependency_starts_its_dependents_and_shutdown_reverses_the_order() {
        let temp = tempfile::TempDir::new().unwrap();
        let marker = temp.path().join("db-ready");
        let order = temp.path().join("order");
        let mut db = node(
            "db",
            &format!(
                "trap 'echo db >> {order}; exit 0' TERM; touch {marker}; while :; do sleep 0.05; done",
                order = order.display(),
                marker = marker.display()
            ),
            &[],
        );
        db.probe = Some(Probe::Exec(format!("test -f {}", marker.display())));
        let web = node(
            "web",
            &format!(
                "trap 'echo web >> {}; exit 0' TERM; while :; do sleep 0.05; done",
                order.display()
            ),
            &[("db", Condition::Healthy)],
        );
        let mut supervisor = Supervisor::new(vec![db, web], temp.path()).unwrap();
        run_until(&mut supervisor, |s| s.roots().len() == 2);
        supervisor.stop();
        run_until(&mut supervisor, |s| s.finished().is_some());
        assert_eq!(std::fs::read_to_string(&order).unwrap(), "web\ndb\n");
        assert_eq!(supervisor.finished(), Some(0));
    }

    #[test]
    fn startup_failures_count_only_until_the_run_is_ready() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut web = node("web", "exit 2", &[]);
        web.probe = Some(Probe::Http {
            port: 1,
            host: "web.localhost".into(),
            path: "/".into(),
        });
        let build = node("build", "true", &[]);
        let mut supervisor = Supervisor::new(vec![web, build], temp.path()).unwrap();
        run_until(&mut supervisor, |s| s.finished().is_some());
        assert_eq!(
            supervisor.failure(true).unwrap(),
            "web exited with code 2 before it was ready; its output is above"
        );
        assert_eq!(supervisor.failure(false), None);
        assert!(
            supervisor
                .children()
                .values()
                .all(|child| child.exit.is_some())
        );
    }

    #[test]
    fn a_hook_waiting_for_its_dependency_is_not_running() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut db = node("db", "sleep 30", &[]);
        db.probe = Some(Probe::Exec("false".into()));
        let mut supervisor = Supervisor::new(
            vec![db, node("hook-a", "true", &[("db", Condition::Healthy)])],
            temp.path(),
        )
        .unwrap();
        run_until(&mut supervisor, |s| s.roots().len() == 1);
        assert!(!supervisor.hooks_running());
        supervisor.stop();
        run_until(&mut supervisor, |s| s.finished().is_some());
    }

    #[test]
    fn a_publisher_restarts_after_a_failure_up_to_its_limit() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut publisher = node("parcela-lan-web", "exit 1", &[]);
        publisher.kind = Kind::Publisher;
        publisher.max_restarts = 2;
        let mut supervisor = Supervisor::new(vec![publisher], temp.path()).unwrap();
        run_until(&mut supervisor, |s| s.finished().is_some());
        assert_eq!(supervisor.children()["parcela-lan-web"].restarts, 2);
    }

    #[test]
    fn an_adopted_process_is_stopped_through_its_verified_group() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut first = Supervisor::new(vec![node("web", "sleep 30", &[])], temp.path()).unwrap();
        run_until(&mut first, |s| s.roots().len() == 1);
        let children = first.children();
        let identity = children["web"].identity.clone().unwrap();
        // The first owner "dies" without stopping its child.
        for slot in &mut first.slots {
            if let State::Running(Running::Own(child)) =
                std::mem::replace(&mut slot.state, State::Skipped)
            {
                child.detach();
            }
        }
        let mut second =
            Supervisor::adopt(vec![node("web", "sleep 30", &[])], temp.path(), &children);
        assert_eq!(second.roots(), [identity.pid]);
        second.stop();
        run_until(&mut second, |s| s.finished().is_some());
        assert!(!identity.alive());
    }

    #[test]
    fn a_stop_command_runs_after_its_dependents_stopped_and_before_the_signal() {
        let temp = tempfile::TempDir::new().unwrap();
        let order = temp.path().join("order");
        // Each shell marks itself ready once its trap is set, so the signal cannot arrive first.
        let script = |name: &str| {
            format!(
                "trap 'echo {name} >> {}; exit 0' TERM; touch {}; while :; do sleep 0.05; done",
                order.display(),
                temp.path().join(format!("{name}.ready")).display()
            )
        };
        let mut db = node("db", &script("db"), &[]);
        db.stop = Some(format!("echo stop >> {}", order.display()));
        let web = node("web", &script("web"), &[("db", Condition::Started)]);
        let mut supervisor = Supervisor::new(vec![db, web], temp.path()).unwrap();
        let ready = |name: &str| temp.path().join(format!("{name}.ready")).exists();
        run_until(&mut supervisor, |_| ready("db") && ready("web"));
        supervisor.stop();
        run_until(&mut supervisor, |s| s.finished().is_some());
        assert_eq!(std::fs::read_to_string(&order).unwrap(), "web\nstop\ndb\n");
        assert!(supervisor.children()["db"].stopped);
        assert_eq!(supervisor.stop_failure(), None);
    }

    #[test]
    fn a_service_that_already_exited_still_runs_its_stop_command() {
        let temp = tempfile::TempDir::new().unwrap();
        let marker = temp.path().join("stopped");
        let mut deps = node("deps", "exit 0", &[]);
        deps.stop = Some(format!("touch {}", marker.display()));
        let mut supervisor = Supervisor::new(vec![deps], temp.path()).unwrap();
        run_until(&mut supervisor, |s| s.finished().is_some());
        assert!(!marker.exists());
        supervisor.stop();
        run_until(&mut supervisor, |s| s.finished().is_some());
        assert!(marker.exists());
    }

    #[test]
    fn a_failed_stop_command_is_reported_and_runs_again_for_a_successor() {
        let temp = tempfile::TempDir::new().unwrap();
        let attempts = temp.path().join("attempts");
        let deps = || {
            let mut deps = node("deps", "exit 0", &[]);
            deps.stop = Some(format!(
                "echo try >> {0}; test $(wc -l < {0}) -ge 2",
                attempts.display()
            ));
            deps
        };
        let mut first = Supervisor::new(vec![deps()], temp.path()).unwrap();
        run_until(&mut first, |s| s.finished().is_some());
        first.stop();
        run_until(&mut first, |s| s.finished().is_some());
        assert_eq!(
            first.stop_failure().unwrap(),
            "services.deps.stop failed with exit code 1; see parcela logs deps"
        );
        let children = first.children();
        assert!(!children["deps"].stopped);
        let mut second = Supervisor::adopt(vec![deps()], temp.path(), &children);
        second.stop();
        run_until(&mut second, |s| s.finished().is_some());
        assert_eq!(second.stop_failure(), None);
        let children = second.children();
        assert!(children["deps"].stopped);
        let mut third = Supervisor::adopt(vec![deps()], temp.path(), &children);
        third.stop();
        run_until(&mut third, |s| s.finished().is_some());
        assert_eq!(
            std::fs::read_to_string(&attempts).unwrap().lines().count(),
            2
        );
    }
}

use crate::{
    config::Manifest,
    private, process,
    registry::{Session, SessionStatus},
    supervisor::Child,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Starting,
    Running,
    Stopping,
    Stopped,
    Interrupted,
}
impl Phase {
    /// The registry status a run in this phase reports.
    pub fn status(self) -> SessionStatus {
        match self {
            Self::Starting => SessionStatus::Starting,
            Self::Running => SessionStatus::Ready,
            Self::Stopping => SessionStatus::Stopping,
            Self::Stopped => SessionStatus::Stopped,
            Self::Interrupted => SessionStatus::Interrupted,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub session: Session,
    pub phase: Phase,
    pub owner: process::Identity,
    pub lock: private::Identity,
    pub control: private::Identity,
    /// Every process the graph started, by name.
    pub children: BTreeMap<String, Child>,
    /// The children and every process descending from them, as last seen.
    pub processes: Vec<process::Identity>,
    pub manifest: Manifest,
    pub diagnostic: Option<String>,
    pub share: crate::share::Share,
}
impl Record {
    pub fn read(path: &Path, expected: &Session) -> Result<Self> {
        let record: Self = private::read_json(&path.join("owner.json"))?;
        ensure!(
            record.session.environment_id == expected.environment_id
                && record.session.generation == expected.generation
                && record.session.token == expected.token
                && record.session.backend_port == expected.backend_port
                && record.session.public_url == expected.public_url
                && record.session.endpoints == expected.endpoints,
            "runtime evidence belongs to a different generation"
        );
        record.manifest.validate()?;
        let hosts = record.manifest.service_hosts(
            record
                .session
                .public_url
                .as_deref()
                .unwrap_or_default()
                .trim_start_matches("https://")
                .split(':')
                .next()
                .unwrap_or_default(),
        )?;
        let endpoints = record
            .session
            .endpoints(record.manifest.default_service.as_deref());
        ensure!(
            endpoints.len() == hosts.len()
                && endpoints
                    .iter()
                    .all(|endpoint| hosts.get(&endpoint.service) == Some(&endpoint.hostname)),
            "recorded service set differs from launch manifest"
        );
        Ok(record)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        private::write_json(&path.join("owner.json"), self)
    }
    /// Forgets processes that exited and adds every descendant of a live child.
    pub fn refresh(&mut self, path: &Path) -> Result<()> {
        let previous = self.processes.clone();
        self.processes.retain(process::Identity::alive);
        let roots: Vec<process::Identity> = self
            .children
            .values()
            .filter(|child| child.exit.is_none())
            .filter_map(|child| child.identity.clone())
            .collect();
        for identity in process::tree(&roots)? {
            if !self.processes.contains(&identity) {
                self.processes.push(identity);
            }
        }
        if self.processes != previous {
            self.save(path)?;
        }
        Ok(())
    }
    /// The live children, by pid: a listener in their process trees belongs to this run.
    pub fn roots(&self) -> Vec<u32> {
        self.children
            .values()
            .filter(|child| child.exit.is_none())
            .filter_map(|child| child.identity.as_ref())
            .filter(|identity| identity.alive())
            .map(|identity| identity.pid)
            .collect()
    }
    /// A running generation whose live owner still holds the ownership lock.
    pub fn owner_holds(&self, path: &Path) -> bool {
        self.phase == Phase::Running
            && self.owner.alive()
            && self.lock.locked(&path.join("owner.lock")).unwrap_or(false)
    }
    /// `owner_holds`, and every endpoint listens inside the recorded process trees.
    pub fn routable(&self, path: &Path) -> bool {
        let roots = self.roots();
        self.owner_holds(path)
            && self
                .session
                .endpoints(self.manifest.default_service.as_deref())
                .iter()
                .all(|endpoint| {
                    process::listener_identity(endpoint.backend_port, &roots)
                        .is_ok_and(|pids| !pids.is_empty())
                })
    }
    /// The owner is gone and the held lock matches. A changed device number is allowed only
    /// after every recorded process's boot ended, with the lock inode still unchanged.
    pub fn abandoned(&self, lock: &private::Lock) -> Result<bool> {
        let actual = lock.identity()?;
        if self.owner.alive() {
            return Ok(false);
        }
        if actual == self.lock {
            return Ok(true);
        }
        if !self.lock.same_inode(&actual) || !self.owner.previous_boot()? {
            return Ok(false);
        }
        for identity in self
            .children
            .values()
            .filter_map(|child| child.identity.as_ref())
            .chain(self.processes.iter())
        {
            if !identity.previous_boot()? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// No recorded child or descendant is alive.
    pub fn quiescent(&self) -> bool {
        self.children.values().all(|child| {
            !child
                .identity
                .as_ref()
                .is_some_and(process::Identity::alive)
        }) && self.processes.iter().all(|process| !process.alive())
    }
}

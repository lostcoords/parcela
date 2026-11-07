//! Anonymous usage telemetry: which commands run and how they end, never what they act on.
//! docs/site/telemetry.md lists every field and the ways to turn it off.
use super::args::{Args, TelemetryCommand};
use super::output::Output;
use anyhow::Result;
use clap::{ArgMatches, CommandFactory, parser::ValueSource};
use parcela::host::Host;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

const ENDPOINT: &str = "https://parcela.run/api/telemetry";
const NOTICE: &str = "Parcela collects anonymous usage data: commands, exit codes and versions, never paths or names.\nTurn it off with `parcela telemetry disable` or DO_NOT_TRACK=1. https://parcela.run/docs/telemetry/";

/// `<state>/telemetry.json`: the install's random ID, whether the notice was shown, and the
/// user's choice.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Settings {
    #[serde(default)]
    install_id: Option<Uuid>,
    #[serde(default)]
    noticed: bool,
    #[serde(default)]
    enabled: Option<bool>,
}

impl Settings {
    fn path(state: &Path) -> PathBuf {
        state.join("telemetry.json")
    }

    pub(super) fn load(state: &Path) -> Self {
        parcela::private::read_json(&Self::path(state)).unwrap_or_default()
    }

    fn save(&self, state: &Path) -> Result<()> {
        parcela::private::write_json(&Self::path(state), self)
    }

    /// Saves the choice in the host config, which supersedes the one older releases kept here.
    pub(super) fn set_enabled(state: &Path, enabled: bool) -> Result<()> {
        let path = parcela::host::path()?;
        let mut host = Host::load(&path)?;
        host.telemetry = Some(enabled);
        host.save(&path)?;
        let mut settings = Self::load(state);
        settings.enabled = None;
        settings.noticed = true;
        settings.save(state)
    }
}

/// `parcela telemetry [status|enable|disable]`.
pub(super) fn run(state: &Path, command: TelemetryCommand, out: Output) -> Result<i32> {
    match command {
        TelemetryCommand::Status => {}
        TelemetryCommand::Enable => Settings::set_enabled(state, true)?,
        TelemetryCommand::Disable => Settings::set_enabled(state, false)?,
    }
    let reason = disabled_by(state);
    out.emit(
        &json!({"enabled": reason.is_none(), "disabled_by": reason}),
        |style| match reason {
            None => format!("{} Telemetry is on", style.ok()),
            Some(reason) => format!("Telemetry is off ({reason})"),
        },
    )?;
    Ok(0)
}

/// Why telemetry is off, or None when it is on.
pub(super) fn disabled_by(state: &Path) -> Option<&'static str> {
    if truthy("DO_NOT_TRACK") {
        Some("DO_NOT_TRACK")
    } else if truthy("PARCELA_TELEMETRY_DISABLED") {
        Some("PARCELA_TELEMETRY_DISABLED")
    } else {
        // An unreadable host config sends nothing rather than guess.
        match Host::current().map(|host| host.telemetry) {
            Err(_) => Some("invalid host config"),
            Ok(Some(false)) => Some("parcela telemetry disable"),
            Ok(Some(true)) => None,
            Ok(None) if Settings::load(state).enabled == Some(false) => {
                Some("parcela telemetry disable")
            }
            Ok(None) => None,
        }
    }
}

fn truthy(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| !matches!(value.trim(), "" | "0" | "false" | "no"))
}

/// One CLI invocation, timed from parse to exit.
pub(super) struct Session {
    started: Instant,
    state: Option<PathBuf>,
}

pub(super) fn begin(state_dir: Option<PathBuf>) -> Session {
    Session {
        started: Instant::now(),
        state: super::commands::state_dir(state_dir).ok(),
    }
}

impl Session {
    /// Report the invocation after it ran. Never fails and never waits on the network.
    pub(super) fn finish(self, exit: i32) {
        let Some(state) = self.state else { return };
        let Some(invocation) = Invocation::current() else {
            return;
        };
        if invocation.internal || invocation.command == "telemetry" || disabled_by(&state).is_some()
        {
            return;
        }
        let mut event = event(&invocation, exit, self.started.elapsed());
        if truthy("PARCELA_TELEMETRY_DEBUG") {
            event["install_id"] = json!(Settings::load(&state).install_id);
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&event).unwrap_or_default()
            );
            return;
        }
        // Development builds, including `cargo test`, print, write and send nothing.
        if cfg!(debug_assertions) {
            return;
        }
        match next_step(&state) {
            Step::Notice => eprintln!("{NOTICE}"),
            Step::Send(id) => {
                event["install_id"] = json!(id);
                send(&event);
            }
            Step::Nothing => {}
        }
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Notice,
    Send(Uuid),
    Nothing,
}

/// Nothing is sent until the notice has been shown once; after that, the install's ID.
fn next_step(state: &Path) -> Step {
    let mut settings = Settings::load(state);
    let step = if settings.noticed {
        Step::Send(*settings.install_id.get_or_insert_with(Uuid::new_v4))
    } else {
        settings.noticed = true;
        Step::Notice
    };
    // An unsaved notice or ID would repeat on every run; stay quiet instead.
    if settings.save(state).is_err() {
        return Step::Nothing;
    }
    step
}

/// The subcommand path and the flags given on the command line, by name only.
pub(super) struct Invocation {
    pub(super) command: String,
    pub(super) flags: Vec<String>,
    /// A hidden command or flag: Parcela re-running itself, or a test hook.
    pub(super) internal: bool,
    pub(super) json: bool,
}

impl Invocation {
    fn current() -> Option<Self> {
        Self::from_args(std::env::args_os())
    }

    pub(super) fn from_args<I, T>(args: I) -> Option<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let mut command = Args::command();
        command.build();
        let matches = command.clone().try_get_matches_from(args).ok()?;
        let (mut current, mut found): (&clap::Command, &ArgMatches) = (&command, &matches);
        let mut names = Vec::new();
        let mut flags = BTreeSet::new();
        let mut internal = false;
        loop {
            for arg in current.get_arguments() {
                let id = arg.get_id().as_str();
                if found.try_contains_id(id).is_err()
                    || found.value_source(id) != Some(ValueSource::CommandLine)
                {
                    continue;
                }
                internal |= arg.is_hide_set();
                if let Some(long) = arg.get_long() {
                    flags.insert(format!("--{long}"));
                }
            }
            let Some((name, sub)) = found.subcommand() else {
                break;
            };
            let Some(next) = current.find_subcommand(name) else {
                break;
            };
            internal |= next.is_hide_set();
            names.push(next.get_name().to_owned());
            (current, found) = (next, sub);
        }
        let json = flags.contains("--json");
        Some(Self {
            command: names.join(" "),
            flags: flags.into_iter().collect(),
            internal,
            json,
        })
    }
}

fn event(invocation: &Invocation, exit: i32, elapsed: Duration) -> Value {
    let version = env!("PARCELA_VERSION");
    json!({
        "schema": 1,
        "event": "command",
        "command": invocation.command,
        "flags": invocation.flags,
        "exit": exit,
        "duration": bucket(elapsed),
        "version": version,
        "channel": if version.contains('+') { "nightly" } else { "stable" },
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "install": super::upgrade::install_method(),
        "ci": ci(),
        "terminal": std::io::IsTerminal::is_terminal(&std::io::stdout()),
        "json": invocation.json,
    })
}

pub(super) fn bucket(elapsed: Duration) -> &'static str {
    match elapsed.as_secs() {
        0 => "<1s",
        1..5 => "1-5s",
        5..30 => "5-30s",
        30..120 => "30s-2m",
        _ => ">2m",
    }
}

/// Whether a CI provider set its usual variable; only the yes or no is reported.
fn ci() -> bool {
    truthy("CI")
        || [
            "GITHUB_ACTIONS",
            "GITLAB_CI",
            "BUILDKITE",
            "CIRCLECI",
            "TF_BUILD",
            "JENKINS_URL",
            "TEAMCITY_VERSION",
            "TRAVIS",
        ]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

/// Hand the event to a detached curl that gives up after 3 seconds; errors are ignored.
fn send(event: &Value) {
    let Ok(body) = serde_json::to_vec(event) else {
        return;
    };
    let mut command = Command::new("curl");
    command
        .args(["--proto", "=https", "--tlsv1.2", "-fsS", "--max-time", "3"])
        .args([
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
            "-o",
            "/dev/null",
        ])
        .arg(ENDPOINT)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches no memory; a new session keeps the
    // terminal's signals away from curl once Parcela exits.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    if let Ok(mut child) = command.spawn()
        && let Some(mut stdin) = child.stdin.take()
    {
        let _ = stdin.write_all(&body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation(args: &[&str]) -> Invocation {
        Invocation::from_args(std::iter::once("parcela").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn flags_are_names_never_values() {
        let up = invocation(&[
            "up",
            "--name",
            "secret-client",
            "--detach",
            "--path",
            "/Users/me/acme",
        ]);
        assert_eq!(up.command, "up");
        assert_eq!(up.flags, ["--detach", "--name", "--path"]);
        let task = invocation(&["task", "deploy", "--", "--token", "hunter2"]);
        assert_eq!(task.command, "task");
        assert!(task.flags.is_empty());
        let run = invocation(&["run", "--", "rails", "server", "--port", "3000"]);
        assert!(run.flags.is_empty());
        let text = serde_json::to_string(&event(&up, 0, Duration::ZERO)).unwrap();
        for leak in ["secret-client", "/Users/me", "acme", "hunter2", "rails"] {
            assert!(!text.contains(leak), "{leak} leaked into {text}");
        }
    }

    #[test]
    fn nested_and_global_flags() {
        let install = invocation(&["--json", "proxy", "install"]);
        assert_eq!(install.command, "proxy install");
        assert_eq!(install.flags, ["--json"]);
        assert!(install.json);
        assert_eq!(invocation(&[]).command, "");
    }

    #[test]
    fn hidden_commands_and_flags_are_internal() {
        assert!(invocation(&["proxy", "run"]).internal);
        assert!(invocation(&["recover"]).internal);
        assert!(invocation(&["up", "--launch-file", "/tmp/launch.json"]).internal);
        assert!(!invocation(&["up", "--detach"]).internal);
    }

    #[test]
    fn notice_first_then_one_install_id() {
        let state = tempfile::TempDir::new().unwrap();
        let state = state.path().join("state");
        assert_eq!(next_step(&state), Step::Notice);
        let Step::Send(id) = next_step(&state) else {
            panic!("second run sends")
        };
        assert_eq!(next_step(&state), Step::Send(id));
    }

    #[test]
    fn durations_are_buckets() {
        assert_eq!(bucket(Duration::from_millis(300)), "<1s");
        assert_eq!(bucket(Duration::from_secs(4)), "1-5s");
        assert_eq!(bucket(Duration::from_secs(29)), "5-30s");
        assert_eq!(bucket(Duration::from_secs(90)), "30s-2m");
        assert_eq!(bucket(Duration::from_secs(600)), ">2m");
    }
}

//! `parcela upgrade` reruns install.sh for an install.sh install and names the right command for
//! any other. Interactive use checks stable releases at most once a day.
use super::args::{Args, Command as CliCommand};
use anyhow::{Context, Result, bail, ensure};
use parcela::{private, proxy, router::VERSION};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::{
    io::{IsTerminal, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const INSTALLER: &str = "https://parcela.run/install.sh";

#[derive(Debug, PartialEq)]
enum Install {
    Script,
    Homebrew,
    Cargo,
    Unknown,
}

/// How the binary at `exe` was installed, from where it lives once symlinks are resolved.
fn classify(exe: &Path, home: &Path, cargo_home: &Path) -> Install {
    let real = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let exe = real(exe);
    if exe.components().any(|part| part.as_os_str() == "Cellar") {
        Install::Homebrew
    } else if exe.starts_with(real(&cargo_home.join("bin"))) {
        Install::Cargo
    } else if exe == real(&home.join(".local/bin/parcela")) {
        Install::Script
    } else {
        Install::Unknown
    }
}

/// How this binary was installed, for telemetry.
pub(super) fn install_method() -> &'static str {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return "unknown";
    };
    let cargo_home =
        std::env::var_os("CARGO_HOME").map_or_else(|| home.join(".cargo"), PathBuf::from);
    let Ok(exe) = std::env::current_exe() else {
        return "unknown";
    };
    match classify(&exe, &home, &cargo_home) {
        Install::Script => "install.sh",
        Install::Homebrew => "homebrew",
        Install::Cargo => "cargo",
        Install::Unknown => "unknown",
    }
}

pub(super) fn run(nightly: bool, state: &Path) -> Result<i32> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME missing")?);
    let cargo_home =
        std::env::var_os("CARGO_HOME").map_or_else(|| home.join(".cargo"), PathBuf::from);
    let exe = std::env::current_exe().context("cannot find the running parcela")?;
    match classify(&exe, &home, &cargo_home) {
        Install::Script => {}
        Install::Homebrew => bail!("installed with Homebrew; run `brew upgrade parcela`"),
        Install::Cargo => bail!(
            "installed with cargo; run `cargo install --git https://github.com/lostcoords/parcela --locked --force`"
        ),
        Install::Unknown => bail!(
            "{} was not installed by install.sh; upgrade it the way you installed it",
            exe.display()
        ),
    }
    let dir = tempfile::tempdir()?;
    let script = dir.path().join("install.sh");
    let fetched = Command::new("curl")
        .args(["--proto", "=https", "--tlsv1.2", "-fsSL", "-o"])
        .arg(&script)
        .arg(INSTALLER)
        .status()
        .context("curl not found")?;
    ensure!(fetched.success(), "could not download {INSTALLER}");
    let mut install = Command::new("sh");
    install.arg(&script).env("PARCELA_STATE_DIR", state);
    if nightly {
        install.arg("--nightly");
    }
    Ok(install
        .status()
        .context("sh not found")?
        .code()
        .unwrap_or(1))
}

const LATEST: &str = "https://api.github.com/repos/lostcoords/parcela/releases/latest";
const DAY: u64 = 24 * 60 * 60;

#[derive(Default, Deserialize, Serialize)]
struct UpdateCheck {
    checked_at: u64,
    latest: Option<String>,
    prompted_at: u64,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn latest_release() -> Result<String> {
    let mut command = Command::new("curl");
    command.args([
        "--proto",
        "=https",
        "--tlsv1.2",
        "-fsSL",
        "--connect-timeout",
        "1",
        "--max-time",
        "2",
        "--max-filesize",
        "65536",
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "User-Agent: parcela",
        LATEST,
    ]);
    let output = parcela::process::capture(&mut command, Duration::from_secs(3), 65536)?;
    ensure!(output.status.success(), "release check unavailable");
    let release: Release = serde_json::from_slice(&output.stdout)?;
    let version = semver::Version::parse(release.tag_name.trim_start_matches('v'))?;
    ensure!(
        !release.draft && !release.prerelease && version.pre.is_empty(),
        "not a stable release"
    );
    Ok(version.to_string())
}

fn offer_update(state: &Path) -> Result<Option<semver::Version>> {
    if std::env::var_os("PARCELA_NO_UPDATE").is_some_and(|value| value == "1")
        || !std::io::stdin().is_terminal()
        || !std::io::stderr().is_terminal()
        || !std::io::stdout().is_terminal()
        || install_method() == "unknown"
    {
        return Ok(None);
    }
    private::directory(state)?;
    let Some(_lock) = private::Lock::try_acquire(&state.join("update-check.lock"))? else {
        return Ok(None);
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let file = state.join("update-check.json");
    let mut check: UpdateCheck = private::read_json(&file).unwrap_or_default();
    if now.saturating_sub(check.checked_at) >= DAY {
        check.checked_at = now;
        check.latest = latest_release().ok();
        private::write_json(&file, &check)?;
    }
    if now.saturating_sub(check.prompted_at) < DAY {
        return Ok(None);
    }
    let Some(latest) = check
        .latest
        .as_ref()
        .and_then(|value| semver::Version::parse(value).ok())
    else {
        return Ok(None);
    };
    if !latest
        .cmp_precedence(&semver::Version::parse(VERSION)?)
        .is_gt()
    {
        return Ok(None);
    }
    check.prompted_at = now;
    private::write_json(&file, &check)?;
    if install_method() != "install.sh" {
        eprintln!(
            "parcela: {latest} is available (current {VERSION}); run `parcela upgrade` for your installation's update command"
        );
        return Ok(None);
    }
    eprint!("parcela: {latest} is available (current {VERSION}). Update now? [Y/n] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer)? == 0 {
        return Ok(None);
    }
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    )
    .then_some(latest))
}

/// Only foreground commands perform maintenance. Detached owners keep their original binary
/// and must never replace a newer proxy after the CLI on disk has changed.
pub(super) fn prepare(args: &Args) -> Result<()> {
    let foreground = matches!(
        args.command,
        None | Some(
            CliCommand::Status { .. }
                | CliCommand::Ls
                | CliCommand::Ui
                | CliCommand::Open { .. }
                | CliCommand::Url { .. }
                | CliCommand::Logs { .. }
                | CliCommand::Run { .. }
                | CliCommand::Up {
                    launch_file: None,
                    ..
                }
                | CliCommand::Restart {
                    launch_file: None,
                    ..
                }
        )
    );
    if !foreground {
        return Ok(());
    }
    let state = super::commands::state_dir(args.global.state_dir.clone())?;
    // An offline update check never prevents the requested command.
    let offered = if args.global.json {
        Ok(None)
    } else {
        offer_update(&state)
    };
    match offered {
        Ok(Some(latest)) => {
            let executable = std::env::current_exe()?;
            let code = run(false, &state)?;
            ensure!(
                code == 0,
                "update did not complete (exit {code}); rerun your command to continue with the installed version"
            );
            let version = Command::new(&executable).arg("--version").output()?;
            ensure!(
                version.status.success(),
                "updated executable could not report its version"
            );
            let version = String::from_utf8(version.stdout)?;
            let version = semver::Version::parse(
                version
                    .trim()
                    .strip_prefix("parcela ")
                    .context("updated executable did not report its version")?,
            )?;
            ensure!(
                !version.cmp_precedence(&latest).is_lt(),
                "installer returned {version}, expected at least {latest}; run `parcela upgrade` to retry"
            );
            let error = Command::new(executable)
                .args(std::env::args_os().skip(1))
                .env("PARCELA_NO_UPDATE", "1")
                .exec();
            return Err(error.into());
        }
        Ok(None) => (),
        Err(error) => eprintln!("parcela: update check skipped: {error:#}"),
    }
    if let Err(error) = proxy::refresh(&state) {
        eprintln!("parcela: automatic proxy refresh failed: {error:#}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_location() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let cargo = home.join(".cargo");
        let file = |path: &Path| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
            path.to_owned()
        };
        let script = file(&home.join(".local/bin/parcela"));
        let cellar = file(&root.path().join("brew/Cellar/parcela/0.4.0/bin/parcela"));
        let cargo_bin = file(&cargo.join("bin/parcela"));
        let other = file(&root.path().join("usr/bin/parcela"));
        assert_eq!(classify(&script, &home, &cargo), Install::Script);
        assert_eq!(classify(&cellar, &home, &cargo), Install::Homebrew);
        assert_eq!(classify(&cargo_bin, &home, &cargo), Install::Cargo);
        assert_eq!(classify(&other, &home, &cargo), Install::Unknown);
        // A Homebrew link in ~/.local/bin still belongs to Homebrew.
        std::fs::remove_file(&script).unwrap();
        std::os::unix::fs::symlink(&cellar, &script).unwrap();
        assert_eq!(classify(&script, &home, &cargo), Install::Homebrew);
    }
}

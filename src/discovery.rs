//! Read-only VCS inventory. Discovery never registers, creates, snapshots, or removes a checkout.
use crate::process;
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// One checkout of the repository: its root and, for a jj workspace, its name.
#[derive(Debug)]
pub struct Checkout {
    pub source: PathBuf,
    pub name: Option<String>,
}

/// Every jj workspace or Git worktree of the repository holding `source`, or `source` alone
/// when it is not under version control.
pub fn discover(source: &Path) -> Result<Vec<Checkout>> {
    let source = source
        .canonicalize()
        .context("discovery requires an existing source directory")?;
    ensure!(source.is_dir(), "discovery requires a directory");
    let Some(root) = source
        .ancestors()
        .find(|path| path.join(".jj").is_dir() || path.join(".git").exists())
    else {
        return Ok(vec![Checkout { source, name: None }]);
    };
    let jj = root.join(".jj").is_dir();
    let mut command = Command::new(if jj { "jj" } else { "git" });
    if jj {
        command.arg("--repository").arg(root).args([
            "--ignore-working-copy",
            "--at-op=@",
            "--color=never",
            "--no-pager",
            "workspace",
            "list",
            "--template",
            "name ++ \"\\0\" ++ root ++ \"\\0\"",
        ]);
    } else {
        command
            .arg("-C")
            .arg(root)
            .args(["worktree", "list", "--porcelain", "-z"]);
    }
    let output = process::capture(&mut command, Duration::from_secs(3), 512000)?;
    ensure!(
        output.status.success(),
        "{} discovery unavailable (jj needs workspace root templates, tested with 0.40): {}",
        if jj { "jj" } else { "Git" },
        String::from_utf8_lossy(&output.stderr)
    );
    if jj {
        parse_jj(&output.stdout)
    } else {
        parse_git(&output.stdout)
    }
}
fn parse_jj(bytes: &[u8]) -> Result<Vec<Checkout>> {
    let text = std::str::from_utf8(bytes)?;
    let fields = text.split_terminator('\0').collect::<Vec<_>>();
    let (rows, remainder) = fields.as_chunks::<2>();
    ensure!(remainder.is_empty(), "invalid jj workspace inventory");
    let mut checkouts = Vec::new();
    for row in rows {
        let path = PathBuf::from(row[1]);
        if !path.is_absolute() {
            eprintln!(
                "parcela: jj workspace {:?} has no readable root; omitted from checkout inventory",
                row[0]
            );
            continue;
        }
        checkouts.push(Checkout {
            source: path,
            name: Some(row[0].into()),
        });
    }
    Ok(checkouts)
}
fn parse_git(bytes: &[u8]) -> Result<Vec<Checkout>> {
    let mut rows = Vec::new();
    let mut current: Option<Checkout> = None;
    for field in std::str::from_utf8(bytes)?.split('\0') {
        if let Some(path) = field.strip_prefix("worktree ") {
            ensure!(current.is_none(), "invalid Git worktree record");
            let path = PathBuf::from(path);
            ensure!(
                path.is_absolute(),
                "Git returned a nonabsolute worktree root"
            );
            current = Some(Checkout {
                source: path,
                name: None,
            });
        } else if field.is_empty()
            && let Some(row) = current.take()
        {
            rows.push(row);
        }
    }
    ensure!(current.is_none(), "incomplete Git worktree inventory");
    Ok(rows)
}
#[cfg(test)]
mod tests {
    #[test]
    fn nul_inventory_preserves_paths_and_detached_state() {
        let rows = super::parse_git(b"worktree /tmp/with\nnewline\0HEAD abc\0branch refs/heads/main\0\0worktree /tmp/detached\0HEAD def\0detached\0\0").unwrap();
        assert_eq!(rows[0].source.to_str().unwrap(), "/tmp/with\nnewline");
        assert_eq!(rows[1].source.to_str().unwrap(), "/tmp/detached");
        assert!(super::parse_jj(b"odd\0fields\0count\0").is_err());
    }
}

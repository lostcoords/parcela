//! Prints what an owner's processes write, while it runs.
use std::os::fd::AsFd;
use std::{thread, time::Duration};

/// Prints what every process writes, as `name | line`, while the owner runs: to the terminal in
/// the foreground, and to `owner.log` when detached, there only until the run is first ready, so
/// the file stays small.
pub(super) struct Echo {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: thread::JoinHandle<()>,
}
impl Echo {
    /// `takeover` is for an owner that takes over a run: it skips what the processes wrote
    /// before and prints to stderr, so the command's own output stays one document.
    pub(super) fn start<'a>(
        runtime: &std::path::Path,
        names: impl Iterator<Item = &'a str>,
        prefixed: bool,
        takeover: bool,
        ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        use std::io::Write;
        use std::sync::atomic::Ordering;
        let mut names: Vec<String> = names.map(str::to_owned).collect();
        names.sort();
        let width = names.iter().map(String::len).max().unwrap_or(0);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let detached = stdout_is(&runtime.join("log/owner.log"));
        let mut follower = crate::environment::logs::Follower::new(
            &crate::environment::logs::directory(runtime),
            takeover.then_some(0),
            Some(LOG_LIMIT),
        );
        let thread = thread::spawn(move || {
            let paint = if takeover {
                crate::paint::Paint::stderr()
            } else {
                crate::paint::Paint::stdout()
            };
            loop {
                let last = stopped.load(Ordering::Relaxed);
                let quiet = detached && ready.load(Ordering::Relaxed);
                let mut out: Box<dyn Write> = if takeover {
                    Box::new(std::io::stderr().lock())
                } else {
                    Box::new(std::io::stdout().lock())
                };
                // Still read when quiet, so the logs keep rotating.
                let _ = follower.poll(|name, line| {
                    if quiet {
                        return;
                    }
                    let _ = if prefixed {
                        let index = names.iter().position(|known| known == name).unwrap_or(0);
                        writeln!(out, "{} {line}", paint.log_prefix(index, name, width))
                    } else {
                        writeln!(out, "{line}")
                    };
                });
                let _ = out.flush();
                drop(out);
                if last {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        Self { stop, thread }
    }
    /// Prints what is left, then stops.
    pub(super) fn finish(self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.thread.join();
    }
}

/// Whether stdout is `path`, as for the owner `up --detach` starts with its output in owner.log.
fn stdout_is(path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(file), Ok(out)) = (
        std::fs::metadata(path),
        std::io::stdout().as_fd().try_clone_to_owned(),
    ) else {
        return false;
    };
    std::fs::File::from(out)
        .metadata()
        .is_ok_and(|out| out.dev() == file.dev() && out.ino() == file.ino())
}

/// A process log is copied to `<name>.log.1` and emptied past this size.
const LOG_LIMIT: u64 = 16 * 1024 * 1024;

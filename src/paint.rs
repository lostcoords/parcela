//! Terminal colors, shown when the stream is a terminal or `CLICOLOR_FORCE`/`FORCE_COLOR` asks
//! for them, and never when `NO_COLOR` is set.
use std::io::{IsTerminal, Write};
use std::time::Instant;

#[derive(Clone, Copy)]
pub struct Paint {
    color: bool,
}

/// `CLICOLOR_FORCE` or `FORCE_COLOR` set to anything but empty or `0`.
fn forced() -> bool {
    ["CLICOLOR_FORCE", "FORCE_COLOR"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty() && value != "0"))
}

impl Paint {
    pub fn stdout() -> Self {
        Self::new(std::io::stdout().is_terminal())
    }
    pub fn stderr() -> Self {
        Self::new(std::io::stderr().is_terminal())
    }
    fn new(terminal: bool) -> Self {
        Self {
            color: std::env::var_os("NO_COLOR").is_none() && (terminal || forced()),
        }
    }
    pub fn paint(self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }
    /// A message with each `command` in cyan, so the fix in a hint stands out.
    pub fn commands(self, text: &str) -> String {
        if !self.color {
            return text.to_owned();
        }
        let mut out = String::new();
        let mut parts = text.split('`');
        out.push_str(parts.next().unwrap_or_default());
        let rest: Vec<&str> = parts.collect();
        // An odd backtick count leaves the last one literal.
        let pairs = rest.len() / 2 * 2;
        for (index, part) in rest.iter().enumerate() {
            if index < pairs && index % 2 == 0 {
                out.push_str(&self.paint("36", &format!("`{part}`")));
            } else if index < pairs {
                out.push_str(part);
            } else {
                out.push('`');
                out.push_str(part);
            }
        }
        out
    }
    /// `parcela: message` on stderr, with commands highlighted.
    pub fn note(text: &str) {
        let paint = Self::stderr();
        eprintln!("{} {}", paint.paint("2", "parcela:"), paint.commands(text));
    }
    /// `name | ` before a log line. `index` is the process's place in the list, so neighbours
    /// get different colors.
    pub fn log_prefix(self, index: usize, name: &str, width: usize) -> String {
        const COLORS: [&str; 6] = ["36", "33", "32", "35", "34", "31"];
        self.paint(COLORS[index % COLORS.len()], &format!("{name:width$} |"))
    }
}

/// One updating status line on stderr while something runs, only on a terminal. It clears
/// itself when dropped, so later output starts on a clean line.
pub struct Spinner {
    started: Instant,
    frame: usize,
    shown: bool,
    terminal: bool,
}

impl Spinner {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            frame: 0,
            shown: false,
            terminal: std::io::stderr().is_terminal() && std::env::var_os("CI").is_none(),
        }
    }

    pub fn update(&mut self, text: &str) {
        if !self.terminal {
            return;
        }
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let paint = Paint::stderr();
        let frame = paint.paint("36", FRAMES[self.frame % FRAMES.len()]);
        self.frame += 1;
        let elapsed = paint.paint("2", &format!("{}s", self.started.elapsed().as_secs()));
        let width = terminal_width().saturating_sub(12);
        let text: String = text.chars().take(width).collect();
        let mut err = std::io::stderr().lock();
        let _ = write!(err, "\r\x1b[2K{frame} {text} {elapsed}");
        let _ = err.flush();
        self.shown = true;
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        if self.shown {
            let mut err = std::io::stderr().lock();
            let _ = write!(err, "\r\x1b[2K");
            let _ = err.flush();
        }
    }
}

fn terminal_width() -> usize {
    // SAFETY: TIOCGWINSZ fills a winsize struct we own; failure leaves the default.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDERR_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    if ok && size.ws_col > 0 {
        usize::from(size.ws_col)
    } else {
        80
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_highlighted_only_in_pairs() {
        let paint = Paint { color: true };
        assert_eq!(
            paint.commands("run `parcela trust` now"),
            "run \x1b[36m`parcela trust`\x1b[0m now"
        );
        assert_eq!(paint.commands("odd ` tick"), "odd ` tick");
        assert_eq!(Paint { color: false }.commands("a `b` c"), "a `b` c");
    }
}

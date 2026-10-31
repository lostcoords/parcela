//! Each process of a run writes its output to `<runtime>/log/processes/<name>.log`. These read
//! them back: a tail for `parcela logs`, and a follower for `logs --follow` and the owner's echo.
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// Where a run's processes write, under the environment's runtime directory.
pub fn directory(runtime: &Path) -> PathBuf {
    runtime.join("log/processes")
}

/// The file `name` writes to, opened for appending: a private regular file, never a link or FIFO.
pub fn open(directory: &Path, name: &str) -> Result<File> {
    ensure!(valid(name), "invalid process name {name:?}");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(format!("{name}.log")))
        .with_context(|| format!("open the log of {name}"))?;
    let metadata = file.metadata()?;
    // SAFETY: getuid has no pointers or failure case.
    ensure!(
        metadata.is_file() && metadata.uid() == unsafe { libc::getuid() },
        "the log of {name} must be a regular file you own"
    );
    Ok(file)
}

/// Empties the directory for a new run, so `logs` shows only processes of the latest one.
pub fn reset(directory: &Path) -> Result<()> {
    crate::private::directory(directory)?;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// The processes with a log, sorted, so each keeps one color everywhere.
pub fn names(directory: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(names),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        if let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_suffix(".log"))
            && valid(name)
            && entry.file_type()?.is_file()
        {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// The last `count` lines `name` wrote, reading back from the end only as far as they reach.
pub fn tail(directory: &Path, name: &str, count: usize) -> Result<Vec<String>> {
    ensure!(valid(name), "invalid process name {name:?}");
    let mut file = read_private(&directory.join(format!("{name}.log")))?;
    let length = file.metadata()?.len();
    let mut window = 64 * 1024u64;
    let bytes = loop {
        let start = length.saturating_sub(window);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.by_ref().take(length - start).read_to_end(&mut bytes)?;
        if start == 0 {
            break bytes;
        }
        // More newlines than lines wanted: the first, possibly cut, line is not among them.
        if bytes.iter().filter(|byte| **byte == b'\n').count() > count {
            let first = bytes
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(0, |end| end + 1);
            break bytes.split_off(first);
        }
        window *= 4;
    };
    let lines: Vec<String> = String::from_utf8_lossy(&bytes).lines().map(clean).collect();
    Ok(lines[lines.len().saturating_sub(count)..].to_vec())
}

/// Reads new complete lines from every log in a directory, including logs that appear later.
pub struct Follower {
    directory: PathBuf,
    files: BTreeMap<String, Followed>,
    /// Logs present when following started are read from this many lines before their end, or
    /// from the start with None.
    back: Option<usize>,
    /// Copy a log to `<name>.log.1` and empty it once it grows past this many bytes.
    limit: Option<u64>,
}
struct Followed {
    file: File,
    offset: u64,
    partial: Vec<u8>,
}
impl Follower {
    pub fn new(directory: &Path, back: Option<usize>, limit: Option<u64>) -> Self {
        Self {
            directory: directory.to_owned(),
            files: BTreeMap::new(),
            back,
            limit,
        }
    }

    /// Calls `emit` with each new line, in name order per call. Missing logs are skipped.
    pub fn poll(&mut self, mut emit: impl FnMut(&str, &str)) -> Result<()> {
        for name in names(&self.directory)? {
            if !self.files.contains_key(&name) {
                let Ok(mut file) = read_private(&self.directory.join(format!("{name}.log"))) else {
                    continue;
                };
                let offset = match self.back {
                    Some(lines) => {
                        let mut bytes = Vec::new();
                        file.read_to_end(&mut bytes)?;
                        start_of_last(&bytes, lines) as u64
                    }
                    None => 0,
                };
                self.files.insert(
                    name.clone(),
                    Followed {
                        file,
                        offset,
                        partial: Vec::new(),
                    },
                );
            }
        }
        self.back = None;
        for (name, followed) in &mut self.files {
            let length = followed.file.metadata()?.len();
            // Emptied by rotation or a new run: start again from the top.
            if length < followed.offset {
                followed.offset = 0;
                followed.partial.clear();
            }
            followed.file.seek(SeekFrom::Start(followed.offset))?;
            let mut bytes = Vec::new();
            followed.file.read_to_end(&mut bytes)?;
            followed.offset += bytes.len() as u64;
            followed.partial.extend_from_slice(&bytes);
            while let Some(end) = followed.partial.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = followed.partial.drain(..=end).collect();
                emit(name, &clean(&String::from_utf8_lossy(&line[..end])));
            }
            // Output without newlines is shown in pieces rather than held without bound.
            if followed.partial.len() > LINE_LIMIT {
                let line = std::mem::take(&mut followed.partial);
                emit(name, &clean(&String::from_utf8_lossy(&line)));
            }
            if let Some(limit) = self.limit
                && followed.offset > limit
            {
                let path = self.directory.join(format!("{name}.log"));
                // Through a new file and a rename, so a link at `.log.1` is replaced, not followed.
                let previous = self.directory.join(format!("{name}.log.1"));
                let partial = self.directory.join(format!(".{name}.log.1.part"));
                let _ = std::fs::remove_file(&partial);
                let mut copy = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(&partial)?;
                std::io::copy(&mut read_private(&path)?, &mut copy)?;
                std::fs::rename(&partial, &previous)?;
                // The writer appends, so it continues at the start of the emptied file.
                OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(&path)?
                    .set_len(0)?;
                followed.offset = 0;
            }
        }
        Ok(())
    }
}

/// Where the last `lines` complete lines of `bytes` begin; the end for none.
fn start_of_last(bytes: &[u8], lines: usize) -> usize {
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |end| end + 1);
    if lines == 0 {
        return complete;
    }
    bytes[..complete]
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, byte)| **byte == b'\n')
        .nth(lines)
        .map_or(0, |(index, _)| index + 1)
}

/// Longest piece of output shown without a newline.
const LINE_LIMIT: usize = 64 * 1024;

fn read_private(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?)
}

/// Names are manifest keys, hooks or `parcela-` publishers: no path separators or dots.
fn valid(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// A line safe to print on a terminal: control characters and escape sequences are dropped,
/// except plain SGR colors, which are reset at the end so they never reach the next line. Process
/// output can carry anything a client sent, so nothing may move the cursor, set a title or reach
/// the clipboard.
fn clean(line: &str) -> String {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut text = String::with_capacity(line.len());
    let mut colored = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            if c == '\t' || !c.is_control() {
                text.push(c);
            }
            continue;
        }
        match chars.next() {
            // CSI: parameters 0x30-0x3F, intermediates 0x20-0x2F, one final byte 0x40-0x7E.
            Some('[') => {
                let mut parameters = String::new();
                while let Some(&next) = chars.peek().filter(|c| ('\x30'..='\x3f').contains(*c)) {
                    parameters.push(next);
                    chars.next();
                }
                let mut intermediates = false;
                while chars.next_if(|c| ('\x20'..='\x2f').contains(c)).is_some() {
                    intermediates = true;
                }
                // A malformed sequence is dropped up to here; what follows is read as text.
                if let Some(last) = chars.next_if(|c| ('\x40'..='\x7e').contains(c))
                    && last == 'm'
                    && !intermediates
                    && parameters
                        .chars()
                        .all(|c| c.is_ascii_digit() || c == ';' || c == ':')
                {
                    text.push_str("\x1b[");
                    text.push_str(&parameters);
                    text.push('m');
                    colored = true;
                }
            }
            // OSC, DCS, SOS, PM and APC strings run to BEL or ST.
            Some(']' | 'P' | 'X' | '^' | '_') => {
                while let Some(next) = chars.next() {
                    if next == '\x07' || (next == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            // Two-character escapes, such as ESC 7 or ESC (.
            _ => {}
        }
    }
    if colored {
        text.push_str("\x1b[0m");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn lines_keep_colors_and_lose_titles_and_cursor_moves() {
        assert_eq!(clean("\x1b[32mok\x1b[0m\r"), "\x1b[32mok\x1b[0m\x1b[0m");
        assert_eq!(clean("\x1b]0;title\x07[setup]\tbroken"), "[setup]\tbroken");
        assert_eq!(clean("a\x1b[2Kb\x1b]2;t\x1b\\c"), "abc");
        // Sequences hidden inside a would-be color code never survive.
        assert_eq!(clean("\x1b[\x1b(0mx"), "0mx");
        assert_eq!(clean("\x1b[\x1b]52;;0000\x07mx"), "mx");
        assert_eq!(clean("\x1b[\r\x08mspoof"), "mspoof");
        assert_eq!(clean("a\x1b7b\u{9b}2Jc"), "ab2Jc");
        assert_eq!(clean("\x1bPq#0\x1b\\d"), "d");
    }

    #[test]
    fn a_tail_reads_back_only_as_far_as_its_lines() {
        let temp = tempfile::TempDir::new().unwrap();
        let directory = temp.path();
        let mut web = open(directory, "web").unwrap();
        for line in 0..20_000 {
            writeln!(web, "line {line:05} {}", "x".repeat(20)).unwrap();
        }
        write!(web, "partial").unwrap();
        let x = "x".repeat(20);
        assert_eq!(
            tail(directory, "web", 3).unwrap(),
            [
                format!("line 19998 {x}"),
                format!("line 19999 {x}"),
                "partial".into()
            ]
        );
        let many = tail(directory, "web", 5000).unwrap();
        assert_eq!(many.len(), 5000);
        assert_eq!(many[0], format!("line 15001 {x}"));
        assert_eq!(tail(directory, "web", 30_000).unwrap().len(), 20_001);
    }

    #[test]
    fn a_follower_reads_new_lines_and_starts_over_after_rotation() {
        let temp = tempfile::TempDir::new().unwrap();
        let directory = temp.path();
        let mut web = open(directory, "web").unwrap();
        writeln!(web, "old").unwrap();
        let mut follower = Follower::new(directory, Some(0), Some(16));
        let mut seen = Vec::new();
        follower
            .poll(|name, line| seen.push(format!("{name}:{line}")))
            .unwrap();
        assert!(seen.is_empty());
        write!(web, "one\ntw").unwrap();
        let mut api = open(directory, "api").unwrap();
        writeln!(api, "up").unwrap();
        follower
            .poll(|name, line| seen.push(format!("{name}:{line}")))
            .unwrap();
        assert_eq!(seen, ["api:up", "web:one"]);
        writeln!(web, "o and more than the limit").unwrap();
        follower
            .poll(|name, line| seen.push(format!("{name}:{line}")))
            .unwrap();
        assert_eq!(seen.last().unwrap(), "web:two and more than the limit");
        assert_eq!(
            std::fs::metadata(directory.join("web.log")).unwrap().len(),
            0
        );
        assert!(directory.join("web.log.1").exists());
        writeln!(web, "after").unwrap();
        follower
            .poll(|name, line| seen.push(format!("{name}:{line}")))
            .unwrap();
        assert_eq!(seen.last().unwrap(), "web:after");
        assert_eq!(names(directory).unwrap(), ["api", "web"]);
        assert_eq!(tail(directory, "web", 5).unwrap(), ["after"]);
        let mut from_tail = Vec::new();
        Follower::new(directory, Some(1), None)
            .poll(|name, line| from_tail.push(format!("{name}:{line}")))
            .unwrap();
        assert_eq!(from_tail, ["api:up", "web:after"]);
        assert_eq!(start_of_last(b"a\nb\nc\npartial", 2), 2);
    }
}

//! URLs an environment's processes asked a browser to open. Processes see `BROWSER` as a shim
//! that runs `parcela browse`, and every browser showing the environment offers the request.
use crate::private;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const KEPT: usize = 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Open {
    pub id: String,
    pub at_unix_ms: u128,
    pub target: Target,
    /// An opened page's `<title>`, read once when it was opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Url { url: String },
    File { path: PathBuf },
}

impl Target {
    /// A browser argument: an `http(s)://` URL, a `file://` URL, or an absolute file path.
    pub fn parse(argument: &str) -> Result<Self> {
        let lowered = argument.to_ascii_lowercase();
        if lowered.starts_with("http://") || lowered.starts_with("https://") {
            return Ok(Self::Url {
                url: argument.to_owned(),
            });
        }
        let path = match argument.strip_prefix("file://") {
            Some(rest) => PathBuf::from(decode(rest.strip_prefix("localhost").unwrap_or(rest))?),
            None => PathBuf::from(argument),
        };
        ensure!(
            path.is_absolute(),
            "cannot open {argument}: expected an http(s) URL, a file:// URL or an absolute path"
        );
        let path = path
            .canonicalize()
            .with_context(|| format!("cannot open {}", path.display()))?;
        ensure!(path.is_file(), "cannot open {}: not a file", path.display());
        Ok(Self::File { path })
    }
}

pub fn file(runtime: &Path) -> PathBuf {
    runtime.join("opens.json")
}

pub fn record(runtime: &Path, target: Target) -> Result<Open> {
    let _lock = private::Lock::wait(&runtime.join("opens.lock"))?;
    let mut opens = list(runtime)?;
    let title = match &target {
        Target::File { path } => title(path),
        Target::Url { .. } => None,
    };
    let open = Open {
        id: uuid::Uuid::new_v4().simple().to_string(),
        at_unix_ms: now_unix_ms(),
        target,
        title,
    };
    opens.push(open.clone());
    opens.drain(..opens.len().saturating_sub(KEPT));
    private::write_json(&file(runtime), &opens)?;
    Ok(open)
}

/// The page title, read without blocking on a FIFO.
fn title(path: &Path) -> Option<String> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut head = String::new();
    file.take(64 * 1024).read_to_string(&mut head).ok()?;
    let lower = head.to_ascii_lowercase();
    let start = lower.find("<title>")? + "<title>".len();
    let end = start + lower[start..].find("</title>")?;
    let title = head[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!title.is_empty()).then_some(title)
}

pub fn list(runtime: &Path) -> Result<Vec<Open>> {
    let path = file(runtime);
    if !path.exists() {
        return Ok(Vec::new());
    }
    private::read_json(&path)
}

/// The opened file, or a file beside it so a page's relative links and images resolve. `name` is
/// one path component; symlinks that leave the opened file's directory are refused. Beside the
/// opened file only page assets are offered, never dotfiles, since a page opened from a checkout's
/// root sits next to its secrets.
pub fn sibling(runtime: &Path, id: &str, name: &str) -> Result<PathBuf> {
    let open = list(runtime)?
        .into_iter()
        .find(|open| open.id == id)
        .context("no such open request")?;
    let Target::File { path } = open.target else {
        bail!("open request is not a file");
    };
    let directory = path.parent().context("opened file has no directory")?;
    let mut components = Path::new(name).components();
    ensure!(
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none(),
        "invalid file name"
    );
    let sibling = directory.join(name).canonicalize()?;
    ensure!(
        sibling.parent() == Some(directory) && sibling.is_file(),
        "file outside the opened directory"
    );
    ensure!(sibling == path || asset(&sibling), "file type not offered");
    Ok(sibling)
}

fn asset(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    !name.starts_with('.')
        && matches!(
            extension.as_deref(),
            Some(
                "html"
                    | "htm"
                    | "css"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "avif"
                    | "svg"
                    | "ico"
                    | "woff"
                    | "woff2"
            )
        )
}

pub fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_millis())
        .unwrap_or_default()
}

pub fn decode(text: &str) -> Result<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text
                .get(index + 1..index + 3)
                .context("invalid percent escape")?;
            out.push(u8::from_str_radix(hex, 16).context("invalid percent escape")?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Ok(String::from_utf8(out)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            directory.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        directory
    }

    #[test]
    fn targets_accept_web_urls_file_urls_and_absolute_paths() {
        let directory = runtime();
        let page = directory.path().join("rich page.html");
        std::fs::write(&page, "<title>Mail</title>").unwrap();
        let page = page.canonicalize().unwrap();
        assert_eq!(
            Target::parse("https://shop.localhost:1443/a?b=c").unwrap(),
            Target::Url {
                url: "https://shop.localhost:1443/a?b=c".into()
            }
        );
        let url = format!("file://{}", page.display()).replace(' ', "%20");
        assert_eq!(
            Target::parse(&url).unwrap(),
            Target::File { path: page.clone() }
        );
        assert_eq!(
            Target::parse(page.to_str().unwrap()).unwrap(),
            Target::File { path: page }
        );
        assert!(Target::parse("relative.html").is_err());
        assert!(Target::parse("javascript:alert(1)").is_err());
        assert!(Target::parse(directory.path().to_str().unwrap()).is_err());
    }

    #[test]
    fn records_keep_the_latest_requests() {
        let directory = runtime();
        for index in 0..KEPT + 3 {
            record(
                directory.path(),
                Target::Url {
                    url: format!("https://shop.localhost/{index}"),
                },
            )
            .unwrap();
        }
        let opens = list(directory.path()).unwrap();
        assert_eq!(opens.len(), KEPT);
        assert_eq!(
            opens.last().unwrap().target,
            Target::Url {
                url: format!("https://shop.localhost/{}", KEPT + 2)
            }
        );
    }

    #[test]
    fn titles_are_read_once_and_never_wait_on_a_fifo() {
        let directory = runtime();
        let page = directory.path().join("mail.html");
        std::fs::write(&page, "<TITLE>\n Your  link </title>").unwrap();
        let open = record(
            directory.path(),
            Target::parse(page.to_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(open.title.as_deref(), Some("Your link"));
        std::fs::write(&page, "<title>changed</title>").unwrap();
        assert_eq!(
            list(directory.path()).unwrap()[0].title.as_deref(),
            Some("Your link")
        );

        let fifo = directory.path().join("pipe.html");
        let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path; mkfifo reads it and returns.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert_eq!(title(&fifo), None);
    }

    #[test]
    fn siblings_stay_inside_the_opened_directory() {
        let directory = runtime();
        let mail = directory.path().join("mail");
        std::fs::create_dir(&mail).unwrap();
        std::fs::write(mail.join("rich.html"), "rich").unwrap();
        std::fs::write(mail.join("plain.html"), "plain").unwrap();
        std::fs::write(directory.path().join("secret"), "secret").unwrap();
        std::os::unix::fs::symlink(directory.path().join("secret"), mail.join("link")).unwrap();
        std::fs::write(mail.join(".env"), "TOKEN=1").unwrap();
        std::fs::write(mail.join("config.yaml"), "token: 1").unwrap();
        std::os::unix::fs::symlink(mail.join(".env"), mail.join("style.css")).unwrap();
        let open = record(
            directory.path(),
            Target::parse(mail.join("rich.html").to_str().unwrap()).unwrap(),
        )
        .unwrap();
        let runtime = directory.path();
        assert!(
            sibling(runtime, &open.id, "plain.html")
                .unwrap()
                .ends_with("mail/plain.html")
        );
        assert!(sibling(runtime, &open.id, "rich.html").is_ok());
        for name in [
            "../secret",
            "link",
            "",
            "/etc/passwd",
            "missing.html",
            ".env",
            "config.yaml",
            "style.css",
        ] {
            assert!(sibling(runtime, &open.id, name).is_err(), "{name}");
        }
        assert!(sibling(runtime, "unknown", "plain.html").is_err());
    }
}

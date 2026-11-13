//! Adds Parcela's badge script to HTML pages. Caddy sends this server only top-level page loads on
//! loopback app names, so assets, API calls, WebSockets, frames and non-browser clients never pass
//! through it. Anything that is not plain HTML goes back unchanged.
use crate::{dashboard, private, sockets};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    os::{
        fd::{AsRawFd, RawFd},
        unix::net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Which backend a request is for. Caddy always sets it and the app never sees it.
pub const UPSTREAM_HEADER: &str = "X-Parcela-Upstream";

const REQUEST_TIME: Duration = Duration::from_secs(5);
const CONNECT_TIME: Duration = Duration::from_secs(5);
const HEAD_LIMIT: usize = 256 * 1024;
/// How much of a page, and for how long after its headers, to look for where the tag goes. A page
/// past either limit goes through without the badge rather than stalling.
const SCAN_LIMIT: usize = 1024 * 1024;
const SCAN_TIME: Duration = Duration::from_secs(5);
/// Page loads served at once. Each holds three file descriptors, and the proxy may have only 256.
const PAGES_AT_ONCE: usize = 64;
/// Of those, how many one app can hold, so an app that hangs leaves room for the others.
const PAGES_PER_APP: usize = 16;
/// An app holding this few can always get a place, up to the limit...
const PAGES_ANY_APP: usize = 4;
/// ...and busier ones stop here, so that several hung apps together leave that room too.
const PAGES_BUSY_APPS: usize = 48;
/// How often a load waiting on its app checks that someone still wants the page.
const CLIENT_CHECK: Duration = Duration::from_secs(1);

const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];
/// Request headers that would stop the app sending the whole page uncompressed.
const REQUEST_DROPPED: [&str; 5] = [
    "accept-encoding",
    "if-none-match",
    "if-modified-since",
    "if-range",
    "range",
];
/// Response headers that describe the app's bytes, which no longer match once the tag is added.
const CHANGED_BY_TAG: [&str; 7] = [
    "accept-ranges",
    "content-digest",
    "content-length",
    "content-md5",
    "digest",
    "etag",
    "last-modified",
];

pub(crate) struct Server {
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    socket: PathBuf,
    identity: private::Identity,
}
impl Server {
    pub(crate) fn start(state: &Path) -> Result<Self> {
        let socket = sockets::injector(state)?;
        let (listener, identity) =
            dashboard::listen(&socket, &state.join("proxy/injector-socket.json"))?;
        let stopped = Arc::new(AtomicBool::new(false));
        let serving = Arc::clone(&stopped);
        let worker = thread::Builder::new()
            .name("parcela-injector".into())
            .spawn(move || accept(&listener, &serving))?;
        Ok(Self {
            stopped,
            worker: Some(worker),
            socket,
            identity,
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        // Wakes the blocking accept.
        drop(UnixStream::connect(&self.socket));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = self.identity.remove_socket(&self.socket);
    }
}

/// Gives back a page's place when its thread ends, however it ends.
struct Place(Arc<AtomicUsize>);
impl Drop for Place {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

type Apps = Arc<Mutex<HashMap<SocketAddr, usize>>>;

/// A page's place among those of its app.
struct AppPlace(Apps, SocketAddr);
impl AppPlace {
    fn take(apps: &Apps, app: SocketAddr) -> Option<Self> {
        let mut counts = apps.lock().ok()?;
        let total: usize = counts.values().sum();
        let count = counts.entry(app).or_default();
        let limit = if *count < PAGES_ANY_APP {
            PAGES_AT_ONCE
        } else {
            PAGES_BUSY_APPS
        };
        if total >= limit || *count >= PAGES_PER_APP {
            return None;
        }
        *count += 1;
        Some(Self(Arc::clone(apps), app))
    }
}
impl Drop for AppPlace {
    fn drop(&mut self) {
        let Ok(mut counts) = self.0.lock() else {
            return;
        };
        if let Some(count) = counts.get_mut(&self.1) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.1);
            }
        }
    }
}

fn accept(listener: &UnixListener, stopped: &AtomicBool) {
    let pages = Arc::new(AtomicUsize::new(0));
    let apps = Apps::default();
    while !stopped.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let place = Place(Arc::clone(&pages));
                let apps = Arc::clone(&apps);
                if pages.fetch_add(1, Ordering::Relaxed) >= PAGES_AT_ONCE {
                    let _ = reply(&mut stream, "503 Service Unavailable");
                    continue;
                }
                // A slow page holds one thread, never another page or the dashboard.
                drop(
                    thread::Builder::new()
                        .name("parcela-page".into())
                        .spawn(move || {
                            let _place = place;
                            drop(serve(stream, &apps));
                        }),
                );
            }
            // Out of file descriptors, for one: those come back, so keep listening.
            Err(error) => {
                crate::paint::Paint::note(&format!("page injector: {error}"));
                thread::sleep(CLIENT_CHECK);
            }
        }
    }
}

/// The app's side of a page load. It gives up when the client has gone, since an app that never
/// answers would otherwise hold this load's thread and sockets forever.
struct Upstream {
    stream: TcpStream,
    client: Option<RawFd>,
    limit: Option<Instant>,
    checked: Instant,
}
impl Upstream {
    fn new(stream: TcpStream, client: Option<RawFd>) -> Result<Self> {
        stream.set_read_timeout(Some(CLIENT_CHECK))?;
        Ok(Self {
            stream,
            client,
            limit: None,
            checked: Instant::now(),
        })
    }

    fn client_gone(&self) -> bool {
        let Some(client) = self.client else {
            return false;
        };
        let mut byte = 0u8;
        // SAFETY: the buffer is one writable byte, and the caller keeps the descriptor open for
        // as long as this value lives.
        let read = unsafe {
            libc::recv(
                client,
                (&raw mut byte).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        read == 0
            || (read < 0
                && !matches!(
                    io::Error::last_os_error().kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ))
    }
}
impl Read for Upstream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.stream.read(buffer) {
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if self.client_gone() {
                        return Err(io::ErrorKind::BrokenPipe.into());
                    }
                    if self.limit.is_some_and(|limit| Instant::now() >= limit) {
                        return Err(error);
                    }
                }
                // An app that trickles never times out, so look between reads as well.
                Ok(size) => {
                    if self.checked.elapsed() >= CLIENT_CHECK {
                        self.checked = Instant::now();
                        if self.client_gone() {
                            return Err(io::ErrorKind::BrokenPipe.into());
                        }
                    }
                    return Ok(size);
                }
                other => return other,
            }
        }
    }
}

fn serve(stream: UnixStream, apps: &Apps) -> Result<()> {
    stream.set_read_timeout(Some(REQUEST_TIME))?;
    let mut client = stream.try_clone()?;
    let mut request = BufReader::new(stream);
    let head = read_head(&mut request)?;
    let Some(forward) = forwarded_request(&head) else {
        return reply(&mut client, "400 Bad Request");
    };
    let Some(_place) = AppPlace::take(apps, forward.upstream) else {
        return reply(&mut client, "503 Service Unavailable");
    };
    let Ok(upstream) = TcpStream::connect_timeout(&forward.upstream, CONNECT_TIME) else {
        return reply(&mut client, "502 Bad Gateway");
    };
    (&upstream).write_all(&forward.bytes)?;
    let upstream = Upstream::new(upstream, Some(client.as_raw_fd()))?;
    respond(BufReader::new(upstream), &mut client)
}

fn reply(client: &mut UnixStream, status: &str) -> Result<()> {
    write!(
        client,
        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )?;
    Ok(())
}

/// One HTTP head without its blank line, split into lines without their line endings.
fn read_head<R: BufRead>(reader: &mut R) -> Result<Vec<Vec<u8>>> {
    let mut lines = Vec::new();
    let mut size = 0;
    loop {
        let mut line = Vec::new();
        reader
            .take((HEAD_LIMIT - size + 1) as u64)
            .read_until(b'\n', &mut line)?;
        ensure!(line.ends_with(b"\n"), "incomplete or oversized HTTP head");
        size += line.len();
        ensure!(size <= HEAD_LIMIT, "HTTP head too large");
        line.pop();
        if line.ends_with(b"\r") {
            line.pop();
        }
        if line.is_empty() {
            ensure!(!lines.is_empty(), "empty HTTP head");
            return Ok(lines);
        }
        lines.push(line);
    }
}

fn header_name(line: &[u8]) -> Option<String> {
    let colon = line.iter().position(|byte| *byte == b':')?;
    Some(String::from_utf8_lossy(&line[..colon]).to_ascii_lowercase())
}

fn header_value(line: &[u8]) -> &[u8] {
    let colon = line.iter().position(|byte| *byte == b':').unwrap_or(0);
    line[colon + 1..].trim_ascii()
}

struct Forward {
    upstream: SocketAddr,
    bytes: Vec<u8>,
}

/// The request for the app: asks for an uncompressed, whole page and removes what only concerned
/// this hop. `None` when Caddy named no loopback backend or the request carries a body, which no
/// page load has.
fn forwarded_request(head: &[Vec<u8>]) -> Option<Forward> {
    let upstream_header = UPSTREAM_HEADER.to_ascii_lowercase();
    let mut upstream = None;
    let mut bytes = head.first()?.clone();
    bytes.extend_from_slice(b"\r\n");
    for line in &head[1..] {
        let name = header_name(line)?;
        if name == upstream_header {
            let address: SocketAddr = std::str::from_utf8(header_value(line)).ok()?.parse().ok()?;
            upstream = Some(address).filter(|address| address.ip().is_loopback());
        } else if name == "content-length" {
            if header_value(line) != b"0" {
                return None;
            }
        } else if name != "transfer-encoding" {
            if !HOP_BY_HOP.contains(&name.as_str()) && !REQUEST_DROPPED.contains(&name.as_str()) {
                bytes.extend_from_slice(line);
                bytes.extend_from_slice(b"\r\n");
            }
        } else {
            return None;
        }
    }
    bytes.extend_from_slice(b"Accept-Encoding: identity\r\nConnection: close\r\n\r\n");
    Some(Forward {
        upstream: upstream?,
        bytes,
    })
}

fn respond<W: Write>(mut upstream: BufReader<Upstream>, client: &mut W) -> Result<()> {
    let head = loop {
        let head = read_head(&mut upstream)?;
        let status = status_code(&head[0]).context("invalid response status")?;
        match status {
            // Early Hints reach the browser; `100 Continue` concerned only this hop.
            103 => {
                client.write_all(&join(&head))?;
            }
            100..=199 => ensure!(status == 100, "unexpected interim response"),
            _ => break head,
        }
    };
    let status = status_code(&head[0]).context("invalid response status")?;
    let mut body = Body::new(&head, status, upstream);
    if !is_page(&head, status) {
        client.write_all(&response_head(&head, false))?;
        io::copy(&mut body, client)?;
        return Ok(());
    }
    let (page, position) = find_position(&mut body);
    let tagged = position.is_some();
    client.write_all(&response_head(&head, tagged))?;
    match position {
        Some(position) => {
            client.write_all(&page[..position])?;
            client.write_all(&tag(&head))?;
            client.write_all(&page[position..])?;
        }
        None => client.write_all(&page)?,
    }
    io::copy(&mut body, client)?;
    Ok(())
}

fn join(head: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for line in head {
        bytes.extend_from_slice(line);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend_from_slice(b"\r\n");
    bytes
}

fn status_code(line: &[u8]) -> Option<u16> {
    std::str::from_utf8(line.split(u8::is_ascii_whitespace).nth(1)?)
        .ok()?
        .parse()
        .ok()
}

fn headers<'a>(head: &'a [Vec<u8>], wanted: &str) -> impl Iterator<Item = &'a [u8]> {
    head[1..]
        .iter()
        .filter(move |line| header_name(line).as_deref() == Some(wanted))
        .map(|line| header_value(line))
}

/// Whether this response is a whole page the tag can go in. Redirects, partial content, downloads
/// and anything the app compressed anyway pass through untouched.
fn is_page(head: &[Vec<u8>], status: u16) -> bool {
    let html = headers(head, "content-type").next().is_some_and(|value| {
        let value = String::from_utf8_lossy(value).to_ascii_lowercase();
        value.split(';').next().map(str::trim) == Some("text/html")
    });
    let identity = headers(head, "content-encoding").all(|value| value == b"identity");
    let inline = headers(head, "content-disposition").all(|value| {
        !String::from_utf8_lossy(value)
            .to_ascii_lowercase()
            .starts_with("attachment")
    });
    html && identity && inline && !matches!(status, 204 | 206 | 304 | 300..=399)
}

fn response_head(head: &[Vec<u8>], tagged: bool) -> Vec<u8> {
    let chunked = chunked(head);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&head[0]);
    bytes.extend_from_slice(b"\r\n");
    for line in &head[1..] {
        let name = header_name(line).unwrap_or_default();
        let name = name.as_str();
        let dropped = HOP_BY_HOP.contains(&name)
            || (chunked && name == "content-length")
            || (tagged && CHANGED_BY_TAG.contains(&name));
        if !dropped {
            bytes.extend_from_slice(line);
            bytes.extend_from_slice(b"\r\n");
        }
    }
    bytes.extend_from_slice(b"Connection: close\r\n\r\n");
    bytes
}

fn chunked(head: &[Vec<u8>]) -> bool {
    headers(head, "transfer-encoding").any(|value| {
        String::from_utf8_lossy(value)
            .to_ascii_lowercase()
            .contains("chunked")
    })
}

/// Reads as far as where the tag goes. `None` when no place turned up within the limits, or the
/// page is not markup at all; the bytes read so far are still returned.
fn find_position(body: &mut Body) -> (Vec<u8>, Option<usize>) {
    let deadline = Instant::now() + SCAN_TIME;
    body.set_limit(deadline);
    let mut page = Vec::new();
    let mut buffer = [0; 16 * 1024];
    let mut resume = 0;
    let position = loop {
        match scan(&page, resume) {
            Ok(position) => break Some(position),
            Err(next) => resume = next,
        }
        if Instant::now() >= deadline || page.len() >= SCAN_LIMIT {
            break None;
        }
        match body.read(&mut buffer) {
            // UTF-16 and the like have NUL bytes, and an ASCII tag would be garbage in them.
            Ok(0) => break (page.contains(&b'<') && !page.contains(&0)).then_some(page.len()),
            Ok(size) => page.extend_from_slice(&buffer[..size]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
            Err(_) => break None,
        }
    };
    body.clear_limit();
    (page, position)
}

/// The offset of `</head>`, or of `<body` for a page without one, looking from `at`, an offset no
/// tag or comment straddles. Comments, quoted attribute values and raw-text elements like
/// `<script>` do not count, so a tag never lands inside them. `Err` holds where to look again once
/// more of the page arrives, the start of what is still incomplete, so a page that comes in many
/// reads is still read through only about once.
fn scan(html: &[u8], mut at: usize) -> Result<usize, usize> {
    const RAW_TEXT: [&[u8]; 9] = [
        b"script",
        b"style",
        b"title",
        b"textarea",
        b"noscript",
        b"xmp",
        b"iframe",
        b"noembed",
        b"noframes",
    ];
    loop {
        let Some(start) = html
            .get(at..)
            .and_then(|rest| rest.iter().position(|byte| *byte == b'<'))
        else {
            return Err(html.len());
        };
        let start = at + start;
        // Whatever is cut off here is looked at again, whole, from `start`.
        macro_rules! whole {
            ($found:expr) => {
                match $found {
                    Some(found) => found,
                    None => return Err(start),
                }
            };
        }
        let rest = &html[start..];
        if rest.starts_with(b"<!--") {
            // From the second byte, so that `<!-->` and `<!--->` end where browsers end them.
            let end = find(&rest[2..], b"-->").map(|end| end + 3);
            let bang = find(&rest[2..], b"--!>").map(|end| end + 4);
            at = start + 2 + whole!(end.into_iter().chain(bang).min());
            continue;
        }
        if matches!(whole!(rest.get(1)), b'!' | b'?') {
            at = start + whole!(find(rest, b">")) + 1;
            continue;
        }
        let closing = rest[1] == b'/';
        let name_start = start + 1 + usize::from(closing);
        if !whole!(html.get(name_start)).is_ascii_alphabetic() {
            at = start + 1;
            continue;
        }
        let name_length = whole!(
            html[name_start..]
                .iter()
                .position(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b':'))
        );
        let name = html[name_start..name_start + name_length].to_ascii_lowercase();
        let end = name_start + name_length + whole!(tag_end(&html[name_start + name_length..]));
        match (closing, name.as_slice()) {
            // Before `<base>`, which would otherwise send the tag's URL to another host.
            (true, b"head") | (false, b"body" | b"base") => return Ok(start),
            (false, name) if RAW_TEXT.contains(&name) => {
                let mut closer = b"</".to_vec();
                closer.extend_from_slice(name);
                let content = &html[end + 1..];
                let mut from = 0;
                // `</scripty>` does not close a script.
                at = loop {
                    let found = from
                        + whole!(
                            content[from..]
                                .windows(closer.len())
                                .position(|part| part.eq_ignore_ascii_case(&closer))
                        );
                    match whole!(content.get(found + closer.len())) {
                        b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => {
                            break end + 1 + found;
                        }
                        _ => from = found + closer.len(),
                    }
                };
            }
            _ => at = end + 1,
        }
    }
}

/// The offset of the `>` that ends a tag, skipping quoted attribute values.
fn tag_end(tag: &[u8]) -> Option<usize> {
    let mut quote = None;
    for (index, byte) in tag.iter().enumerate() {
        match (quote, byte) {
            (None, b'>') => return Some(index),
            (None, b'"' | b'\'') => quote = Some(*byte),
            (Some(open), byte) if open == *byte => quote = None,
            _ => (),
        }
    }
    None
}

fn find(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes.windows(needle.len()).position(|part| part == needle)
}

/// The script tag, with the nonce the page's policy already allows when it uses one. A policy that
/// allows neither that nor same-origin scripts blocks the badge; Parcela never relaxes it.
fn tag(head: &[Vec<u8>]) -> Vec<u8> {
    match headers(head, "content-security-policy").find_map(nonce) {
        Some(nonce) => {
            format!("<script defer src=\"/_parcela/widget.js\" nonce=\"{nonce}\"></script>")
        }
        None => "<script defer src=\"/_parcela/widget.js\"></script>".to_owned(),
    }
    .into_bytes()
}

fn nonce(policy: &[u8]) -> Option<String> {
    let policy = String::from_utf8_lossy(policy);
    let directive = |wanted: &str| {
        policy.split(';').find(|directive| {
            directive
                .split_ascii_whitespace()
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
        })
    };
    let scripts = directive("script-src-elem")
        .or_else(|| directive("script-src"))
        .or_else(|| directive("default-src"))?;
    scripts
        .split_ascii_whitespace()
        .find_map(|source| source.strip_prefix("'nonce-")?.strip_suffix('\''))
        .filter(|nonce| {
            !nonce.is_empty()
                && nonce
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"+/=-_".contains(&byte))
        })
        .map(str::to_owned)
}

enum Framing {
    Empty,
    UntilClose,
    Length(u64),
    Chunked { remaining: u64, line: Vec<u8> },
}

/// A response body with its framing removed.
struct Body {
    reader: BufReader<Upstream>,
    framing: Framing,
}
impl Body {
    fn new(head: &[Vec<u8>], status: u16, reader: BufReader<Upstream>) -> Self {
        let length = headers(head, "content-length")
            .next()
            .and_then(|value| std::str::from_utf8(value).ok()?.parse().ok());
        let framing = if matches!(status, 204 | 304) {
            Framing::Empty
        } else if chunked(head) {
            Framing::Chunked {
                remaining: 0,
                line: Vec::new(),
            }
        } else if let Some(length) = length {
            Framing::Length(length)
        } else {
            Framing::UntilClose
        };
        Self { reader, framing }
    }

    fn set_limit(&mut self, limit: Instant) {
        self.reader.get_mut().limit = Some(limit);
    }

    fn clear_limit(&mut self) {
        self.reader.get_mut().limit = None;
    }
}
impl Read for Body {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        match &mut self.framing {
            Framing::Empty => Ok(0),
            Framing::UntilClose => self.reader.read(output),
            Framing::Length(remaining) => {
                let wanted = output
                    .len()
                    .min(usize::try_from(*remaining).unwrap_or(usize::MAX));
                if wanted == 0 {
                    return Ok(0);
                }
                let size = self.reader.read(&mut output[..wanted])?;
                if size == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                *remaining -= size as u64;
                Ok(size)
            }
            Framing::Chunked { remaining, line } => {
                while *remaining == 0 {
                    // `line` outlives a read timeout, so a size split across packets stays whole.
                    let room = HEAD_LIMIT.saturating_sub(line.len()) as u64;
                    (&mut self.reader).take(room).read_until(b'\n', line)?;
                    if !line.ends_with(b"\n") {
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                    let text = String::from_utf8_lossy(line);
                    let size = text.split(';').next().unwrap_or_default().trim();
                    if !size.is_empty() {
                        let size = u64::from_str_radix(size, 16)
                            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
                        if size == 0 {
                            self.framing = Framing::Empty;
                            return Ok(0);
                        }
                        *remaining = size;
                    }
                    line.clear();
                }
                let wanted = output
                    .len()
                    .min(usize::try_from(*remaining).unwrap_or(usize::MAX));
                let size = self.reader.read(&mut output[..wanted])?;
                if size == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                *remaining -= size as u64;
                Ok(size)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn insertion_point(html: &[u8]) -> Option<usize> {
        scan(html, 0).ok()
    }

    fn lines(text: &str) -> Vec<Vec<u8>> {
        text.split("\r\n")
            .map(|line| line.as_bytes().to_vec())
            .collect()
    }

    #[test]
    fn finds_where_the_tag_goes() {
        let at = |html: &str| insertion_point(html.as_bytes());
        assert_eq!(at("<html><head><title>x</title></head>"), Some(28));
        assert_eq!(at("<!doctype html><HEAD></HEAD>"), Some(21));
        assert_eq!(at("<html><body class=\"a\">"), Some(6));
    }

    #[test]
    fn a_head_with_no_end_stops_at_the_limit() {
        let endless = vec![b'x'; HEAD_LIMIT * 4];
        let mut reader = &endless[..];
        assert!(read_head(&mut reader).is_err());
        assert_eq!(reader.len(), endless.len() - HEAD_LIMIT - 1);
    }

    #[test]
    fn one_app_cannot_take_every_place() {
        let apps = Apps::default();
        let first: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let second: SocketAddr = "127.0.0.1:2".parse().unwrap();
        let held: Vec<_> = (0..PAGES_PER_APP)
            .map(|_| AppPlace::take(&apps, first).unwrap())
            .collect();
        assert!(AppPlace::take(&apps, first).is_none());
        assert!(AppPlace::take(&apps, second).is_some());
        drop(held);
        assert!(AppPlace::take(&apps, first).is_some());
    }

    #[test]
    fn several_hung_apps_leave_room_for_the_rest() {
        let apps = Apps::default();
        let address = |port| SocketAddr::from(([127, 0, 0, 1], port));
        let hung: Vec<_> = (1..=4)
            .flat_map(|port| (0..PAGES_PER_APP).map(move |_| port))
            .filter_map(|port| AppPlace::take(&apps, address(port)))
            .collect();
        assert_eq!(hung.len(), PAGES_BUSY_APPS + PAGES_ANY_APP);
        let others: Vec<_> = (0..PAGES_ANY_APP)
            .map(|_| AppPlace::take(&apps, address(99)).expect("a place for a quiet app"))
            .collect();
        assert_eq!(others.len(), PAGES_ANY_APP);
    }

    #[test]
    fn leaves_other_encodings_alone_and_goes_before_base() {
        let at = |html: &str| insertion_point(html.as_bytes());
        let base = "<head><base href=\"https://cdn.example/\"><title>x</title></head>";
        assert_eq!(at(base), Some(6));
        let output = through(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n<\0h\0t\0m\0l\0");
        assert!(!output.contains("widget.js"), "{output}");
    }

    #[test]
    fn follows_how_browsers_end_comments_and_raw_text() {
        let at = |html: &str| insertion_point(html.as_bytes());
        let script = "<head><script>var a=\"</scripty></head>\";</script></head>";
        assert_eq!(at(script), Some(script.rfind("</head>").unwrap()));
        for comment in ["<!-->", "<!--->", "<!-- x --!>", "<!-- x -->"] {
            let html = format!("<head>{comment}</head>");
            assert_eq!(at(&html), Some(html.rfind("</head>").unwrap()), "{comment}");
        }
    }

    #[test]
    fn a_load_ends_when_its_client_leaves_an_app_that_never_answers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let app = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (client, proxy) = UnixStream::pair().unwrap();
        let upstream = Upstream::new(app, Some(proxy.as_raw_fd())).unwrap();
        let load = thread::spawn(move || {
            let mut output = Vec::new();
            let _proxy = proxy;
            respond(BufReader::new(upstream), &mut output)
        });
        drop(client);
        let started = Instant::now();
        while !load.is_finished() {
            assert!(
                started.elapsed() < 5 * CLIENT_CHECK,
                "the load is still waiting"
            );
            thread::sleep(Duration::from_millis(20));
        }
        assert!(load.join().unwrap().is_err());
    }

    #[test]
    fn a_trickling_app_loses_no_bytes_and_ends_when_its_client_leaves() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (client, proxy) = UnixStream::pair().unwrap();
        let mut upstream = Upstream::new(stream, Some(proxy.as_raw_fd())).unwrap();
        let (mut app, _) = listener.accept().unwrap();
        app.write_all(b"first").unwrap();
        upstream.checked = Instant::now() - CLIENT_CHECK;
        let mut buffer = [0; 5];
        assert_eq!(upstream.read(&mut buffer).unwrap(), 5);
        assert_eq!(&buffer, b"first");
        drop(client);
        app.write_all(b"second").unwrap();
        upstream.checked = Instant::now() - CLIENT_CHECK;
        assert_eq!(
            upstream.read(&mut buffer).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        drop(proxy);
    }

    #[test]
    fn never_lands_inside_a_comment_string_or_attribute() {
        let at = |html: &str| insertion_point(html.as_bytes());
        let script = "<head><script>let a = \"</head>\";</script></head>";
        assert_eq!(at(script), Some(script.rfind("</head>").unwrap()));
        let comment = "<head><!-- </head> --></head>";
        assert_eq!(at(comment), Some(comment.rfind("</head>").unwrap()));
        let attribute = "<head><meta content=\"</head><body>\"></head>";
        assert_eq!(at(attribute), Some(attribute.rfind("</head>").unwrap()));
        let title = "<head><title><body></title></head>";
        assert_eq!(at(title), Some(title.rfind("</head>").unwrap()));
    }

    #[test]
    fn waits_for_more_when_a_page_stops_mid_token() {
        for partial in [
            "<head",
            "<head><",
            "<head></he",
            "<head><!-- </head>",
            "<head><script>\"</head>",
            "<head><meta content=\"</head>",
            "plain text",
        ] {
            assert_eq!(insertion_point(partial.as_bytes()), None, "{partial}");
        }
    }

    #[test]
    fn reuses_only_the_nonce_that_applies_to_scripts() {
        let found = |policy: &str| nonce(policy.as_bytes());
        assert_eq!(
            found("script-src 'self' 'nonce-a+b/c='"),
            Some("a+b/c=".into())
        );
        assert_eq!(found("default-src 'nonce-x'"), Some("x".into()));
        assert_eq!(found("default-src 'nonce-x'; script-src 'self'"), None);
        assert_eq!(found("style-src 'nonce-x'"), None);
        assert_eq!(found("script-src 'nonce-\"><x'"), None);
    }

    #[test]
    fn only_whole_html_pages_take_the_tag() {
        let page = |headers: &str, status| {
            is_page(&lines(&format!("HTTP/1.1 {status} X\r\n{headers}")), status)
        };
        assert!(page("Content-Type: text/html; charset=utf-8", 200));
        assert!(page("content-type: TEXT/HTML", 500));
        assert!(!page("Content-Type: application/json", 200));
        assert!(!page(
            "Content-Type: text/html\r\nContent-Encoding: gzip",
            200
        ));
        assert!(!page(
            "Content-Type: text/html\r\nContent-Disposition: attachment",
            200
        ));
        assert!(!page("Content-Type: text/html", 302));
        assert!(!page("Content-Type: text/html", 206));
        assert!(!page("X-Other: 1", 200));
    }

    #[test]
    fn asks_the_app_for_a_whole_uncompressed_page() {
        let forward = forwarded_request(&lines(
            "GET /a HTTP/1.1\r\nHost: a.localhost\r\nAccept-Encoding: gzip, br\r\nIf-None-Match: \"1\"\r\nConnection: keep-alive\r\nX-Parcela-Upstream: 127.0.0.1:20000\r\nCookie: s=1",
        ))
        .unwrap();
        assert_eq!(forward.upstream, "127.0.0.1:20000".parse().unwrap());
        assert_eq!(
            String::from_utf8(forward.bytes).unwrap(),
            "GET /a HTTP/1.1\r\nHost: a.localhost\r\nCookie: s=1\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn refuses_any_backend_but_loopback_and_any_body() {
        for refused in [
            "GET / HTTP/1.1\r\nHost: a",
            "GET / HTTP/1.1\r\nX-Parcela-Upstream: 192.168.1.5:80",
            "GET / HTTP/1.1\r\nX-Parcela-Upstream: not-an-address",
            "GET / HTTP/1.1\r\nX-Parcela-Upstream: 127.0.0.1:1\r\nContent-Length: 3",
            "GET / HTTP/1.1\r\nX-Parcela-Upstream: 127.0.0.1:1\r\nTransfer-Encoding: chunked",
        ] {
            assert!(forwarded_request(&lines(refused)).is_none(), "{refused}");
        }
    }

    /// Serves `response` to one connection and returns what `respond` makes of it.
    fn through(response: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let app = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(response).unwrap();
        });
        let mut output = Vec::new();
        respond(
            BufReader::new(Upstream::new(TcpStream::connect(address).unwrap(), None).unwrap()),
            &mut output,
        )
        .unwrap();
        app.join().unwrap();
        String::from_utf8(output).unwrap()
    }

    const TAG: &str = "<script defer src=\"/_parcela/widget.js\"></script>";

    const MARKUP: [&[u8]; 34] = [
        b"<",
        b">",
        b"</",
        b"<!--",
        b"-->",
        b"--!>",
        b"<!",
        b"<?",
        b"<!doctype html>",
        b"<head>",
        b"</head>",
        b"</HEAD >",
        b"<body",
        b"<BODY>",
        b" ",
        b"\"",
        b"'",
        b"=",
        b"<script>",
        b"</script>",
        b"</scripty>",
        b"<style>",
        b"</style",
        b"<title>",
        b"</title>",
        b"<base href=x>",
        b"<meta a='>'>",
        b"x",
        b"\n",
        b"<b",
        b"<-",
        b"\0",
        "é".as_bytes(),
        b"<textarea>",
    ];

    /// The injector looks again after every read, so the place must not depend on how the page
    /// arrived: once a prefix has one, the whole page has the same one.
    #[test]
    fn where_the_tag_goes_does_not_depend_on_how_the_page_arrives() {
        crate::fuzz::rounds(2000, |rng| {
            let html = rng.text(&MARKUP, 40);
            let whole = insertion_point(&html);
            if let Some(at) = whole {
                assert_eq!(html[at], b'<', "{:?}", String::from_utf8_lossy(&html));
            }
            for end in 0..html.len() {
                if let Some(at) = insertion_point(&html[..end]) {
                    assert_eq!(Some(at), whole, "{:?}", String::from_utf8_lossy(&html));
                }
            }
            // Resuming where an earlier, shorter read stopped finds the same place.
            let mut resume = 0;
            let mut resumed = None;
            let mut cuts: Vec<usize> = (0..rng.below(5))
                .map(|_| rng.below(html.len() + 1))
                .collect();
            cuts.sort_unstable();
            for end in cuts.into_iter().chain([html.len()]) {
                match scan(&html[..end], resume) {
                    Ok(at) => {
                        resumed = Some(at);
                        break;
                    }
                    Err(next) => resume = next,
                }
            }
            assert_eq!(resumed, whole, "{:?}", String::from_utf8_lossy(&html));
        });
    }

    /// The search runs again after every read, so its cost must not grow with the square of the
    /// page: a head of many small scripts once took seconds of CPU per load.
    #[test]
    fn a_page_of_many_scripts_is_searched_quickly() {
        let mut page = b"<html><head>".to_vec();
        while page.len() < SCAN_LIMIT - 1000 {
            page.extend_from_slice(b"<script>var a = 1;</script><style>p{}</style>");
        }
        page.extend_from_slice(b"</head><body></body>");
        let started = Instant::now();
        let mut resume = 0;
        let mut found = None;
        for end in (0..page.len()).step_by(16 * 1024).chain([page.len()]) {
            match scan(&page[..end], resume) {
                Ok(at) => {
                    found = Some(at);
                    break;
                }
                Err(next) => resume = next,
            }
        }
        assert_eq!(found, Some(page.len() - 20));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }

    /// Whatever the app answers, the body that reaches the browser is the app's, plus at most
    /// one tag where `scan` says, and only on a whole HTML page.
    #[test]
    fn a_response_only_ever_gains_the_tag() {
        crate::fuzz::rounds(300, |rng| {
            let status = *rng.pick(&[200u16, 204, 206, 301, 304, 404, 500]);
            let mut head = format!("HTTP/1.1 {status} X\r\n");
            let html = rng.below(4) != 0;
            if let Some(kind) = [
                Some("text/html"),
                Some("Text/HTML; charset=utf-8"),
                Some("application/json"),
                None,
            ][if html { rng.below(2) } else { 2 + rng.below(2) }]
            {
                head.push_str(&format!("Content-Type: {kind}\r\n"));
            }
            let encoded = rng.below(6) == 0;
            if encoded {
                head.push_str("Content-Encoding: gzip\r\n");
            }
            let mut body = rng.text(&MARKUP, 60);
            if rng.below(3) == 0 {
                body.extend(std::iter::repeat_n(b'x', rng.below(70_000)));
            }
            let framing = rng.below(3);
            let mut wire = Vec::new();
            match framing {
                0 => head.push_str(&format!("Content-Length: {}\r\n", body.len())),
                1 => {
                    head.push_str("Transfer-Encoding: chunked\r\n");
                    let mut rest = &body[..];
                    while !rest.is_empty() {
                        let size = (1 + rng.below(5000)).min(rest.len());
                        wire.extend_from_slice(format!("{size:x}\r\n").as_bytes());
                        wire.extend_from_slice(&rest[..size]);
                        wire.extend_from_slice(b"\r\n");
                        rest = &rest[size..];
                    }
                    wire.extend_from_slice(b"0\r\n\r\n");
                }
                _ => (),
            }
            if framing != 1 {
                wire = body.clone();
            }
            let mut response = format!("{head}\r\n").into_bytes();
            response.extend_from_slice(&wire);
            let splits: Vec<usize> = (0..rng.below(6))
                .map(|_| rng.below(response.len() + 1))
                .collect();

            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let app = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut from = 0;
                let mut cuts = splits;
                cuts.sort_unstable();
                for cut in cuts.into_iter().chain([response.len()]) {
                    if cut > from {
                        let _ = stream.write_all(&response[from..cut]);
                        let _ = stream.flush();
                        from = cut;
                    }
                }
            });
            let mut output = Vec::new();
            respond(
                BufReader::new(Upstream::new(TcpStream::connect(address).unwrap(), None).unwrap()),
                &mut output,
            )
            .unwrap();
            app.join().unwrap();

            let split = find(&output, b"\r\n\r\n").expect("a head") + 4;
            let (_, got) = output.split_at(split);
            let empty = matches!(status, 204 | 304);
            let page = html && !encoded && !matches!(status, 204 | 206 | 304 | 301);
            let mut expected = if empty { Vec::new() } else { body.clone() };
            let place = insertion_point(&body)
                .or_else(|| (body.contains(&b'<') && !body.contains(&0)).then_some(body.len()));
            if let (true, Some(at)) = (page && !empty, place.filter(|at| *at <= SCAN_LIMIT)) {
                expected.splice(at..at, TAG.bytes());
            }
            assert_eq!(
                String::from_utf8_lossy(got),
                String::from_utf8_lossy(&expected),
                "status {status}, framing {framing}, html {html}, encoded {encoded}"
            );
        });
    }

    #[test]
    fn adds_the_tag_and_drops_what_described_the_old_bytes() {
        let output = through(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 28\r\nETag: \"1\"\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\n\r\n<head></head><body>x</body>\n");
        let (head, body) = output.split_once("\r\n\r\n").unwrap();
        assert_eq!(
            head,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nConnection: close"
        );
        assert_eq!(body, format!("<head>{TAG}</head><body>x</body>\n"));
    }

    #[test]
    fn decodes_chunks_split_anywhere_and_passes_nonces_on() {
        let output = through(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Security-Policy: script-src 'nonce-n1'\r\nTransfer-Encoding: chunked\r\n\r\n8\r\n<head></\r\n5;ext=1\r\nhead>\r\n1\r\nx\r\n0\r\nTrailer: 1\r\n\r\n");
        assert!(
            output.ends_with(
                "\r\n\r\n<head><script defer src=\"/_parcela/widget.js\" nonce=\"n1\"></script></head>x"
            ),
            "{output}"
        );
        assert!(!output.to_ascii_lowercase().contains("transfer-encoding"));
    }

    #[test]
    fn appends_to_markup_without_a_head_and_leaves_plain_text() {
        assert!(
            through(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<p>hello</p>")
                .ends_with(&format!("<p>hello</p>{TAG}"))
        );
        let plain = through(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nETag: \"1\"\r\n\r\nhi");
        assert_eq!(
            plain,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nETag: \"1\"\r\nConnection: close\r\n\r\nhi"
        );
    }

    #[test]
    fn passes_everything_else_through_unchanged() {
        let json = through(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
        );
        assert_eq!(
            json,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
        );
        let redirect = through(b"HTTP/1.1 302 Found\r\nLocation: /a\r\nContent-Type: text/html\r\nContent-Length: 4\r\n\r\n<a/>");
        assert!(redirect.ends_with("\r\n\r\n<a/>"), "{redirect}");
        let empty = through(b"HTTP/1.1 304 Not Modified\r\nETag: \"1\"\r\n\r\n");
        assert_eq!(
            empty,
            "HTTP/1.1 304 Not Modified\r\nETag: \"1\"\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn forwards_early_hints_and_swallows_continue() {
        let output = through(b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 103 Early Hints\r\nLink: </a.css>; rel=preload\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 1\r\n\r\nx");
        assert!(
            output.starts_with(
                "HTTP/1.1 103 Early Hints\r\nLink: </a.css>; rel=preload\r\n\r\nHTTP/1.1 200 OK"
            ),
            "{output}"
        );
    }

    #[test]
    fn streams_the_rest_of_a_page_as_it_arrives() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let app = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<head></head>first")
                .unwrap();
            released.recv().unwrap();
            stream.write_all(b"second").unwrap();
        });
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let injector = thread::spawn(move || {
            respond(
                BufReader::new(Upstream::new(TcpStream::connect(address).unwrap(), None).unwrap()),
                &mut server,
            )
        });
        let mut seen = Vec::new();
        let mut buffer = [0; 1024];
        while !String::from_utf8_lossy(&seen).ends_with("first") {
            let size = client.read(&mut buffer).unwrap();
            assert!(size > 0, "the page stalled before its first part");
            seen.extend_from_slice(&buffer[..size]);
        }
        release.send(()).unwrap();
        client.read_to_end(&mut seen).unwrap();
        injector.join().unwrap().unwrap();
        app.join().unwrap();
        assert!(String::from_utf8_lossy(&seen).ends_with("firstsecond"));
    }
}

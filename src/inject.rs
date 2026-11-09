//! Adds Parcela's badge script to HTML pages. Caddy sends this server only top-level page loads on
//! loopback app names, so assets, API calls, WebSockets, frames and non-browser clients never pass
//! through it. Anything that is not plain HTML goes back unchanged.
use crate::{dashboard, private, sockets};
use anyhow::{Context, Result, ensure};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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

fn accept(listener: &UnixListener, stopped: &AtomicBool) {
    while !stopped.load(Ordering::Relaxed) {
        match listener.accept() {
            // A slow page holds one thread, never another page or the dashboard.
            Ok((stream, _)) => drop(
                thread::Builder::new()
                    .name("parcela-page".into())
                    .spawn(move || drop(serve(stream))),
            ),
            Err(error) => {
                crate::paint::Paint::note(&format!("page injector failed: {error}"));
                break;
            }
        }
    }
}

fn serve(stream: UnixStream) -> Result<()> {
    stream.set_read_timeout(Some(REQUEST_TIME))?;
    let mut client = stream.try_clone()?;
    let mut request = BufReader::new(stream);
    let head = read_head(&mut request)?;
    let Some(forward) = forwarded_request(&head) else {
        return reply(&mut client, "400 Bad Request");
    };
    let Ok(upstream) = TcpStream::connect_timeout(&forward.upstream, CONNECT_TIME) else {
        return reply(&mut client, "502 Bad Gateway");
    };
    (&upstream).write_all(&forward.bytes)?;
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
        reader.read_until(b'\n', &mut line)?;
        ensure!(line.ends_with(b"\n"), "incomplete HTTP head");
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

fn respond<W: Write>(mut upstream: BufReader<TcpStream>, client: &mut W) -> Result<()> {
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
    let mut page = Vec::new();
    let mut buffer = [0; 16 * 1024];
    let position = loop {
        if let Some(position) = insertion_point(&page) {
            break Some(position);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || page.len() >= SCAN_LIMIT || body.set_timeout(remaining).is_err() {
            break None;
        }
        match body.read(&mut buffer) {
            Ok(0) => break page.contains(&b'<').then_some(page.len()),
            Ok(size) => page.extend_from_slice(&buffer[..size]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
            Err(_) => break None,
        }
    };
    let _ = body.clear_timeout();
    (page, position)
}

/// The offset of `</head>`, or of `<body` for a page without one. Comments, quoted attribute values
/// and raw-text elements like `<script>` do not count, so a tag never lands inside them.
fn insertion_point(html: &[u8]) -> Option<usize> {
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
    let mut at = 0;
    loop {
        let start = at + html.get(at..)?.iter().position(|byte| *byte == b'<')?;
        let rest = &html[start..];
        if rest.starts_with(b"<!--") {
            at = start + 4 + find(&rest[4..], b"-->")? + 3;
            continue;
        }
        if matches!(rest.get(1)?, b'!' | b'?') {
            at = start + find(rest, b">")? + 1;
            continue;
        }
        let closing = rest[1] == b'/';
        let name_start = start + 1 + usize::from(closing);
        if !html.get(name_start)?.is_ascii_alphabetic() {
            at = start + 1;
            continue;
        }
        let name_length = html[name_start..]
            .iter()
            .position(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b':'))?;
        let name = html[name_start..name_start + name_length].to_ascii_lowercase();
        let end = name_start + name_length + tag_end(&html[name_start + name_length..])?;
        match (closing, name.as_slice()) {
            (true, b"head") | (false, b"body") => return Some(start),
            (false, name) if RAW_TEXT.contains(&name) => {
                let mut closer = b"</".to_vec();
                closer.extend_from_slice(name);
                let lowered = html[end + 1..].to_ascii_lowercase();
                at = end + 1 + find(&lowered, &closer)?;
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
    reader: BufReader<TcpStream>,
    framing: Framing,
}
impl Body {
    fn new(head: &[Vec<u8>], status: u16, reader: BufReader<TcpStream>) -> Self {
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

    fn set_timeout(&self, timeout: Duration) -> io::Result<()> {
        self.reader.get_ref().set_read_timeout(Some(timeout))
    }

    fn clear_timeout(&self) -> io::Result<()> {
        self.reader.get_ref().set_read_timeout(None)
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
                    self.reader.read_until(b'\n', line)?;
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
            BufReader::new(TcpStream::connect(address).unwrap()),
            &mut output,
        )
        .unwrap();
        app.join().unwrap();
        String::from_utf8(output).unwrap()
    }

    const TAG: &str = "<script defer src=\"/_parcela/widget.js\"></script>";

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
                BufReader::new(TcpStream::connect(address).unwrap()),
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

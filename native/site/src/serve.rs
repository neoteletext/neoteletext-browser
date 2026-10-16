//! Serves a channel folder over HTTP/1.1, as `docs/channels.md` section 5.3
//! of the engine asks of every server: 404 for a missing file, the
//! project's media types, no query strings. Its push address carries
//! server-sent events (the engine's REQ-WEB-005): every few seconds one of
//! the live pages changes, and an event named `page` sends the new version
//! as a TTI page in its data lines.
//!
//! Each live page rotates through the versions the builder kept, and is
//! stamped on row 23 with the time it changed, so every update is visible.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The longest request head read.
const MAX_HEAD: usize = 8 * 1024;
/// One live page changes this often.
const UPDATE_EVERY: Duration = Duration::from_millis(1500);
const SECONDS_PER_DAY: u64 = 86_400;
const SECONDS_PER_HOUR: u64 = 3600;
const SECONDS_PER_MINUTE: u64 = 60;
/// The row a live page's time of change is written on.
const STAMP_ROW: &str = "OL,23,";
const ESCAPE: char = '\x1b';

fn media_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("tti") => "text/x-teletext-tti",
        Some("t42") => "application/x-teletext-t42",
        Some("json") => "application/json",
        _ => "text/plain",
    }
}

/// The time of day in UTC, as HH:MM:SS.
fn clock() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs()) % SECONDS_PER_DAY;
    format!("{:02}:{:02}:{:02}", seconds / SECONDS_PER_HOUR, seconds % SECONDS_PER_HOUR / SECONDS_PER_MINUTE, seconds % SECONDS_PER_MINUTE)
}

/// A live page's next version: its rows, with row 23 replaced by a
/// stamp saying it is live and when it changed.
fn updated(version: &str) -> String {
    let mut lines: Vec<String> = version.lines().filter(|line| !line.starts_with(STAMP_ROW)).map(str::to_owned).collect();
    lines.push(format!("{STAMP_ROW}{ESCAPE}A{ESCAPE}]{ESCAPE}G LIVE {ESCAPE}\\{ESCAPE}C UPDATED {} UTC", clock()));
    lines.join("\r\n") + "\r\n"
}

struct Live {
    /// Each live page's versions, read from `live/NNN-k.tti`.
    pages: Vec<Vec<String>>,
    turn: usize,
    event: u64,
}

impl Live {
    fn read(folder: &Path) -> Self {
        let mut files: Vec<PathBuf> =
            fs::read_dir(folder.join("live")).map(|entries| entries.flatten().map(|entry| entry.path()).collect()).unwrap_or_default();
        files.sort();
        let mut pages: Vec<Vec<String>> = Vec::new();
        let mut last = String::new();
        for file in files {
            let name = file.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_owned();
            let number = name.split('-').next().unwrap_or_default().to_owned();
            let Ok(text) = fs::read_to_string(&file) else { continue };
            if number != last || pages.is_empty() {
                pages.push(Vec::new());
                last = number;
            }
            if let Some(versions) = pages.last_mut() {
                versions.push(text);
            }
        }
        Self { pages, turn: 0, event: 0 }
    }

    /// The next update: a live page, in turn, at its next version.
    fn next(&mut self) -> Option<(u64, String)> {
        if self.pages.is_empty() {
            return None;
        }
        let page = self.turn % self.pages.len();
        let versions = &self.pages[page];
        let version = (self.turn / self.pages.len()) % versions.len().max(1);
        self.turn += 1;
        self.event += 1;
        Some((self.event, updated(versions.get(version)?)))
    }
}

fn respond(stream: &mut TcpStream, status: &str, media: &str, body: &[u8]) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {media}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\r\n", body.len())?;
    stream.write_all(body)?;
    stream.flush()
}

/// A push connection: one event per update, for as long as it stays open.
fn push(mut stream: TcpStream, updates: &Arc<Mutex<Vec<(u64, String)>>>) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\r\n")?;
    stream.flush()?;
    let mut sent = updates.lock().map_or(0, |list| list.last().map_or(0, |(event, _)| *event));
    loop {
        thread::sleep(UPDATE_EVERY / 4);
        let pending: Vec<(u64, String)> = updates.lock().map(|list| list.iter().filter(|(event, _)| *event > sent).cloned().collect()).unwrap_or_default();
        for (event, page) in pending {
            let mut data = String::new();
            for line in page.lines().filter(|line| !line.is_empty()) {
                data.push_str("data: ");
                data.push_str(line);
                data.push('\n');
            }
            write!(stream, "event: page\nid: {event}\n{data}\n")?;
            stream.flush()?;
            sent = event;
        }
    }
}

fn serve(mut stream: TcpStream, root: &Path, push_path: &str, updates: &Arc<Mutex<Vec<(u64, String)>>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    loop {
        let mut request = String::new();
        let mut head = 0;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            head += line.len();
            if head > MAX_HEAD {
                return respond(&mut stream, "431 Request Header Fields Too Large", "text/plain", b"");
            }
            if request.is_empty() {
                line.trim_end().clone_into(&mut request);
            } else if line == "\r\n" || line == "\n" {
                break;
            }
        }
        let mut parts = request.split(' ');
        let (method, target) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
        let path = target.split('?').next().unwrap_or_default();
        eprintln!("t42-site: {method} {target}");
        if method != "GET" {
            respond(&mut stream, "405 Method Not Allowed", "text/plain", b"")?;
            continue;
        }
        if path == push_path {
            return push(stream, updates);
        }
        // Only plain names under the root: no "..", no hidden files.
        let relative = path.trim_start_matches('/');
        let safe = relative.split('/').all(|part| !part.is_empty() && !part.starts_with('.'));
        match fs::read(root.join(relative)).ok().filter(|_| safe) {
            Some(body) => respond(&mut stream, "200 OK", media_type(Path::new(relative)), &body)?,
            None => respond(&mut stream, "404 Not Found", "text/plain", b"not found")?,
        }
    }
}

/// Serves `root` on `port`; the channel folder is `root/<channel>/`, and
/// its push feed is at `push_path`.
///
/// # Errors
/// When the port cannot be listened on.
pub fn run(root: &Path, channel: &str, port: u16) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|error| format!("port {port}: {error}"))?;
    let push_path = format!("/{channel}/push");
    let updates: Arc<Mutex<Vec<(u64, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let mut live = Live::read(&root.join(channel));
    eprintln!("t42-site: serving http://127.0.0.1:{port}/{channel}/ with {} live pages", live.pages.len());
    let feed = updates.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(UPDATE_EVERY);
            if let Some(update) = live.next()
                && let Ok(mut list) = feed.lock()
            {
                list.push(update);
                // Only the latest updates are kept for new connections.
                let excess = list.len().saturating_sub(64);
                list.drain(..excess);
            }
        }
    });
    for stream in listener.incoming().flatten() {
        let root = root.to_owned();
        let push_path = push_path.clone();
        let updates = updates.clone();
        thread::spawn(move || {
            let _ = serve(stream, &root, &push_path, &updates);
        });
    }
    Ok(())
}

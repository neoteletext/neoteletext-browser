//! A fetcher against a real server of files, confined as the broker
//! confines it: a static channel with no bundle and no contents file gives
//! its pages when they are asked for (REQ-703), and what a server refuses
//! reaches the trusted core as a refusal stating its status, whatever came
//! back (REQ-708).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::Path;

use confine::{Role, Spawn};
use protocol::fetcher::{FromFetcher, Origin, RefusedFile, ToFetcher, read_from_fetcher, write_to_fetcher};

/// A page file, of one page.
const PAGE: &[u8] = b"PN,10000\r\nOL,1,HELLO\r\n";
/// What a web server's error answers with.
const MARKUP: &[u8] = b"<!doctype html><title>Not teletext</title>";
/// The start of an image.
const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

/// One file: status, media type and body.
type File = (u16, &'static str, &'static [u8]);

/// Serves `files` by path, and 404 for anything else, one connection a
/// thread, until the test ends.
fn serve(files: BTreeMap<&'static str, File>) -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for connection in listener.incoming().flatten() {
            let files = files.clone();
            std::thread::spawn(move || answer(connection, &files));
        }
    });
    port
}

fn answer(connection: TcpStream, files: &BTreeMap<&'static str, File>) {
    let mut reader = BufReader::new(connection.try_clone().unwrap());
    let mut writer = connection;
    loop {
        let mut first = String::new();
        if reader.read_line(&mut first).unwrap_or(0) == 0 {
            return;
        }
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap_or(0) == 0 {
                return;
            }
            if header.trim().is_empty() {
                break;
            }
        }
        let path = first.split(' ').nth(1).unwrap_or_default().to_owned();
        let (status, media, body) = files.get(path.as_str()).copied().unwrap_or((404, "text/plain", b""));
        let head = format!("HTTP/1.1 {status} S\r\nContent-Type: {media}\r\nContent-Length: {}\r\n\r\n", body.len());
        if writer.write_all(head.as_bytes()).and_then(|()| writer.write_all(body)).is_err() {
            return;
        }
    }
}

/// A fetcher of the channel at `port`, as the trusted core starts one that
/// may reach this computer.
struct Fetcher {
    process: confine::Process,
    output: BufReader<Box<dyn Read>>,
}

impl Fetcher {
    fn start(port: u16) -> Self {
        let program = Path::new(env!("CARGO_BIN_EXE_neoteletext-fetcher"));
        let arguments = [OsString::from(format!("http://127.0.0.1:{port}/")), OsString::from("--allow-local"), OsString::from("127.0.0.1")];
        let environment = confine::environment(Role::Network);
        let mut process =
            confine::spawn(&Spawn { program, arguments: &arguments, role: Role::Network, environment: &environment, local_network: true, writable: None })
                .unwrap();
        let output: Box<dyn Read> = Box::new(process.stdout.take().unwrap());
        let output = BufReader::new(output);
        Self { process, output }
    }

    /// Reads until `wanted` picks something out of a message. The cookie
    /// questions the agent asks are answered with no cookies.
    fn until<T>(&mut self, mut wanted: impl FnMut(&FromFetcher) -> Option<T>) -> T {
        loop {
            let message = read_from_fetcher(&mut self.output).unwrap().expect("the fetcher ended");
            match &message {
                FromFetcher::CookieQuestion { id, .. } => self.tell(&ToFetcher::CookieAnswer { id: *id, header: Vec::new() }),
                FromFetcher::ReachedQuestion { id, .. } => self.tell(&ToFetcher::ReachedAnswer { id: *id, reached: false }),
                _ => {}
            }
            if let Some(found) = wanted(&message) {
                return found;
            }
        }
    }

    fn tell(&mut self, message: &ToFetcher) {
        let input = self.process.stdin.as_mut().unwrap();
        write_to_fetcher(input, message).unwrap();
        input.flush().unwrap();
    }

    /// The refusal of `file` it says, with its status.
    fn refusal(&mut self, file: RefusedFile) -> Option<u16> {
        self.until(|message| match message {
            FromFetcher::Refused { file: refused, status } if *refused == file => Some(*status),
            _ => None,
        })
    }
}

impl Drop for Fetcher {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// REQ-703: no `channel.ttx`, no bundle, no `contents.txt`, and one page
/// file: the fetcher says there is no contents file, and gives the page when
/// the trusted core asks for it, and a page that is not there is a 404's
/// refusal. On the code before, nothing could ask for a page.
#[test]
fn a_channel_of_one_page_file_gives_it_when_asked() {
    let port = serve(BTreeMap::from([("/100.tti", (200, "text/plain", PAGE))]));
    let mut fetcher = Fetcher::start(port);
    fetcher.until(|message| matches!(message, FromFetcher::NoContents).then_some(()));
    fetcher.tell(&ToFetcher::FetchPage { page: 0x100 });
    let bytes = fetcher.until(|message| match message {
        FromFetcher::Page { origin: Origin::FetchedPage, bytes } => Some(bytes.clone()),
        _ => None,
    });
    assert_eq!(bytes, PAGE);
    fetcher.tell(&ToFetcher::FetchPage { page: 0x200 });
    assert_eq!(fetcher.refusal(RefusedFile::Page(0x200)), Some(404));
}

/// REQ-704: a `channel.ttx` with `refresh` says so, in the bounds it was read
/// to; `refresh` of a channel with a push connection says nothing, which the
/// agent's descriptor gives the fetcher.
#[test]
fn a_declared_refresh_reaches_the_trusted_core() {
    let port = serve(BTreeMap::from([("/channel.ttx", (200, "application/json", &b"{\"refresh\":7}"[..]))]));
    let mut fetcher = Fetcher::start(port);
    let seconds = fetcher.until(|message| if let FromFetcher::Refresh(seconds) = message { Some(*seconds) } else { None });
    assert_eq!(seconds, 7);
}

/// REQ-708: markup, an image and random bytes as a page file, under the
/// statuses 200, 403 and 500, produce no page and a refusal stating that
/// status. Each failed on the code before, where a failed page ended the
/// wait for the next and nothing more.
#[test]
fn what_is_not_teletext_as_a_page_is_refused_with_its_status() {
    const RANDOM: &[u8] = &[0x93, 0x11, 0xFE, 0x02, 0x77, 0xC4, 0x5A, 0x08];
    for body in [MARKUP, IMAGE, RANDOM] {
        for status in [200, 403, 500] {
            let media = if body == MARKUP { "text/html" } else { "application/octet-stream" };
            let port = serve(BTreeMap::from([("/100.tti", (status, media, body)), ("/100.t42", (status, media, body))]));
            let mut fetcher = Fetcher::start(port);
            fetcher.until(|message| matches!(message, FromFetcher::NoContents).then_some(()));
            fetcher.tell(&ToFetcher::FetchPage { page: 0x100 });
            let found = fetcher.until(|message| match message {
                FromFetcher::Refused { file: RefusedFile::Page(0x100), status } => Some(Ok(*status)),
                FromFetcher::Page { .. } => Some(Err("a page was produced")),
                _ => None,
            });
            assert_eq!(found, Ok(Some(status)), "{body:?} under {status}");
        }
    }
}

/// REQ-708: the same as `channel.ttx`.
#[test]
fn what_is_not_a_channel_is_refused_with_its_status() {
    for (status, media, body, said) in [
        (500, "text/html", MARKUP, Some(500)),
        (403, "application/json", &b"{}"[..], Some(403)),
        (200, "text/html", MARKUP, Some(200)),
        (200, "application/octet-stream", IMAGE, Some(200)),
    ] {
        let port = serve(BTreeMap::from([("/channel.ttx", (status, media, body))]));
        assert_eq!(Fetcher::start(port).refusal(RefusedFile::Channel), said, "{body:?} under {status}");
    }
}

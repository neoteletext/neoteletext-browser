//! A fetcher against a real server of files, confined as the broker
//! confines it: a static channel with no bundle and no contents file gives
//! its pages when they are asked for (REQ-703), and what a server refuses
//! reaches the trusted core as a refusal stating its status, whatever came
//! back (REQ-708).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

use confine::{Role, Spawn};
use protocol::fetcher::{FromFetcher, Origin, RefusedFile, ToFetcher, read_from_fetcher, write_to_fetcher};

/// A page file, of one page.
const PAGE: &[u8] = b"PN,10000\r\nOL,1,HELLO\r\n";
/// What a web server's error answers with.
const MARKUP: &[u8] = b"<!doctype html><title>Not teletext</title>";
/// The start of an image.
const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

/// Bytes that are nothing.
const RANDOM: &[u8] = &[0x93, 0x11, 0xFE, 0x02, 0x77, 0xC4, 0x5A, 0x08];

/// One file: status, media type and body.
type File = (u16, &'static str, &'static [u8]);

/// The paths asked for, in order.
type Asked = Arc<Mutex<Vec<String>>>;

/// Serves `files` by path, and 404 for anything else, one connection a
/// thread, until the test ends.
fn serve(files: BTreeMap<&'static str, File>) -> u16 {
    serve_logged(files).0
}

/// [`serve`], saying which paths were asked for.
fn serve_logged(files: BTreeMap<&'static str, File>) -> (u16, Asked) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked = Asked::default();
    let log = Arc::clone(&asked);
    std::thread::spawn(move || {
        for connection in listener.incoming().flatten() {
            let (files, log) = (files.clone(), Arc::clone(&log));
            std::thread::spawn(move || answer(connection, &files, &log));
        }
    });
    (port, asked)
}

fn answer(connection: TcpStream, files: &BTreeMap<&'static str, File>, asked: &Asked) {
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
        asked.lock().unwrap().push(path.clone());
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
    for body in [MARKUP, IMAGE, RANDOM] {
        for status in [200, 403, 500] {
            let media = if body == MARKUP { "text/html" } else { "application/octet-stream" };
            let port = serve(BTreeMap::from([("/channel.ttx", (status, media, body))]));
            assert_eq!(Fetcher::start(port).refusal(RefusedFile::Channel), Some(status), "{body:?} under {status}");
        }
    }
}

/// Part `part` of the list of page 210 of `total` sub-pages, sub-codes 0001
/// up, as a generator writes it (DOC-1869).
fn list_part(total: u16, part: u16) -> &'static [u8] {
    let mut file = format!("{total}\tv1\r\n");
    for subcode in (part * 64 + 1..=(part * 64 + 64).min(total)).map(|place| format!("{place:04X}")) {
        let _ = write!(file, "{subcode}\t{}\tStory {subcode}\r\n", subcode.to_lowercase());
    }
    file.leak().as_bytes()
}

/// DEC-2116, REQ-1887 and REQ-1888: a page's sub-pages in its folder. The
/// fetcher asks for the parts of its list the trusted core names,
/// `210/0.sub` and `210/1.sub`, each one message at its part's first place
/// with the page's count and version; a part not there, and part 0 of a
/// page with no folder, are said to be missing; a sub-page is fetched from
/// its own file, `210/0002.t42` and then `210/0002.tti`; and nothing else
/// is asked for. On the code before, the fetcher asked for `210.sub`.
#[test]
fn a_pages_sub_pages_come_from_its_folder() {
    const SUB_PAGE: &[u8] = b"PN,21002\r\nSC,0002\r\nOL,1,SECOND\r\n";
    let (port, asked) = serve_logged(BTreeMap::from([
        ("/210/0.sub", (200, "text/plain", list_part(70, 0))),
        ("/210/1.sub", (200, "text/plain", list_part(70, 1))),
        ("/210/0002.tti", (200, "text/plain", SUB_PAGE)),
    ]));
    let mut fetcher = Fetcher::start(port);
    fetcher.until(|message| matches!(message, FromFetcher::NoContents).then_some(()));
    let ask = |fetcher: &mut Fetcher, part: u8| {
        fetcher.tell(&ToFetcher::FetchSubPages { page: 0x210, part });
        fetcher.until(|message| match message {
            FromFetcher::SubPages { page: 0x210, version, part } => Some(Ok((version.clone(), part.count, part.first, part.rows.len()))),
            FromFetcher::SubPagesMissing { page: 0x210, part } => Some(Err(*part)),
            _ => None,
        })
    };
    assert_eq!(ask(&mut fetcher, 0), Ok((String::from("v1"), 70, 0, 64)));
    assert_eq!(ask(&mut fetcher, 1), Ok((String::from("v1"), 70, 64, 6)));
    assert_eq!(ask(&mut fetcher, 2), Err(2), "a part not there");
    fetcher.tell(&ToFetcher::FetchSubPage { page: 0x210, subcode: 2 });
    let bytes = fetcher.until(|message| match message {
        FromFetcher::Page { origin: Origin::FetchedPage, bytes } => Some(bytes.clone()),
        _ => None,
    });
    assert_eq!(bytes, SUB_PAGE);
    fetcher.tell(&ToFetcher::FetchSubPages { page: 0x300, part: 0 });
    let missing = fetcher.until(|message| match message {
        FromFetcher::SubPagesMissing { page: 0x300, part } => Some(*part),
        _ => None,
    });
    assert_eq!(missing, 0, "a page with no folder");
    let asked: Vec<String> = asked.lock().unwrap().iter().filter(|path| path.starts_with("/210") || path.starts_with("/300")).cloned().collect();
    assert_eq!(asked, ["/210/0.sub", "/210/1.sub", "/210/2.sub", "/210/0002.t42", "/210/0002.tti", "/300/0.sub"]);
}

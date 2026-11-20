//! The tabs of a fetcher process against a server played on this
//! computer: each tab's messages named with its tab, a message for a tab
//! not held dropped, and the breaks of the pipe's protocol that end the
//! process (REQ-6980).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "a test reports a failure by panicking, so it uses `unwrap` on results it expects to be good, `expect` naming what it expected, indexing into data it built and `panic!` on outcomes it rules out"
)]

use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::sync::mpsc;
use std::time::Duration;

use protocol::fetcher::{RefusedFile, read_from_fetcher_for, write_to_fetcher_for};

use super::*;
use crate::process::agent_config;

/// How long a test waits for what should come at once.
const LIMIT: Duration = Duration::from_secs(20);
/// The status the played server answers every request with.
const NOT_FOUND: u16 = 404;

/// The trusted core's side of a pipe, read as a blocking stream.
struct PipeReader {
    chunks: mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
}

impl Read for PipeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.pending.is_empty() {
            match self.chunks.recv_timeout(LIMIT) {
                Ok(chunk) => self.pending = chunk,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
                Err(mpsc::RecvTimeoutError::Timeout) => return Err(io::ErrorKind::TimedOut.into()),
            }
        }
        let count = buffer.len().min(self.pending.len());
        buffer[..count].copy_from_slice(&self.pending[..count]);
        self.pending.drain(..count);
        Ok(count)
    }
}

/// A pipe's writing side.
struct PipeWriter(mpsc::Sender<Vec<u8>>);

impl Write for PipeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.send(bytes.to_vec()).map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn pipe() -> (PipeWriter, PipeReader) {
    let (sender, chunks) = mpsc::channel();
    (PipeWriter(sender), PipeReader { chunks, pending: Vec::new() })
}

/// A server on this computer that answers every request 404 and closes,
/// and its port: a channel with no files, which each tab's own agent opens
/// and reports for itself.
fn refusing_server() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { return };
            std::thread::spawn(move || {
                let mut lines = BufReader::new(socket.try_clone().unwrap());
                let mut line = String::new();
                while lines.read_line(&mut line).is_ok_and(|read| read > 0) && line != "\r\n" {
                    line.clear();
                }
                let _ = write!(socket, "HTTP/1.1 {NOT_FOUND} Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            });
        }
    });
    port
}

/// A fetcher process's tabs for the channel at `port` of this computer,
/// served on a thread: the pipe to write the trusted core's messages on,
/// the pipe its own are read from, and how `serve` ended.
fn served(port: u16) -> (PipeWriter, PipeReader, mpsc::Receiver<Result<(), String>>) {
    let (to_fetcher, mut input) = pipe();
    let (output, from_fetcher) = pipe();
    let (ended, end) = mpsc::channel();
    std::thread::spawn(move || {
        let local = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let config = agent_config(&[local], false);
        let allowed = config.allowed_local;
        let transport: MakeTransport = Arc::new(move |wake| Tcp::new(wake, None, allowed));
        let _ = ended.send(serve(&format!("http://127.0.0.1:{port}/"), config, &transport, &mut input, Box::new(output)));
    });
    (to_fetcher, from_fetcher, end)
}

fn tell(pipe: &mut PipeWriter, tab: u32, message: &ToFetcher) {
    write_to_fetcher_for(pipe, tab, message).unwrap();
}

/// Reads until `tab` says its channel's bundle settled, the last thing a
/// static channel's opening says; the tabs every message read named, that
/// one's among them.
fn until_settled(from_fetcher: &mut PipeReader, tab: u32) -> Vec<u32> {
    let mut named = Vec::new();
    loop {
        let (said, message) = read_from_fetcher_for(from_fetcher).unwrap().expect("the fetcher ended");
        named.push(said);
        if said == tab && message == FromFetcher::BundleSettled {
            return named;
        }
    }
}

/// REQ-6980: the process says it is confined once, as itself; each tab
/// the trusted core adds opens the channel for itself and its messages
/// carry its number; a message for a tab not held is dropped; a tab that
/// leaves ends alone; and the pipe's end ends the rest cleanly. Before the
/// requirement a fetcher served one tab and no message named one.
#[test]
fn each_tab_is_served_for_itself_and_named() {
    let (mut to_fetcher, mut from_fetcher, end) = served(refusing_server());
    assert_eq!(read_from_fetcher_for(&mut from_fetcher).unwrap(), Some((PROCESS_TAB, FromFetcher::Confined)), "first, and the process's own");
    // A message for a tab never opened, and one to leave: both dropped.
    tell(&mut to_fetcher, 9, &ToFetcher::FetchPage { page: 0x100 });
    tell(&mut to_fetcher, 9, &ToFetcher::TabLeaves);
    tell(&mut to_fetcher, 3, &ToFetcher::TabOpens);
    assert!(until_settled(&mut from_fetcher, 3).iter().all(|&tab| tab == 3), "only the tab opened speaks");
    tell(&mut to_fetcher, 4, &ToFetcher::TabOpens);
    assert!(until_settled(&mut from_fetcher, 4).iter().all(|&tab| tab == 3 || tab == 4));
    // Tab 3 leaves; tab 4 is still served, and tab 3's number is free of
    // messages: one for it is dropped.
    tell(&mut to_fetcher, 3, &ToFetcher::TabLeaves);
    tell(&mut to_fetcher, 3, &ToFetcher::FetchPage { page: 0x100 });
    tell(&mut to_fetcher, 4, &ToFetcher::FetchPage { page: 0x100 });
    let refused = FromFetcher::Refused { file: RefusedFile::Page(0x100), status: Some(NOT_FOUND) };
    loop {
        let (tab, message) = read_from_fetcher_for(&mut from_fetcher).unwrap().expect("the fetcher ended");
        assert_ne!((tab, &message), (3, &refused), "a tab that left is asked nothing");
        if (tab, &message) == (4, &refused) {
            break;
        }
    }
    drop(to_fetcher);
    assert_eq!(end.recv_timeout(LIMIT).unwrap(), Ok(()), "the pipe's end is a clean end");
}

/// REQ-6980: a tab opened under a number the process holds, or under the
/// process's own, breaks the pipe's protocol and ends the process.
#[test]
fn a_tab_number_taken_ends_the_process() {
    let (mut to_fetcher, _from_fetcher, end) = served(refusing_server());
    tell(&mut to_fetcher, 3, &ToFetcher::TabOpens);
    tell(&mut to_fetcher, 3, &ToFetcher::TabOpens);
    assert!(end.recv_timeout(LIMIT).unwrap().is_err(), "a second tab of one number");
    let (mut to_fetcher, _from_fetcher, end) = served(refusing_server());
    tell(&mut to_fetcher, PROCESS_TAB, &ToFetcher::TabOpens);
    assert!(end.recv_timeout(LIMIT).unwrap().is_err(), "the process's own number is no tab's");
}

/// REQ-6980: a process holds at most `MAX_FETCHER_TABS` tabs; one more
/// ends it. A tab that left makes room.
#[test]
fn one_tab_past_the_bound_ends_the_process() {
    let (mut to_fetcher, _from_fetcher, end) = served(refusing_server());
    let most = u32::try_from(MAX_FETCHER_TABS).unwrap();
    for tab in 1..=most {
        tell(&mut to_fetcher, tab, &ToFetcher::TabOpens);
    }
    tell(&mut to_fetcher, 1, &ToFetcher::TabLeaves);
    tell(&mut to_fetcher, most + 1, &ToFetcher::TabOpens);
    assert_eq!(end.recv_timeout(Duration::from_millis(300)), Err(mpsc::RecvTimeoutError::Timeout), "at the bound it serves");
    tell(&mut to_fetcher, most + 2, &ToFetcher::TabOpens);
    assert!(end.recv_timeout(LIMIT).unwrap().is_err(), "past it, it ends");
}

/// REQ-6980: a message without its tab number is not a message, and ends
/// the process as any break of the protocol does.
#[test]
fn a_message_without_its_tab_ends_the_process() {
    let (mut to_fetcher, _from_fetcher, end) = served(refusing_server());
    protocol::fetcher::write_to_fetcher(&mut to_fetcher, &ToFetcher::TabOpens).unwrap();
    protocol::fetcher::write_to_fetcher(&mut to_fetcher, &ToFetcher::TabOpens).unwrap();
    assert!(end.recv_timeout(LIMIT).unwrap().is_err());
}

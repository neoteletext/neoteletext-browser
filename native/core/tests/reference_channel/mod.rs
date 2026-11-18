//! What the tests against the reference channel share: `neoteletext-site
//! terminal` and the real fetcher, built for the test's profile and started
//! as the trusted core starts them, with a session opened as it opens one.

#![allow(dead_code, reason = "each test uses part of this")]

use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use neoteletext_input::caps::SELECT_CAP;
use neoteletext_input::control::{Control, FRAGMENT_DATA, from_packet};
use neoteletext_input::event::Item;
use neoteletext_input::session::Session;
use protocol::fetcher::{FromFetcher, Origin, ToFetcher, read_from_fetcher, write_to_fetcher};

mod site;
#[allow(unused_imports, reason = "each test uses part of this")]
pub use site::{Killed, Site, build, cargo, said_until};

/// The channel and the fetcher, built for this test's profile: a test
/// builds only its own package's programs.
pub fn programs() -> (PathBuf, PathBuf) {
    let directory = build(&["neoteletext-site", "neoteletext-fetcher"], None);
    let path = |name: &str| directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    (path("neoteletext-site"), path("neoteletext-fetcher"))
}

/// The fetcher's input, written by the test and by the thread that answers
/// its cookie questions as the trusted core does, one message at a time.
pub type Input = Arc<Mutex<Option<BufWriter<ChildStdin>>>>;

/// An input message alone: not a move, nothing more of its moment.
#[must_use]
pub fn alone(bytes: Vec<u8>) -> ToFetcher {
    ToFetcher::Send { bytes, move_of: None, more: false }
}

pub fn send(input: &Input, message: &ToFetcher) {
    let mut input = input.lock().unwrap();
    let input = input.as_mut().expect("the fetcher's input is open");
    write_to_fetcher(&mut *input, message).unwrap();
    input.flush().unwrap();
}

/// `item` as the receiver's session encodes it.
pub fn message(session: &mut Session, item: &Item<'_>) -> Vec<u8> {
    let mut out = [0; neoteletext_input::MAX_MESSAGE];
    let length = session.send(item, 0, &mut out).unwrap();
    assert!(length > 0, "{item:?} went");
    out[..length].to_vec()
}

/// Whether pushed packets hold a page header, packet 0.
#[must_use]
pub fn has_header(bytes: &[u8]) -> bool {
    bytes
        .as_chunks::<42>()
        .0
        .iter()
        .any(|bytes| neoteletext_core::packet::decode_address(&neoteletext_core::packet::Packet { bytes: *bytes }).is_ok_and(|address| address.number == 0))
}

/// A tab's fetcher on the reference channel, with what it says.
pub struct Tab {
    input: Input,
    pub received: mpsc::Receiver<(Instant, FromFetcher)>,
    process: Killed,
}

impl Tab {
    /// Starts a fetcher for the channel on `port`.
    pub fn open(program: &Path, port: u16) -> Self {
        Self::open_at(program, &format!("http://127.0.0.1:{port}/terminal/"))
    }

    /// Starts a fetcher for `channel`, an address on this computer, as the
    /// trusted core starts one: with the address, query and all.
    pub fn open_at(program: &Path, channel: &str) -> Self {
        let mut fetcher = Command::new(program).args([channel, "--allow-local", "127.0.0.1"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let input: Input = Arc::new(Mutex::new(Some(BufWriter::new(fetcher.stdin.take().unwrap()))));
        let output = fetcher.stdout.take().unwrap();
        let (sender, received) = mpsc::channel();
        let answering = Arc::clone(&input);
        std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            while let Ok(Some(message)) = read_from_fetcher(&mut output) {
                // The reference channel asks no state: a challenge, were one
                // to come, is answered as a stateless client answers
                // (VS-204).
                match message {
                    FromFetcher::StateChallenged { request, .. } => {
                        send(
                            &answering,
                            &ToFetcher::StateAnswer { request, reply: protocol::fetcher::StateReply::None(protocol::fetcher::NoneReason::Stateless) },
                        );
                    }
                    message => {
                        if sender.send((Instant::now(), message)).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Self { input, received, process: Killed::new(fetcher) }
    }

    /// The fetcher's input.
    #[must_use]
    pub fn input(&self) -> Input {
        Arc::clone(&self.input)
    }

    /// Closes the fetcher's input, as `Fetcher::stop` does.
    pub fn close_input(&self) {
        *self.input.lock().unwrap() = None;
    }

    /// Writes `message` and closes the input in one go, so that the fetcher
    /// reads the two together, as it does for a tab's last SELECT.
    pub fn send_and_close(&self, message: &ToFetcher) {
        let mut input = self.input.lock().unwrap();
        write_to_fetcher(input.as_mut().expect("the fetcher's input is open"), message).unwrap();
        *input = None;
    }

    /// Ends the fetcher now, as a fault would.
    pub fn kill(&mut self) {
        self.process.kill();
    }

    /// Waits for the push connection, then HELLO and the channel's WELCOME
    /// in pushed packets (DOC-1921): the session input then
    /// goes in.
    pub fn session(&self, deadline: Instant) -> Session {
        loop {
            match self.received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok((_, FromFetcher::Input { open: true, .. })) => break,
                Ok(_) => {}
                Err(error) => panic!("the channel's input never opened: {error}"),
            }
        }
        let mut session = Session::new(SELECT_CAP);
        let mut hello = [0; neoteletext_input::MAX_MESSAGE];
        let length = session.open(0, None, &mut hello).unwrap();
        send(&self.input, &alone(hello[..length].to_vec()));
        loop {
            match self.received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok((_, FromFetcher::Page { origin: Origin::PushedPackets, bytes })) => {
                    let mut buffer = [0; FRAGMENT_DATA];
                    let welcome = bytes.as_chunks::<42>().0.iter().find_map(|bytes| {
                        match from_packet(&neoteletext_core::packet::Packet { bytes: *bytes }, &mut buffer) {
                            Some(Ok(Control::Welcome { version, caps, resume })) => Some((version, caps, resume)),
                            _ => None,
                        }
                    });
                    if let Some((version, caps, resume)) = welcome {
                        session.welcome(version, caps, resume).unwrap();
                        return session;
                    }
                }
                Ok(_) => {}
                Err(error) => panic!("no WELCOME: {error}"),
            }
        }
    }

    /// The next pushed page, whole T42 packets with a page header.
    pub fn next_page(&self, deadline: Instant) -> Vec<u8> {
        loop {
            match self.received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok((_, FromFetcher::Page { origin: Origin::PushedPackets, bytes })) if has_header(&bytes) => return bytes,
                Ok(_) => {}
                Err(error) => panic!("no page in time: {error}"),
            }
        }
    }

    /// Whether anything came within `wait`.
    pub fn page_within(&self, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        loop {
            match self.received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok((_, FromFetcher::Page { origin: Origin::PushedPackets, bytes })) if has_header(&bytes) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    }
}

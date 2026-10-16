//! The fetcher process (`architecture.md` section 6.1): it holds the
//! network connections of one channel and nothing else. The engine's web
//! module opens the channel, reads its contents file, fetches every page
//! the contents file lists in that order at the engine's polite rate, and
//! holds the channel's push connection; this program only moves bytes
//! between the agent and the sockets, and sends what arrives to the
//! trusted core on standard output.
//!
//! Usage: `t42-fetcher <channel address> [--allow-local <ip>]...`. A
//! server on this computer or the local network is refused unless named
//! with `--allow-local` (the engine's REQ-SEC-006).

// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

mod transport;

use std::collections::VecDeque;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufWriter, Read};
use std::net::IpAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant as Clock};

use neoteletext_core::types::{Instant, PageAddress};
use neoteletext_web::agent::{Config, Event, Input, Output, Source, Storage, Subscription, UserAgent};
use neoteletext_web::consts::ALLOWED_LOCAL;
use neoteletext_web::contents::{self, Entry as ContentsEntry};
use neoteletext_web::port::{ConnectionId, NetOutput, Pump, Security};
use protocol::fetcher::{Entry, FromFetcher, write_to_core};

use crate::transport::{Tcp, address};

/// The engine's own sizes for the cookie jar (its REQ-WEB-006 defaults).
const COOKIE_SLOTS: usize = 100;
const COOKIE_BYTES: usize = 32 * 1024;
/// Bytes read from a socket at a time.
const READ_CHUNK: usize = 16 * 1024;
/// How long the loop sleeps when nothing happened: short enough that a
/// pushed page arrives without noticeable delay.
const IDLE: Duration = Duration::from_millis(5);

/// Randomness for the agent (WebSocket masks), from the operating system's
/// per-process random keys, which `RandomState` draws.
struct Random {
    state: RandomState,
    counter: u64,
}

impl neoteletext_web::port::RandomSource for Random {
    fn fill(&mut self, bytes: &mut [u8]) {
        for chunk in bytes.chunks_mut(8) {
            self.counter = self.counter.wrapping_add(1);
            let mut hasher = self.state.build_hasher();
            hasher.write_u64(self.counter);
            let value = hasher.finish().to_le_bytes();
            for (byte, &random) in chunk.iter_mut().zip(&value) {
                *byte = random;
            }
        }
    }
}

/// What the agent asked for, owned, to act on after its call returns.
enum Action {
    Connect {
        connection: ConnectionId,
        host: String,
        port: u16,
        security: Security,
    },
    Send(ConnectionId, Vec<u8>),
    Close(ConnectionId),
    Timer(Instant),
    Bytes {
        pushed: bool,
        bytes: Vec<u8>,
    },
    Ready,
    Contents(Vec<u8>),
    /// The page asked for has been answered, one way or another.
    Answered,
    Notice(String),
}

fn collect(output: Output<'_>, actions: &mut Vec<Action>) {
    if let Some(net) = output.net() {
        actions.push(match net {
            NetOutput::Connect { connection, host, port, security } => Action::Connect { connection, host: host.to_owned(), port, security },
            NetOutput::Send(connection, bytes) => Action::Send(connection, bytes.to_vec()),
            NetOutput::Close(connection) => Action::Close(connection),
        });
        return;
    }
    if let Output::PageBytes(page) = output {
        actions.push(Action::Bytes { pushed: page.source == Source::Push, bytes: page.bytes.to_vec() });
        // A bundle, a form's answer or a push is page bytes too; only a page
        // fetch ends the wait for the next one.
        if matches!(page.source, Source::Page { .. }) {
            actions.push(Action::Answered);
        }
        return;
    }
    actions.push(match output {
        Output::Timer { at } => Action::Timer(at),
        Output::Event(Event::ServiceReady { .. }) => Action::Ready,
        Output::Event(Event::Contents(bytes)) => Action::Contents(bytes.to_vec()),
        Output::Event(Event::PageCurrent { .. } | Event::PageNotInService { .. } | Event::FetchFailed { .. }) => Action::Answered,
        Output::Event(event) => Action::Notice(format!("{event:?}")),
        _ => return,
    });
}

fn contents_entries(bytes: &[u8]) -> (Vec<Entry>, Vec<PageAddress>) {
    let mut entries = Vec::new();
    let mut pages = Vec::new();
    let Ok(read) = contents::read(bytes) else { return (entries, pages) };
    for entry in read.flatten() {
        match entry {
            ContentsEntry::Section { title } => entries.push(Entry::Section(title.to_owned())),
            ContentsEntry::Page { page, title, .. } => {
                entries.push(Entry::Page { page: page.mpp(), title: title.to_owned() });
                pages.push(page);
            }
            ContentsEntry::Channel { .. } => {}
        }
    }
    (entries, pages)
}

fn run(channel: &str, allowed: &[IpAddr]) -> Result<(), String> {
    let mut storage = Box::new(Storage::default());
    // No TLS in this port yet: https addresses are reported unreachable.
    let mut config = Config { tls_available: false, redirects_within_channel: true, ..Config::default() };
    for (slot, &ip) in config.allowed_local.iter_mut().zip(allowed.iter().take(ALLOWED_LOCAL)) {
        *slot = Some(address(ip));
    }
    let random = Random { state: RandomState::new(), counter: 0 };
    let mut agent: Box<UserAgent<'_, Random, COOKIE_SLOTS, COOKIE_BYTES>> = Box::new(UserAgent::new(config, storage.buffers(), random));
    let mut pump = Pump::new(Tcp::default());
    let mut output = BufWriter::new(io::stdout().lock());
    let start = Clock::now();
    let clock = || Instant(u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX));
    let mut buffer = vec![0_u8; READ_CHUNK];
    let mut next_tick: Option<Instant> = None;
    let mut queue: VecDeque<PageAddress> = VecDeque::new();
    let mut waiting = false;

    // The trusted core closes standard input to end the fetcher.
    let closed = Arc::new(AtomicBool::new(false));
    let flag = closed.clone();
    std::thread::spawn(move || {
        let mut sink = [0_u8; 64];
        while matches!(io::stdin().read(&mut sink), Ok(count) if count > 0) {}
        flag.store(true, Ordering::Relaxed);
    });

    let mut actions = Vec::new();
    agent.handle(clock(), Input::OpenChannel(channel), &mut |out| collect(out, &mut actions));
    while !closed.load(Ordering::Relaxed) {
        let mut busy = !actions.is_empty();
        for action in std::mem::take(&mut actions) {
            let send = |output: &mut BufWriter<io::StdoutLock<'_>>, message: FromFetcher| write_to_core(output, &message).map_err(|error| error.to_string());
            match action {
                Action::Connect { connection, host, port, security } => pump.execute(NetOutput::Connect { connection, host: &host, port, security }),
                Action::Send(connection, bytes) => pump.execute(NetOutput::Send(connection, &bytes)),
                Action::Close(connection) => pump.execute(NetOutput::Close(connection)),
                Action::Timer(at) => next_tick = Some(next_tick.map_or(at, |current| current.min(at))),
                Action::Bytes { pushed, bytes } => send(&mut output, FromFetcher::Page { pushed, bytes })?,
                Action::Ready => {
                    // The whole channel: its contents file, then every page it
                    // lists; and the push connection, for all its pages.
                    agent.handle(clock(), Input::FetchContents, &mut |out| collect(out, &mut actions));
                    agent.handle(clock(), Input::Subscribe(Subscription::All), &mut |out| collect(out, &mut actions));
                }
                Action::Contents(bytes) => {
                    let (entries, pages) = contents_entries(&bytes);
                    send(&mut output, FromFetcher::Contents(entries))?;
                    queue.extend(pages);
                }
                Action::Answered => waiting = false,
                Action::Notice(text) => send(&mut output, FromFetcher::Notice(text))?,
            }
        }
        // One page at a time, in the contents file's order; the agent
        // spaces the requests at the engine's rate.
        if !waiting && let Some(page) = queue.pop_front() {
            waiting = true;
            busy = true;
            agent.handle(clock(), Input::FetchPage { page, subcode: None }, &mut |out| collect(out, &mut actions));
        }
        for connection in [ConnectionId::Fetch, ConnectionId::Push] {
            if let Some(input) = pump.poll(connection, &mut buffer) {
                busy = true;
                agent.handle(clock(), Input::Net(input), &mut |out| collect(out, &mut actions));
            }
        }
        if next_tick.is_some_and(|at| at <= clock()) {
            next_tick = None;
            busy = true;
            agent.handle(clock(), Input::Tick, &mut |out| collect(out, &mut actions));
        }
        if !busy {
            std::thread::sleep(IDLE);
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let Some(channel) = arguments.next() else {
        eprintln!("t42-fetcher: usage: t42-fetcher <channel address> [--allow-local <ip>]...");
        return ExitCode::FAILURE;
    };
    let mut allowed = Vec::new();
    while let Some(argument) = arguments.next() {
        if let ("--allow-local", Some(Ok(ip))) = (argument.as_str(), arguments.next().map(|ip| ip.parse::<IpAddr>())) {
            allowed.push(ip);
        } else {
            eprintln!("t42-fetcher: {argument}: not understood");
            return ExitCode::FAILURE;
        }
    }
    match run(&channel, &allowed) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("t42-fetcher: {error}");
            ExitCode::FAILURE
        }
    }
}

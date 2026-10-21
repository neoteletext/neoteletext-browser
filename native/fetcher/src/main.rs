//! The fetcher process (`architecture.md` section 6.1): it holds the
//! network connections of one channel and nothing else. The engine's web
//! module opens the channel, reads its contents file, fetches every page
//! the contents file lists in that order at the engine's polite rate, and
//! holds the channel's push connection; this program only moves bytes
//! between the agent and the sockets, and sends what arrives to the
//! trusted core on standard output.
//!
//! Usage: `neoteletext-fetcher <channel address> [--allow-local <ip>]...
//! [--proxy <url>]`. A server on this computer or the local network is
//! refused unless named with `--allow-local` (the engine's REQ-SEC-006).
//! With `--proxy http://host:port` or `socks5://host:port` every
//! connection goes through that proxy (requirement 19.10); it is given only
//! here, when the fetcher starts (requirement 19.12).

// On Windows, a desktop program: no console window opens with it. The
// processes talk through the pipes they are given, not a console.
#![cfg_attr(windows, windows_subsystem = "windows")]
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
use std::io::{self, BufWriter, Write};
use std::net::IpAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant as Clock};

use neoteletext_core::types::{Instant, PageAddress};
use neoteletext_web::agent::{Config, Event, Input, Output, Source, Storage, Subscription, UserAgent};
use neoteletext_web::consts::ALLOWED_LOCAL;
use neoteletext_web::contents::{self, Entry as ContentsEntry};
use neoteletext_web::http::MediaType;
use neoteletext_web::port::{ConnectionId, NetOutput, Pump, Security};
use neoteletext_web::proxy::Proxy;
use protocol::fetcher::{Entry, FromFetcher, Origin, write_to_core};

use crate::transport::{Tcp, address};

/// The engine's own sizes for the cookie jar (its REQ-WEB-006 defaults).
const COOKIE_SLOTS: usize = 100;
const COOKIE_BYTES: usize = 32 * 1024;
/// Bytes passed to the agent at a time.
const READ_CHUNK: usize = 16 * 1024;
// An exchange reports an address the engine accepted, so the protocol's
// bound must hold the engine's longest.
const _: () = assert!(protocol::fetcher::MAX_EXCHANGE_URL >= neoteletext_web::consts::MAX_URL_LENGTH);

/// The wake that ends the loop's wait: none at once while the loop is
/// busy; otherwise the first to come, an input message the moment it
/// arrives, or none once the agent's timer is due; and the end once the
/// wakers are gone.
fn wait_for_wake(woken: &mpsc::Receiver<Wake>, busy: bool, next_tick: Option<Instant>, now: Instant) -> Option<Wake> {
    match (busy, next_tick) {
        (true, _) => woken.try_recv().ok(),
        (false, Some(at)) => match woken.recv_timeout(Duration::from_micros(at.0.saturating_sub(now.0))) {
            Ok(wake) => Some(wake),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(Wake::Ended),
        },
        (false, None) => Some(woken.recv().unwrap_or(Wake::Ended)),
    }
}

/// What ends the loop's wait: it sleeps until one comes, or until the
/// agent's next timer (REQ-INT-135).
enum Wake {
    /// An input message from the trusted core, sent at once.
    #[cfg(feature = "interaction")]
    Record(Vec<u8>),
    /// A socket has bytes, or has ended.
    Network,
    /// The trusted core closed standard input.
    Ended,
}

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
        origin: Origin,
        bytes: Vec<u8>,
    },
    Ready,
    Contents(Vec<u8>),
    /// The page asked for has been answered, one way or another.
    Answered,
    /// A message for the trusted core as it is: a notice, or an exchange
    /// for the developer console (the engine's REQ-WEB-033).
    Tell(FromFetcher),
    /// The push connection opened or was lost.
    Push(bool),
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
        let origin = match (page.source, page.media) {
            (Source::Push, MediaType::T42) => Origin::PushedPackets,
            (Source::Push, _) => Origin::PushedPage,
            (_, MediaType::T42) => Origin::FetchedPackets,
            (_, MediaType::Tti) => Origin::FetchedPage,
            _ => Origin::Fetched,
        };
        actions.push(Action::Bytes { origin, bytes: page.bytes.to_vec() });
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
        Output::Event(Event::PushConnected(_)) => Action::Push(true),
        Output::Event(Event::Exchanged { method, url, status, bytes, elapsed_us }) => Action::Tell(FromFetcher::Exchange {
            method: String::from_utf8_lossy(method.token()).into_owned(),
            url: url.to_owned(),
            status,
            bytes: u64::try_from(bytes).unwrap_or(u64::MAX),
            elapsed_us,
        }),
        // A lost connection waits before the next; a bad message that
        // keeps the connection says it is retried at once, and is not one.
        Output::Event(Event::PushLost { retry_in_us, .. }) if retry_in_us > 0 => Action::Push(false),
        // An input message not sent: nothing is told, as nothing was asked.
        #[cfg(feature = "interaction")]
        Output::Event(Event::InputDropped) => return,
        Output::Event(event) => Action::Tell(FromFetcher::Notice(format!("{event:?}"))),
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

/// The browser as it names itself to channels, as browsers should have
/// (requirement 5.14, the engine's REQ-WEB-031): `NeoTeletext/0.1
/// (macOS)`, its own product, so the engine is not named again.
fn product() -> Option<neoteletext_web::user_agent::Product> {
    neoteletext_web::user_agent::os_family().map(|family| neoteletext_web::user_agent::Product {
        name: "NeoTeletext",
        version: env!("CARGO_PKG_VERSION"),
        family,
        engine_own: true,
    })
}

/// The aliases and named services of the open channel's descriptor, as
/// written: untrusted, for the trusted core to parse. Aliases of numbers
/// that are not display pages are left out.
#[cfg(feature = "links")]
fn descriptor_links(agent: &UserAgent<'_, Random, COOKIE_SLOTS, COOKIE_BYTES>) -> Option<FromFetcher> {
    use protocol::links::{Alias, MAX_ALIASES, MAX_SERVICES, Service};
    let descriptor = agent.descriptor()?;
    let aliases: Vec<Alias> = descriptor
        .aliases()
        .filter(|alias| interface_core::channel::page_index(alias.page.mpp()).is_some())
        .map(|alias| Alias { page: alias.page.mpp(), target: descriptor.alias_target(alias).to_owned() })
        .take(MAX_ALIASES)
        .collect();
    let services: Vec<Service> = descriptor
        .services()
        .map(|service| Service { name: service.short_name.as_str().to_owned(), address: descriptor.service_address(service).to_owned() })
        .take(MAX_SERVICES)
        .collect();
    (!aliases.is_empty() || !services.is_empty()).then_some(FromFetcher::Links { aliases, services })
}

fn run(channel: &str, allowed: &[IpAddr], proxy: Option<Proxy>) -> Result<(), String> {
    let mut storage = Box::new(Storage::default());
    // Proxied, local names are refused before the proxy is asked (REQ-WEB-032).
    let mut config = Config { tls_available: true, redirects_within_channel: true, product: product(), proxied: proxy.is_some(), ..Config::default() };
    for (slot, &ip) in config.allowed_local.iter_mut().zip(allowed.iter().take(ALLOWED_LOCAL)) {
        *slot = Some(address(ip));
    }
    let random = Random { state: RandomState::new(), counter: 0 };
    let mut agent: Box<UserAgent<'_, Random, COOKIE_SLOTS, COOKIE_BYTES>> = Box::new(UserAgent::new(config, storage.buffers(), random));
    let (wakes, woken) = mpsc::channel::<Wake>();
    let network = wakes.clone();
    let mut pump = Pump::new(Tcp::new(
        Arc::new(move || {
            let _ = network.send(Wake::Network);
        }),
        proxy,
    ));
    let mut output = BufWriter::new(io::stdout().lock());
    let start = Clock::now();
    let clock = || Instant(u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX));
    let mut buffer = vec![0_u8; READ_CHUNK];
    let mut next_tick: Option<Instant> = None;
    let mut queue: VecDeque<PageAddress> = VecDeque::new();
    let mut waiting = false;

    // The trusted core closes standard input to end the fetcher. Before
    // then it sends only input messages (REQ-INT-133); anything else
    // ends the fetcher too.
    std::thread::spawn(move || {
        read_commands(&wakes);
        let _ = wakes.send(Wake::Ended);
    });

    let mut actions = Vec::new();
    agent.handle(clock(), Input::OpenChannel(channel), &mut |out| collect(out, &mut actions));
    loop {
        let mut busy = !actions.is_empty();
        for action in std::mem::take(&mut actions) {
            let send = |output: &mut BufWriter<io::StdoutLock<'_>>, message: FromFetcher| write_to_core(output, &message).map_err(|error| error.to_string());
            match action {
                Action::Connect { connection, host, port, security } => pump.execute(NetOutput::Connect { connection, host: &host, port, security }),
                Action::Send(connection, bytes) => pump.execute(NetOutput::Send(connection, &bytes)),
                Action::Close(connection) => pump.execute(NetOutput::Close(connection)),
                Action::Timer(at) => next_tick = Some(next_tick.map_or(at, |current| current.min(at))),
                Action::Bytes { origin, bytes } => send(&mut output, FromFetcher::Page { origin, bytes })?,
                Action::Ready => {
                    // The descriptor's links, for the trusted core to decide
                    // (`hyperlinks.md` section 6.4).
                    #[cfg(feature = "links")]
                    if let Some(links) = descriptor_links(&agent) {
                        send(&mut output, links)?;
                    }
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
                // Keys may go to the channel while its WebSocket is open, if
                // its descriptor says it takes them (`interaction.md`).
                Action::Push(open) => {
                    // The trusted core keeps a sleeping tab's fetcher while
                    // it holds a push connection (decision 47).
                    send(&mut output, FromFetcher::Pushing(open))?;
                    #[cfg(feature = "interaction")]
                    if let Some(descriptor) = agent.descriptor().filter(|descriptor| descriptor.input()) {
                        let secure = descriptor.push().is_some_and(|(url, _)| url.is_secure());
                        send(&mut output, FromFetcher::Input { open, secure })?;
                    }
                }
                Action::Tell(message) => send(&mut output, message)?,
            }
        }
        if pump.transport.take_proxy_failure() {
            write_to_core(&mut output, &FromFetcher::ProxyFailed).map_err(|error| error.to_string())?;
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
        // What this turn produced goes before waiting.
        output.flush().map_err(|error| error.to_string())?;
        let first = wait_for_wake(&woken, busy, next_tick, clock());
        for wake in first.into_iter().chain(std::iter::from_fn(|| woken.try_recv().ok())) {
            match wake {
                #[cfg(feature = "interaction")]
                Wake::Record(record) => agent.handle(clock(), Input::Send(&record), &mut |out| collect(out, &mut actions)),
                // The sockets are looked at on the next turn.
                Wake::Network => {}
                Wake::Ended => return Ok(()),
            }
        }
    }
}

/// Reads the trusted core's input messages until its pipe closes or breaks
/// the protocol.
fn read_commands(commands: &mpsc::Sender<Wake>) {
    let mut input = io::BufReader::new(io::stdin().lock());
    #[cfg(feature = "interaction")]
    while let Ok(Some(protocol::fetcher::ToFetcher::Send(record))) = protocol::fetcher::read_to_fetcher(&mut input) {
        if commands.send(Wake::Record(record)).is_err() {
            return;
        }
    }
    #[cfg(not(feature = "interaction"))]
    {
        use std::io::Read;
        // Without typing the fetcher takes nothing: the pipe only ends it.
        let _ = commands;
        let mut sink = [0_u8; 64];
        while matches!(input.read(&mut sink), Ok(count) if count > 0) {}
    }
}

/// What the trusted core started the fetcher with.
struct Arguments {
    channel: String,
    allowed: Vec<IpAddr>,
    proxy: Option<Proxy>,
}

fn arguments(given: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    let mut given = given.into_iter();
    let channel = given.next().ok_or("usage: neoteletext-fetcher <channel address> [--allow-local <ip>]... [--proxy <url>]")?;
    let mut read = Arguments { channel, allowed: Vec::new(), proxy: None };
    while let Some(argument) = given.next() {
        let value = given.next().ok_or_else(|| format!("{argument} needs a value"))?;
        match argument.as_str() {
            "--allow-local" => read.allowed.push(value.parse::<IpAddr>().map_err(|_| format!("{value}: not an IP address"))?),
            "--proxy" if read.proxy.is_none() => read.proxy = Some(Proxy::parse(&value)?),
            _ => return Err(format!("{argument}: not understood")),
        }
    }
    Ok(read)
}

fn main() -> ExitCode {
    // First, before any thread: the network and nothing else (requirement 3.3).
    match confine::apply(confine::Role::Network) {
        Ok(report) => eprintln!("neoteletext-fetcher: confined: {report}"),
        Err(error) => {
            eprintln!("neoteletext-fetcher: {error}");
            return ExitCode::FAILURE;
        }
    }
    let read = match arguments(std::env::args().skip(1)) {
        Ok(read) => read,
        Err(error) => {
            eprintln!("neoteletext-fetcher: {error}");
            return ExitCode::FAILURE;
        }
    };
    match run(&read.channel, &read.allowed, read.proxy) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-fetcher: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requirement 5.14, the engine's REQ-WEB-031: the browser names itself
    /// `NeoTeletext/0.1 (<family>)`.
    #[test]
    fn the_user_agent_names_the_browser() {
        let product = product().expect("a desktop names its family");
        let named = neoteletext_web::user_agent::UserAgentString::new(&product).expect("a valid user agent");
        let family = neoteletext_web::user_agent::os_family().expect("a desktop names its family");
        assert_eq!(named.as_str(), format!("NeoTeletext/0.1 ({family})"));
    }

    fn read(given: &[&str]) -> Result<Arguments, String> {
        arguments(given.iter().map(|&argument| argument.to_owned()))
    }

    /// Requirement 19.12: the fetcher takes its proxy as an argument when
    /// it starts, once.
    #[test]
    fn the_fetcher_reads_its_proxy_argument() {
        let plain = read(&["https://a.example/"]).unwrap();
        assert!(plain.proxy.is_none() && plain.allowed.is_empty());
        let given = read(&["https://a.example/", "--allow-local", "10.0.0.2", "--proxy", "http://p.example:3128"]).unwrap();
        let proxy = given.proxy.unwrap();
        assert_eq!((proxy.kind, proxy.host.as_str(), proxy.port), (neoteletext_web::proxy::ProxyKind::Http, "p.example", 3128));
        assert_eq!(given.allowed, ["10.0.0.2".parse::<IpAddr>().unwrap()]);
        assert!(read(&["https://a.example/", "--proxy", "ftp://p.example:21"]).is_err());
        assert!(read(&["https://a.example/", "--proxy"]).is_err());
        assert!(read(&["https://a.example/", "--proxy", "http://p:1", "--proxy", "http://q:1"]).is_err(), "one proxy");
        assert!(read(&[]).is_err());
    }
}

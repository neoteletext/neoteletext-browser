//! The fetcher's loop: the agent's output turned into actions on the
//! sockets and messages for the trusted core, one page at a time in the
//! contents file's order, and the trusted core's messages read on a
//! thread of their own. The input messages of one moment reach the agent
//! together, so they leave in one write (REQ-RCH-010).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, BufWriter, Read, Write};
use std::net::IpAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant as Clock};

use neoteletext_core::types::{Instant, PageAddress, SubCode};
#[cfg(feature = "interaction")]
use neoteletext_web::agent::InputMessage;
use neoteletext_web::agent::{Config, Event, Input, Output, Source, Storage, Subscription, UserAgent};
use neoteletext_web::consts::ALLOWED_LOCAL;
use neoteletext_web::contents::{self, Entry as ContentsEntry};
use neoteletext_web::http::MediaType;
use neoteletext_web::port::{ConnectionId, NetOutput, Pump, Security};
use neoteletext_web::proxy::Proxy;
use neoteletext_web::subpages;
use protocol::fetcher::{CATALOGUE_PART, Entry, FromFetcher, Origin, ToFetcher, read_to_fetcher, write_to_core};
use protocol::subpages::SubPageRows;

#[cfg(feature = "links")]
use crate::agent::descriptor_links;
use crate::agent::{Random, product};
use crate::cookies;
use crate::transport::{AllowedLocal, Tcp, address};

/// The agent's own cookie jar holds nothing: the trusted core keeps the
/// cookies (decision 62).
pub(crate) const COOKIE_SLOTS: usize = 0;
pub(crate) const COOKIE_BYTES: usize = 0;
const _: () = assert!(size_of::<cookies::UnusedJar>() == size_of::<neoteletext_web::cookie::CookieJar<COOKIE_SLOTS, COOKIE_BYTES>>());
/// What the console is told when a name led only to addresses the channel
/// may not reach.
const LOCAL_REFUSED: &str = "Blocked: the server's name leads to this computer or the local network, which this channel may not reach";
/// The longest body the agent takes for a page, a bundle or a form's
/// answer: what one page message to the trusted core carries, its payload
/// being the origin byte and then the bytes (`write_to_core` refuses
/// more, and refusing ends this fetcher; finding F-74).
const PAGE_LIMIT: usize = protocol::MAX_PAYLOAD - 1;
// The agent's body buffer holds it, so it is the limit the agent keeps.
const _: () = assert!(PAGE_LIMIT <= neoteletext_web::consts::DEFAULT_PAGE_LIMIT);
// A pushed message goes to the trusted core as a page message too.
const _: () = assert!(neoteletext_web::consts::DEFAULT_PUSH_LIMIT <= PAGE_LIMIT);
/// Bytes passed to the agent at a time.
const READ_CHUNK: usize = 16 * 1024;
/// The catalogues of sub-pages kept as read, so that one the agent says
/// is current (304, or fresh by its lifetime) is sent again without a
/// request: as many as the trusted core holds a channel.
const CATALOGUES_KEPT: usize = 16;
// A catalogue's entries are the engine's, within the standard's maximum,
// and its titles a row's, within a page title's bound on the wire.
const _: () = assert!(subpages::MAX_ENTRIES == neoteletext_core::limits::MAX_SUBPAGES);
const _: () = assert!(subpages::MAX_TITLE * 4 <= protocol::MAX_PAGE_TITLE);
// An exchange reports an address the engine accepted, so the protocol's
// bound must hold the engine's longest.
const _: () = assert!(protocol::fetcher::MAX_EXCHANGE_URL >= neoteletext_web::consts::MAX_URL_LENGTH);
// A move the trusted core tags is of a contact the agent holds a slot for,
// and fits it (REQ-RCH-035).
#[cfg(feature = "interaction")]
const _: () = assert!(neoteletext_input::MAX_CONTACTS == neoteletext_web::consts::PUSH_MOVE_SLOTS);
#[cfg(feature = "interaction")]
const _: () = assert!(neoteletext_input::event::MAX_MOVE_BYTES <= neoteletext_web::consts::PUSH_MOVE_MAX);
/// The input messages of one moment handed to the loop at most together:
/// a move of every contact and one other event, the most the trusted core
/// writes at one moment (REQ-RCH-010, REQ-RCH-033). More are handed on in
/// groups of this many, still in order.
#[cfg(feature = "interaction")]
const MOMENT_MESSAGES: usize = neoteletext_web::consts::PUSH_MOVE_SLOTS + 1;

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
    /// The input messages of one moment from the trusted core, in order,
    /// sent at once and together.
    #[cfg(feature = "interaction")]
    Records(Vec<Record>),
    /// The trusted core turns the round trip's measurement on or off
    /// (REQ-RCH-016).
    #[cfg(feature = "interaction")]
    RoundTrip(bool),
    /// The trusted core asks for a page's catalogue of sub-pages, or for
    /// one sub-page (decision 112).
    Fetch(ToFetcher),
    /// A socket has bytes, or has ended.
    Network,
    /// The trusted core closed standard input.
    Ended,
}

/// One input message from the trusted core, checked by the protocol's
/// reader: its bytes, and the contact whose move it is, alone.
#[cfg(feature = "interaction")]
#[derive(Debug, PartialEq, Eq)]
struct Record {
    bytes: Vec<u8>,
    move_of: Option<u8>,
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
    /// A page's catalogue of sub-pages, as fetched.
    SubPages(PageAddress, Vec<u8>),
    /// A page's catalogue is current: the one kept is sent again.
    SubPagesCurrent(PageAddress),
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
        Output::Event(Event::SubPages { page, bytes }) => Action::SubPages(page, bytes.to_vec()),
        Output::Event(Event::SubPagesCurrent { page }) => Action::SubPagesCurrent(page),
        Output::Event(Event::SubPagesMissing { page }) => Action::Tell(FromFetcher::SubPagesMissing { page: page.mpp() }),
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
        Output::Event(Event::PushLost { retry_in_us, problem }) if retry_in_us > 0 => {
            // Why the push connection went is otherwise never seen.
            eprintln!("neoteletext-fetcher: the push connection was lost ({problem:?}); trying again in {retry_in_us} µs");
            Action::Push(false)
        }
        // An input message not sent: nothing is told, as nothing was asked.
        #[cfg(feature = "interaction")]
        Output::Event(Event::InputDropped) => return,
        // The push WebSocket's round trip, for the strip (REQ-RCH-016).
        #[cfg(feature = "interaction")]
        Output::Event(Event::RoundTrip { microseconds }) => Action::Tell(FromFetcher::RoundTrip { microseconds }),
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

/// A page's catalogue of sub-pages, read by the engine's bounded reader
/// (REQ-WEB-034), as the parts the trusted core takes: every entry the
/// reader keeps, at most the standard's 8,190, in the page's order, at
/// most [`CATALOGUE_PART`] a part. A catalogue refused whole, or with no
/// entry, is one part of none.
fn catalogue_parts(bytes: &[u8]) -> Vec<SubPageRows> {
    let entries: Vec<(u16, String)> = match subpages::read(bytes) {
        Ok(entries) => entries.flatten().map(|entry| (entry.subcode.value(), entry.title.unwrap_or_default().to_owned())).collect(),
        Err(_) => Vec::new(),
    };
    let count = u16::try_from(entries.len().min(subpages::MAX_ENTRIES)).unwrap_or(0);
    if entries.is_empty() {
        return vec![SubPageRows::default()];
    }
    entries.chunks(CATALOGUE_PART).zip((0_u16..).step_by(CATALOGUE_PART)).map(|(rows, first)| SubPageRows { count, first, rows: rows.to_vec() }).collect()
}

/// The catalogues of sub-pages last read, the most recent last.
#[derive(Default)]
struct Catalogues(VecDeque<(PageAddress, Vec<u8>)>);

impl Catalogues {
    /// Keeps `bytes` as `page`'s catalogue, forgetting the oldest past
    /// [`CATALOGUES_KEPT`].
    fn keep(&mut self, page: PageAddress, bytes: Vec<u8>) {
        self.0.retain(|(kept, _)| *kept != page);
        self.0.push_back((page, bytes));
        if self.0.len() > CATALOGUES_KEPT {
            self.0.pop_front();
        }
    }

    /// The catalogue kept of `page`.
    fn get(&self, page: PageAddress) -> Option<&[u8]> {
        self.0.iter().find(|(kept, _)| *kept == page).map(|(_, bytes)| bytes.as_slice())
    }
}

/// Sends `page`'s catalogue to the trusted core in parts.
fn send_catalogue(output: &cookies::Output, page: PageAddress, bytes: &[u8]) -> Result<(), String> {
    for part in catalogue_parts(bytes) {
        send(output, &FromFetcher::SubPages { page: page.mpp(), part })?;
    }
    Ok(())
}

/// The agent's settings for a channel that may reach the local addresses
/// `allowed`, through a proxy when `proxied`.
fn agent_config(allowed: &[IpAddr], proxied: bool) -> Config {
    // Proxied, local names are refused before the proxy is asked (REQ-WEB-032).
    let mut config = Config { tls_available: true, redirects_within_channel: true, product: product(), proxied, page_limit: PAGE_LIMIT, ..Config::default() };
    for (slot, &ip) in config.allowed_local.iter_mut().zip(allowed.iter().take(ALLOWED_LOCAL)) {
        *slot = Some(address(ip));
    }
    config
}

/// Holds the channel `channel`'s connections until the trusted core closes
/// standard input: the engine's agent opens it, fetches its contents file
/// and every page it lists, and holds its push connection; what arrives
/// goes to the trusted core on standard output. A server on this computer
/// or the local network is reached only when `allowed` names it. Its
/// first message says it is confined, as `main.rs` has made it.
///
/// # Errors
/// When writing to the trusted core fails.
pub fn run(channel: &str, allowed: &[IpAddr], proxy: Option<Proxy>) -> Result<(), String> {
    // Before anything of the channel, so the trusted core can tell a
    // fetcher that could not start from one its server ended (finding
    // F-12).
    let output: cookies::Output = Rc::new(RefCell::new(BufWriter::new(io::stdout())));
    send(&output, &FromFetcher::Confined)?;
    output.borrow_mut().flush().map_err(|error| error.to_string())?;
    let mut storage = Box::new(Storage::default());
    let config = agent_config(allowed, proxy.is_some());
    // The transport refuses the same local addresses before connecting
    // (finding F-62).
    let allowed_local = config.allowed_local;
    let random = Random::new();
    let (wakes, woken) = mpsc::channel::<Wake>();
    // The trusted core's answers to the cookie questions, read by the same
    // thread that reads its input messages.
    let (answers, answered) = mpsc::channel();
    let mut store = cookies::CoreCookies::new(Rc::clone(&output), answered);
    let mut agent: Box<Agent<'_>> = Box::new(UserAgent::with_cookies(config, storage.buffers(), random, &mut store));
    let mut pump = sockets(wakes.clone(), proxy, allowed_local);
    let start = Clock::now();
    let clock = || Instant(u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX));
    let mut buffer = vec![0_u8; READ_CHUNK];
    let mut next_tick: Option<Instant> = None;
    let mut queue: VecDeque<PageAddress> = VecDeque::new();
    let mut waiting = false;
    let mut catalogues = Catalogues::default();

    // The trusted core closes standard input to end the fetcher. Before
    // then it sends only input messages (REQ-INT-133); anything else
    // ends the fetcher too.
    std::thread::spawn(move || {
        read_commands(&mut io::BufReader::new(io::stdin().lock()), &wakes, &answers);
        let _ = wakes.send(Wake::Ended);
    });

    let mut actions = Vec::new();
    agent.handle(clock(), Input::OpenChannel(channel), &mut |out| collect(out, &mut actions));
    loop {
        let mut busy = !actions.is_empty();
        for action in std::mem::take(&mut actions) {
            match action {
                Action::Connect { connection, host, port, security } => pump.execute(NetOutput::Connect { connection, host: &host, port, security }),
                Action::Send(connection, bytes) => pump.execute(NetOutput::Send(connection, &bytes)),
                Action::Close(connection) => pump.execute(NetOutput::Close(connection)),
                Action::Timer(at) => next_tick = Some(next_tick.map_or(at, |current| current.min(at))),
                Action::Bytes { origin, bytes } => send(&output, &FromFetcher::Page { origin, bytes })?,
                Action::Ready => {
                    // The descriptor's links, for the trusted core to decide
                    // (`hyperlinks.md` section 6.4).
                    #[cfg(feature = "links")]
                    if let Some(links) = descriptor_links(&agent) {
                        send(&output, &links)?;
                    }
                    ask_for_channel(&mut agent, &clock, &mut actions);
                }
                Action::Contents(bytes) => {
                    let (entries, pages) = contents_entries(&bytes);
                    send(&output, &FromFetcher::Contents(entries))?;
                    queue.extend(pages);
                }
                Action::SubPages(page, bytes) => {
                    send_catalogue(&output, page, &bytes)?;
                    catalogues.keep(page, bytes);
                }
                // A catalogue no longer kept, though the agent holds its
                // validators, is not sent: the page is listed from what
                // its decoder holds until the catalogue next changes.
                Action::SubPagesCurrent(page) => {
                    if let Some(bytes) = catalogues.get(page) {
                        send_catalogue(&output, page, bytes)?;
                    }
                }
                Action::Answered => waiting = false,
                Action::Push(open) => push_changed(&agent, &output, open)?,
                Action::Tell(message) => send(&output, &message)?,
            }
        }
        tell_failures(&mut pump.transport, &output)?;
        // One page at a time, in the contents file's order; the agent
        // spaces the requests at the engine's rate.
        if !waiting && let Some(page) = queue.pop_front() {
            waiting = true;
            busy = true;
            agent.handle(clock(), Input::FetchPage { page, subcode: None }, &mut |out| collect(out, &mut actions));
        }
        busy |= poll_sockets(&mut pump, &mut buffer, &mut agent, &clock, &mut actions);
        if next_tick.is_some_and(|at| at <= clock()) {
            next_tick = None;
            busy = true;
            agent.handle(clock(), Input::Tick, &mut |out| collect(out, &mut actions));
        }
        // What this turn produced goes before waiting.
        output.borrow_mut().flush().map_err(|error| error.to_string())?;
        let first = wait_for_wake(&woken, busy, next_tick, clock());
        // Every wake already here, so the input messages read at the same
        // moment are known to follow one another (REQ-RCH-010).
        let mut woke: VecDeque<Wake> = first.into_iter().chain(std::iter::from_fn(|| woken.try_recv().ok())).collect();
        while let Some(wake) = woke.pop_front() {
            match wake {
                #[cfg(feature = "interaction")]
                Wake::Records(records) => {
                    let more_after = matches!(woke.front(), Some(Wake::Records(_)));
                    give_records(&records, more_after, &mut |input| agent.handle(clock(), input, &mut |out| collect(out, &mut actions)));
                }
                #[cfg(feature = "interaction")]
                Wake::RoundTrip(on) => agent.handle(clock(), Input::MeasureRoundTrip(on), &mut |out| collect(out, &mut actions)),
                Wake::Fetch(asked) => fetch(&mut agent, &clock, &asked, &mut actions),
                // The sockets are looked at on the next turn.
                Wake::Network => {}
                Wake::Ended => return Ok(()),
            }
        }
    }
}

/// The agent with the cookie jar it keeps: none of its own.
type Agent<'a> = UserAgent<'a, Random, COOKIE_SLOTS, COOKIE_BYTES>;

/// The sockets, through `proxy` when there is one, refusing the local
/// addresses but `allowed_local`; each wakes the loop when it has bytes
/// or has ended.
fn sockets(wakes: mpsc::Sender<Wake>, proxy: Option<Proxy>, allowed_local: AllowedLocal) -> Pump<Tcp> {
    Pump::new(Tcp::new(
        Arc::new(move || {
            let _ = wakes.send(Wake::Network);
        }),
        proxy,
        allowed_local,
    ))
}

/// Writes `message` to the trusted core.
fn send(output: &cookies::Output, message: &FromFetcher) -> Result<(), String> {
    write_to_core(&mut *output.borrow_mut(), message).map_err(|error| error.to_string())
}

/// Gives the agent what each socket read or how it ended; true when one
/// had something.
fn poll_sockets(pump: &mut Pump<Tcp>, buffer: &mut [u8], agent: &mut Agent<'_>, clock: &dyn Fn() -> Instant, actions: &mut Vec<Action>) -> bool {
    let mut busy = false;
    for connection in [ConnectionId::Fetch, ConnectionId::Push] {
        if let Some(input) = pump.poll(connection, buffer) {
            busy = true;
            agent.handle(clock(), Input::Net(input), &mut |out| collect(out, actions));
        }
    }
    busy
}

/// Asks for the whole channel: its contents file, then every page it
/// lists; and the push connection, for all its pages.
fn ask_for_channel(agent: &mut Agent<'_>, clock: &dyn Fn() -> Instant, actions: &mut Vec<Action>) {
    agent.handle(clock(), Input::FetchContents, &mut |out| collect(out, actions));
    agent.handle(clock(), Input::Subscribe(Subscription::All), &mut |out| collect(out, actions));
}

/// The push connection opened or closed. Keys may go to the channel while
/// its WebSocket is open, if its descriptor says it takes them
/// (`interaction.md`).
fn push_changed(agent: &Agent<'_>, output: &cookies::Output, open: bool) -> Result<(), String> {
    // The trusted core keeps a sleeping tab's fetcher while it holds a
    // push connection (decision 47).
    send(output, &FromFetcher::Pushing(open))?;
    #[cfg(feature = "interaction")]
    if let Some(descriptor) = agent.descriptor().filter(|descriptor| descriptor.input()) {
        let secure = descriptor.push().is_some_and(|(url, _)| url.is_secure());
        send(output, &FromFetcher::Input { open, secure })?;
    }
    #[cfg(not(feature = "interaction"))]
    let _ = agent;
    Ok(())
}

/// Tells the trusted core what the transport refused since it was last
/// asked: a proxy's failure, a certificate, a local address.
fn tell_failures(transport: &mut Tcp, output: &cookies::Output) -> Result<(), String> {
    let send = |message: FromFetcher| write_to_core(&mut *output.borrow_mut(), &message).map_err(|error| error.to_string());
    if transport.take_proxy_failure() {
        send(FromFetcher::ProxyFailed)?;
    }
    if let Some(problem) = transport.take_certificate_refusal() {
        send(FromFetcher::CertificateRefused(problem))?;
    }
    // The agent hears only that the connection failed; the console hears
    // why, as it heard the engine's own refusal of a local peer.
    if transport.take_local_refusal() {
        send(FromFetcher::Notice(LOCAL_REFUSED.to_owned()))?;
    }
    Ok(())
}

/// What the trusted core asked of the channel, given to the agent: the
/// trusted core checked the page and the sub-code (decision 112), and the
/// agent queues and spaces the requests at its own rate.
fn fetch(agent: &mut Agent<'_>, clock: &dyn Fn() -> Instant, asked: &ToFetcher, actions: &mut Vec<Action>) {
    let input = match *asked {
        ToFetcher::FetchSubPages { page } => PageAddress::from_mpp(page).map(|page| Input::FetchSubPages { page }),
        ToFetcher::FetchSubPage { page, subcode } => PageAddress::from_mpp(page).map(|page| Input::FetchPage { page, subcode: Some(SubCode::new(subcode)) }),
        _ => None,
    };
    if let Some(input) = input {
        agent.handle(clock(), input, &mut |out| collect(out, actions));
    }
}

/// Gives the agent the input messages of one moment, in order: each but
/// the last says more follow, and the last too when `more_after`, another
/// moment's being already here, so the agent writes them all in one send
/// (REQ-RCH-010). Nothing waits for a message that is not here: the agent
/// writes what it holds when the last is handled.
#[cfg(feature = "interaction")]
fn give_records(records: &[Record], more_after: bool, handle: &mut dyn FnMut(Input<'_>)) {
    let last = records.len().saturating_sub(1);
    for (index, record) in records.iter().enumerate() {
        let more = index < last || more_after;
        handle(Input::Send(InputMessage { bytes: &record.bytes, move_of: record.move_of, more }));
    }
}

/// Hands the loop the input messages of a moment read so far, if any;
/// false when the loop has gone.
#[cfg(feature = "interaction")]
fn hand_moment(commands: &mpsc::Sender<Wake>, moment: &mut Vec<Record>) -> bool {
    moment.is_empty() || commands.send(Wake::Records(std::mem::take(moment))).is_ok()
}

/// Reads the trusted core's messages until its pipe closes or breaks the
/// protocol: input messages and requests wake the loop, and cookie answers
/// go to the store waiting for them. An input message the trusted core
/// says more follow at once waits here for them (REQ-RCH-010): it writes
/// a moment's messages in one write, so they are already on the pipe; any
/// other message, the bound [`MOMENT_MESSAGES`] or the pipe's end hands on
/// what was read.
fn read_commands(input: &mut dyn Read, commands: &mpsc::Sender<Wake>, answers: &mpsc::Sender<ToFetcher>) {
    #[cfg(feature = "interaction")]
    let mut moment: Vec<Record> = Vec::new();
    while let Ok(Some(message)) = read_to_fetcher(input) {
        #[cfg(feature = "interaction")]
        if !matches!(message, ToFetcher::Send { .. }) && !hand_moment(commands, &mut moment) {
            return;
        }
        let delivered = match message {
            #[cfg(feature = "interaction")]
            ToFetcher::Send { bytes, move_of, more } => {
                moment.push(Record { bytes, move_of });
                (more && moment.len() < MOMENT_MESSAGES) || hand_moment(commands, &mut moment)
            }
            #[cfg(feature = "interaction")]
            ToFetcher::MeasureRoundTrip(on) => commands.send(Wake::RoundTrip(on)).is_ok(),
            answer @ (ToFetcher::CookieAnswer { .. } | ToFetcher::ReachedAnswer { .. }) => answers.send(answer).is_ok(),
            asked @ (ToFetcher::FetchSubPages { .. } | ToFetcher::FetchSubPage { .. }) => commands.send(Wake::Fetch(asked)).is_ok(),
        };
        if !delivered {
            return;
        }
    }
    #[cfg(feature = "interaction")]
    hand_moment(commands, &mut moment);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Finding F-74: the largest page, bundle or form answer the agent
    /// accepts is one message to the trusted core. Before this the agent
    /// took a body of its default limit, 262,144 bytes, which with the
    /// origin byte is one byte too many for a message, so writing it
    /// failed and ended the channel's fetcher.
    #[test]
    fn the_largest_page_the_agent_accepts_reaches_the_core() {
        let config = agent_config(&[], false);
        let page = FromFetcher::Page { origin: Origin::FetchedPage, bytes: vec![b'x'; config.page_limit] };
        let mut written = Vec::new();
        write_to_core(&mut written, &page).unwrap();
        let read = protocol::fetcher::read_from_fetcher(&mut written.as_slice()).unwrap();
        assert!(matches!(read, Some(FromFetcher::Page { bytes, .. }) if bytes.len() == config.page_limit));
    }

    /// Decision 119 and decision 112 item 3 (the engine's REQ-WEB-034): a
    /// catalogue of the standard's 8,190 sub-pages reaches the trusted core
    /// whole, in order, in parts each one message; one the engine's reader
    /// refuses whole goes as a catalogue of none.
    #[test]
    fn a_catalogue_of_8190_sub_pages_goes_in_parts() {
        use neoteletext_core::limits::{MAX_SUBPAGES, from_rank};
        use std::fmt::Write as _;
        let most = u16::try_from(MAX_SUBPAGES).unwrap();
        let mut file = String::new();
        for rank in 1..=most {
            let subcode = from_rank(rank);
            let _ = write!(file, "{subcode:04X}\t{rank:016x}\tStory {rank}\r\n");
        }
        let parts = catalogue_parts(file.as_bytes());
        assert_eq!(parts.len(), MAX_SUBPAGES.div_ceil(CATALOGUE_PART));
        let mut next = 0_u16;
        for part in &parts {
            assert_eq!((part.count, part.first), (most, next), "in order");
            let mut written = Vec::new();
            write_to_core(&mut written, &FromFetcher::SubPages { page: 0x210, part: part.clone() }).unwrap();
            next += u16::try_from(part.rows.len()).unwrap();
        }
        assert_eq!(next, most, "every entry");
        assert_eq!(parts[0].rows[0], (from_rank(1), String::from("Story 1")));
        assert_eq!(catalogue_parts(&vec![b'x'; subpages::MAX_BYTES + 1]), [SubPageRows::default()], "refused whole");
        let mut kept = Catalogues::default();
        for page in 0..=u16::try_from(CATALOGUES_KEPT).unwrap() {
            kept.keep(PageAddress::from_mpp(0x100 + page).unwrap(), vec![1]);
        }
        assert!(kept.get(PageAddress::from_mpp(0x100).unwrap()).is_none(), "the oldest forgotten");
        assert!(kept.get(PageAddress::from_mpp(0x101).unwrap()).is_some());
    }

    /// Decision 62: the agent the fetcher builds asks its store for each
    /// request's cookies, which it does only with the engine's `cookies`
    /// feature compiled in; without it no cookie is ever kept or sent.
    #[test]
    fn the_agent_asks_for_each_requests_cookies() {
        #[derive(Debug, Default)]
        struct Asked(Vec<String>);
        impl neoteletext_web::cookie::Cookies for Asked {
            fn header_for(
                &mut self,
                host: &str,
                path: &str,
                _: bool,
                _: neoteletext_web::cookie::Site,
                _: u64,
                _: &mut neoteletext_web::bytes::Writer<'_>,
            ) -> (usize, bool) {
                self.0.push(format!("{host}{path}"));
                (0, false)
            }
            fn store_within(
                &mut self,
                _: &[u8],
                _: &str,
                _: &str,
                _: Option<&str>,
                _: bool,
                _: u64,
                _: bool,
            ) -> Result<(), neoteletext_web::cookie::CookieRejected> {
                Ok(())
            }
            fn reached_securely(&self, _: &str, _: u64) -> bool {
                false
            }
            fn clear_host(&mut self, _: &str) {}
            fn wipe(&mut self) {}
        }
        let mut storage = Box::new(Storage::default());
        let mut asked = Asked::default();
        let config = Config { tls_available: true, redirects_within_channel: true, ..Config::default() };
        let random = Random::new();
        let mut agent: Box<UserAgent<'_, Random, COOKIE_SLOTS, COOKIE_BYTES>> =
            Box::new(UserAgent::with_cookies(config, storage.buffers(), random, &mut asked));
        let mut actions = Vec::new();
        agent.handle(Instant(0), Input::OpenChannel("http://a.example/tt/"), &mut |out| collect(out, &mut actions));
        let connected = neoteletext_web::port::NetInput::Connected(ConnectionId::Fetch, neoteletext_web::url::IpAddress::V4([93, 184, 216, 34]));
        agent.handle(Instant(0), Input::Net(connected), &mut |out| collect(out, &mut actions));
        drop(agent);
        assert_eq!(asked.0, ["a.example/tt/channel.json"]);
    }

    /// A public address for the test's channel, which the agent may reach.
    #[cfg(feature = "interaction")]
    const PUBLIC: neoteletext_web::url::IpAddress = neoteletext_web::url::IpAddress::V4([93, 184, 216, 34]);

    /// A MOUSE move to the dot `x`, and a KEY press of `a`, as the
    /// engine's writer writes them; the agent does not read them.
    #[cfg(feature = "interaction")]
    fn mouse_move(x: u8) -> Vec<u8> {
        vec![0xFF, 0x02, 0x06, 0x00, 0x01, 0x05, x, 0x64]
    }
    #[cfg(feature = "interaction")]
    const KEY: [u8; 6] = [0xFF, 0x01, 0x10, b'a', 1, b'a'];

    /// The frames of one send on the push WebSocket, unmasked: opcode and
    /// payload each (RFC 6455 section 5.2; short frames only).
    #[cfg(feature = "interaction")]
    #[allow(clippy::arithmetic_side_effects, reason = "a test's frames, short and well formed")]
    fn client_frames(mut bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            let (length, offset) = match bytes[1] & 0x7F {
                126 => (usize::from(u16::from_be_bytes([bytes[2], bytes[3]])), 4),
                length => (usize::from(length), 2),
            };
            let mask = &bytes[offset..offset + 4];
            let payload = bytes[offset + 4..offset + 4 + length].iter().zip(mask.iter().cycle()).map(|(byte, key)| byte ^ key).collect();
            frames.push((bytes[0] & 0x0F, payload));
            bytes = &bytes[offset + 4 + length..];
        }
        frames
    }

    /// What the agent sent on the push connection, each send as its
    /// frames, taken from `actions`.
    #[cfg(feature = "interaction")]
    fn push_sends(actions: &mut Vec<Action>) -> Vec<Vec<(u8, Vec<u8>)>> {
        std::mem::take(actions)
            .into_iter()
            .filter_map(|action| match action {
                Action::Send(ConnectionId::Push, bytes) => Some(client_frames(&bytes)),
                _ => None,
            })
            .collect()
    }

    /// Opens the channel `http://tt.example/tt/`, which takes
    /// input, and its push WebSocket, as the loop would from what its
    /// sockets give, at `now` microseconds and on.
    #[cfg(feature = "interaction")]
    fn open_push(agent: &mut Agent<'_>, now: u64) {
        use neoteletext_web::port::NetInput;
        let mut actions = Vec::new();
        let handle = |agent: &mut Agent<'_>, input: Input<'_>, actions: &mut Vec<Action>| agent.handle(Instant(now), input, &mut |out| collect(out, actions));
        handle(agent, Input::OpenChannel("http://tt.example/tt/"), &mut actions);
        handle(agent, Input::Net(NetInput::Connected(ConnectionId::Fetch, PUBLIC)), &mut actions);
        let body = br#"{"name":"Test","push":{"address":"ws://tt.example/tt/push","method":"websocket","input":true}}"#;
        let mut response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        response.extend_from_slice(body);
        handle(agent, Input::Net(NetInput::Received(ConnectionId::Fetch, &response)), &mut actions);
        assert!(actions.iter().any(|action| matches!(action, Action::Ready)), "the channel is ready");
        ask_for_channel(agent, &|| Instant(now), &mut actions);
        assert!(actions.iter().any(|action| matches!(action, Action::Connect { connection: ConnectionId::Push, .. })), "the push connection opens");
        handle(agent, Input::Net(NetInput::Connected(ConnectionId::Push, PUBLIC)), &mut actions);
        let request = actions
            .iter()
            .rev()
            .find_map(|action| match action {
                Action::Send(ConnectionId::Push, bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
                _ => None,
            })
            .expect("the upgrade request");
        let key = request.lines().find_map(|line| line.strip_prefix("Sec-WebSocket-Key: ")).expect("a key").trim().to_owned();
        let accept = neoteletext_web::ws::accept_for(key.as_bytes());
        let upgrade = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            std::str::from_utf8(&accept).unwrap()
        );
        handle(agent, Input::Net(NetInput::Received(ConnectionId::Push, upgrade.as_bytes())), &mut actions);
        assert!(actions.iter().any(|action| matches!(action, Action::Push(true))), "the push WebSocket is open");
    }

    /// The agent the fetcher builds, over `storage`.
    #[cfg(feature = "interaction")]
    fn agent(storage: &mut Storage) -> Agent<'_> {
        UserAgent::new(agent_config(&[], false), storage.buffers(), Random::new())
    }

    /// The wakes `read_commands` hands the loop for `messages` on the pipe.
    #[cfg(feature = "interaction")]
    fn wakes_for(messages: &[ToFetcher]) -> Vec<Wake> {
        let mut pipe = Vec::new();
        for message in messages {
            protocol::fetcher::write_to_fetcher(&mut pipe, message).unwrap();
        }
        let (commands, woken) = mpsc::channel();
        let (answers, _) = mpsc::channel();
        read_commands(&mut pipe.as_slice(), &commands, &answers);
        drop(commands);
        woken.into_iter().collect()
    }

    /// The bytes and contacts of a wake of input messages.
    #[cfg(feature = "interaction")]
    fn records(wake: &Wake) -> Vec<(Vec<u8>, Option<u8>)> {
        match wake {
            Wake::Records(records) => records.iter().map(|record| (record.bytes.clone(), record.move_of)).collect(),
            _ => panic!("not input messages"),
        }
    }

    /// REQ-RCH-010 (requirement 18.30): the input messages the trusted
    /// core writes at one moment reach the loop together, in order, and
    /// leave in one write; a message alone goes at once; any other message
    /// or the pipe's end hands on what was read; a moment is bounded.
    #[cfg(feature = "interaction")]
    #[test]
    fn the_messages_of_one_moment_reach_the_agent_together_and_leave_in_one_write() {
        let send = |bytes: &[u8], move_of: Option<u8>, more: bool| ToFetcher::Send { bytes: bytes.to_vec(), move_of, more };
        let wakes = wakes_for(&[
            send(&mouse_move(1), Some(0), true),
            send(&KEY, None, false),
            send(&KEY, None, false),
            send(&KEY, None, true),
            ToFetcher::MeasureRoundTrip(true),
            send(&mouse_move(2), Some(0), true),
        ]);
        assert_eq!(wakes.len(), 5);
        assert_eq!(records(&wakes[0]), [(mouse_move(1), Some(0)), (KEY.to_vec(), None)], "one moment, in order");
        assert_eq!(records(&wakes[1]), [(KEY.to_vec(), None)], "a message alone, at once");
        assert_eq!(records(&wakes[2]), [(KEY.to_vec(), None)], "another message hands on the moment");
        assert!(matches!(wakes[3], Wake::RoundTrip(true)));
        assert_eq!(records(&wakes[4]), [(mouse_move(2), Some(0))], "the pipe's end hands on the moment");
        let many: Vec<ToFetcher> = (0..=MOMENT_MESSAGES).map(|_| send(&KEY, None, true)).collect();
        let wakes = wakes_for(&many);
        assert_eq!(wakes.iter().map(|wake| records(wake).len()).collect::<Vec<_>>(), [MOMENT_MESSAGES, 1], "bounded");

        let mut storage = Box::new(Storage::default());
        let mut agent = agent(&mut storage);
        open_push(&mut agent, 0);
        let mut actions = Vec::new();
        let mut handle = |input: Input<'_>| agent.handle(Instant(0), input, &mut |out| collect(out, &mut actions));
        let moment = [Record { bytes: mouse_move(1), move_of: Some(0) }, Record { bytes: KEY.to_vec(), move_of: None }];
        give_records(&moment, false, &mut handle);
        give_records(&[Record { bytes: KEY.to_vec(), move_of: None }], true, &mut handle);
        give_records(&[Record { bytes: mouse_move(3), move_of: Some(0) }], false, &mut handle);
        give_records(&[Record { bytes: KEY.to_vec(), move_of: None }], false, &mut handle);
        let binary = |bytes: &[u8]| (neoteletext_web::consts::OPCODE_BINARY, bytes.to_vec());
        assert_eq!(
            push_sends(&mut actions),
            [vec![binary(&mouse_move(1)), binary(&KEY)], vec![binary(&KEY), binary(&mouse_move(3))], vec![binary(&KEY)]],
            "a moment in one send, in order; one already here joins it; a lone message at once"
        );
    }

    /// REQ-RCH-016 (requirement 18.32): asked to, the agent measures the
    /// push WebSocket's round trip by a ping of one byte, and its pong goes
    /// to the trusted core as the round trip, in microseconds.
    #[cfg(feature = "interaction")]
    #[test]
    fn the_round_trip_is_measured_and_told_to_the_core() {
        let mut storage = Box::new(Storage::default());
        let mut agent = agent(&mut storage);
        open_push(&mut agent, 0);
        let mut actions = Vec::new();
        agent.handle(Instant(1_000), Input::MeasureRoundTrip(true), &mut |out| collect(out, &mut actions));
        agent.handle(Instant(1_000), Input::Tick, &mut |out| collect(out, &mut actions));
        let ping = push_sends(&mut actions).concat().into_iter().find(|(opcode, _)| *opcode == neoteletext_web::consts::OPCODE_PING).expect("a ping");
        assert_eq!(ping.1.len(), 1, "one byte, told from the heartbeat's");
        let answer = [0x80 | neoteletext_web::consts::OPCODE_PONG, 1, ping.1[0]];
        let answered = neoteletext_web::port::NetInput::Received(ConnectionId::Push, &answer);
        agent.handle(Instant(32_250), Input::Net(answered), &mut |out| collect(out, &mut actions));
        let told: Vec<FromFetcher> = actions
            .into_iter()
            .filter_map(|action| match action {
                Action::Tell(message @ FromFetcher::RoundTrip { .. }) => Some(message),
                _ => None,
            })
            .collect();
        assert_eq!(told, [FromFetcher::RoundTrip { microseconds: 31_250 }]);
    }
}

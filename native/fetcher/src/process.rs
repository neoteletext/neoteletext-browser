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
use neoteletext_transport::web_address;
#[cfg(feature = "interaction")]
use neoteletext_web::agent::InputMessage;
use neoteletext_web::agent::{Config, Event, Failure, Input, Output, Source, Storage, Subscription, UserAgent};
use neoteletext_web::channel::SETTINGS_FILE;
use neoteletext_web::consts::ALLOWED_LOCAL;
use neoteletext_web::contents::{self, Entry as ContentsEntry};
use neoteletext_web::http::MediaType;
use neoteletext_web::port::{ConnectionId, NetOutput, Pump, Security, Transport};
use neoteletext_web::proxy::Proxy;
use neoteletext_web::records::redirect::RedirectRecord;
use neoteletext_web::subpages::{self, PartNumber};
use protocol::fetcher::{Entry, FromFetcher, Origin, RefusedFile, ToFetcher, read_to_fetcher, redirect_in_bounds, write_to_core};
use protocol::subpages::SubPageRows;

#[cfg(feature = "links")]
use crate::agent::descriptor_links;
use crate::agent::{Random, product};
use crate::cookies;
use crate::live::{LiveIndex, Noted, SubPageLists};
use crate::transport::{AllowedLocal, Tcp};

const _: () = assert!(protocol::fetcher::MAX_DENIED_PLACE == neoteletext_web::records::denied::MAX_PLACE, "the engine's longest place");
const _: () = assert!(protocol::fetcher::MIN_REFRESH_SECONDS == neoteletext_web::descriptor::MIN_REFRESH_SECONDS, "the engine's shortest refresh");
const _: () = assert!(protocol::fetcher::MAX_REFRESH_SECONDS == neoteletext_web::descriptor::MAX_REFRESH_SECONDS, "the engine's longest refresh");

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
/// The parts of lists of sub-pages kept as read, so that one the agent says
/// is current (304, or fresh by its lifetime) is sent again without a
/// request: two whole lists of the standard's 8,190, or one part of each
/// of 256 pages, at most a megabyte at the engine's 4,119 bytes a part.
const PARTS_KEPT: usize = 2 * subpages::MAX_PARTS;
// A list's rows are the engine's, within the standard's maximum, in the
// protocol's parts, and its titles a row's, within a page title's bound on
// the wire.
const _: () = assert!(subpages::MAX_ENTRIES == neoteletext_core::limits::MAX_SUBPAGES);
const _: () = assert!(subpages::MAX_TITLE * 4 <= protocol::MAX_PAGE_TITLE);
const _: () = assert!(subpages::ROWS_PER_PART == protocol::fetcher::ROWS_PER_PART, "the engine's rows of a part");
const _: () = assert!(subpages::MAX_PARTS == protocol::fetcher::MAX_LIST_PARTS, "the engine's parts of a list");
const _: () = assert!(subpages::MAX_VALIDATOR == protocol::fetcher::MAX_LIST_VERSION, "the engine's longest version");
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

/// How often the last writes are looked at.
const LAST_WRITE_POLL: Duration = Duration::from_millis(1);

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
    Ready(u16, Option<u8>),
    Contents(Vec<u8>),
    /// A part of a page's list of sub-pages, as fetched.
    SubPages(PageAddress, PartNumber, Vec<u8>),
    /// A part of a page's list is current: the one kept is sent again.
    SubPagesCurrent(PageAddress, PartNumber),
    /// The page asked for has been answered, one way or another.
    Answered,
    /// A file was refused, for the trusted core to say (REQ-708).
    Refused(RefusedFile, Answer),
    /// A message for the trusted core as it is: a notice, or an exchange
    /// for the developer console (the engine's REQ-WEB-033).
    Tell(FromFetcher),
    /// The push connection opened or was lost.
    Push(bool),
    /// A part of a live channel's index: its magazine, the magazine's
    /// name, its pages, and the sending's generation.
    Index {
        magazine: u8,
        name: String,
        pages: Vec<(u16, String, String)>,
        generation: u16,
    },
    /// Every part of an index sending arrived.
    IndexComplete(u16),
    /// A part of a page's list of sub-pages from a SUBPAGES record.
    SubPageList(SubPageListPart),
}

/// What came of a file that was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    /// This status.
    Status(u16),
    /// A response whose content was refused: its status is the exchange's
    /// that came before.
    Content,
    /// None came.
    Nothing,
}

impl Answer {
    fn of(failure: Failure) -> Self {
        match failure {
            Failure::Status(status) => Self::Status(status),
            Failure::Content | Failure::MediaType => Self::Content,
            _ => Self::Nothing,
        }
    }

    /// The status to say, `last` being the last response's.
    const fn status(self, last: Option<u16>) -> Option<u16> {
        match self {
            Self::Status(status) => Some(status),
            Self::Content => last,
            Self::Nothing => None,
        }
    }
}

/// The status of the 404 that says a page is not in the service.
const NOT_FOUND: u16 = 404;

/// The refusal an event says, if it says one of a whole file: a sub-page's
/// file is another page's business, and stored pages are not disturbed.
fn refusal(event: &Event<'_>) -> Option<(RefusedFile, Answer)> {
    Some(match *event {
        Event::DescriptorFailed(failure) => (RefusedFile::Channel, Answer::of(failure)),
        Event::DescriptorRejected(_) => (RefusedFile::Channel, Answer::Content),
        Event::BundleFailed(failure) => (RefusedFile::Bundle, Answer::of(failure)),
        Event::ContentsFailed(failure) => (RefusedFile::Contents, Answer::of(failure)),
        Event::PageNotInService { page, subcode: None } => (RefusedFile::Page(page.mpp()), Answer::Status(NOT_FOUND)),
        Event::FetchFailed { page, subcode: None, failure } => (RefusedFile::Page(page.mpp()), Answer::of(failure)),
        _ => return None,
    })
}

/// A SUBPAGES record, owned (DOC-1959).
struct SubPageListPart {
    page: PageAddress,
    version: u16,
    part: u16,
    parts: u16,
    count: u16,
    entries: Vec<(u16, String)>,
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
            (Source::Push, _) => Origin::PushedPackets,
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
    // Said to the trusted core by its role and status, and a page's ends the
    // wait for the next one as well.
    if let Output::Event(event) = output
        && let Some((file, answer)) = refusal(&event)
    {
        actions.push(Action::Refused(file, answer));
        match file {
            RefusedFile::Page(_) => actions.push(Action::Answered),
            RefusedFile::Contents => actions.push(Action::Tell(FromFetcher::NoContents)),
            RefusedFile::Channel | RefusedFile::Bundle => {}
        }
        return;
    }
    // It ended for good and is not tried again (DOC-1920).
    if let Output::Event(Event::PushStopped(stop)) = output {
        eprintln!("neoteletext-fetcher: the push connection stopped ({stop:?}); it is not tried again");
        actions.push(Action::Tell(FromFetcher::Notice(format!("the push connection stopped: {stop:?}"))));
        actions.push(Action::Push(false));
        return;
    }
    actions.push(match output {
        Output::Timer { at } => Action::Timer(at),
        Output::Event(Event::ServiceReady { initial_page, region, .. }) => Action::Ready(initial_page.mpp(), region),
        Output::Event(Event::Contents(bytes)) => Action::Contents(bytes.to_vec()),
        Output::Event(Event::ContentsMissing) => Action::Tell(FromFetcher::NoContents),
        Output::Event(Event::PageCurrent { .. } | Event::PageNotInService { .. } | Event::FetchFailed { .. }) => Action::Answered,
        Output::Event(Event::PushConnected) => Action::Push(true),
        Output::Event(Event::Denied(denied)) => Action::Tell(FromFetcher::Denied { status: denied.status, place: denied.place.to_owned() }),
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
        // A live channel's records (neoteletext-common's DEC-1459): its
        // author's log lines, its invalid packets, its opening, its index
        // and its pages' lists of sub-pages.
        Output::Event(Event::Log { line, page }) => Action::Tell(FromFetcher::ChannelLog {
            level: match line.level {
                neoteletext_formats::log::LogLevel::Error => protocol::console::Level::Error,
                neoteletext_formats::log::LogLevel::Warning => protocol::console::Level::Warning,
                neoteletext_formats::log::LogLevel::Information => protocol::console::Level::Info,
            },
            page: page.map(PageAddress::mpp),
            text: line.text.to_owned(),
        }),
        Output::Event(Event::InvalidPackets { total }) => Action::Tell(FromFetcher::InvalidPackets { total }),
        Output::Event(Event::ChannelConnecting) => Action::Tell(FromFetcher::Connecting(true)),
        Output::Event(Event::ChannelLive) => Action::Tell(FromFetcher::Connecting(false)),
        Output::Event(Event::Index(record)) => {
            let pages = record.pages().map(|page| (page.page, page.title.to_owned(), page.section.to_owned())).collect();
            Action::Index { magazine: record.magazine, name: record.name.to_owned(), pages, generation: record.generation }
        }
        Output::Event(Event::IndexComplete { generation }) => Action::IndexComplete(generation),
        Output::Event(Event::SubPageList(record)) => Action::SubPageList(SubPageListPart {
            page: record.page,
            version: record.version,
            part: record.part,
            parts: record.parts,
            count: record.count,
            entries: record.entries().map(|entry| (entry.subcode.value(), entry.title.to_owned())).collect(),
        }),
        // The trusted core decides: a channel moved to another folder
        // redirects its settings file, so the destination named is the
        // folder, the channel itself (REQ-701).
        Output::Event(Event::RedirectToOtherChannel { location }) => {
            let folder = location.strip_suffix(SETTINGS_FILE).filter(|folder| folder.ends_with('/'));
            Action::Tell(FromFetcher::Redirected(folder.unwrap_or(location).to_owned()))
        }
        // A channel's program moved the viewer (DOC-1960): the
        // trusted core decides, and an address goes by its own rules.
        Output::Event(Event::Redirect(RedirectRecord::Page { page, subcode, parameters })) if redirect_in_bounds(page.mpp(), subcode.value()) => {
            Action::Tell(FromFetcher::PageRedirect { page: page.mpp(), sub: subcode.value(), query: parameters.map(str::to_owned) })
        }
        Output::Event(Event::Redirect(RedirectRecord::Address(address))) => Action::Tell(FromFetcher::ProgramRedirect(address.to_owned())),
        Output::Event(event) => list_action(&event).unwrap_or_else(|| Action::Tell(FromFetcher::Notice(format!("{event:?}")))),
        _ => return,
    });
}

/// What a part of a page's list of sub-pages asks of the loop (REQ-1888).
fn list_action(event: &Event<'_>) -> Option<Action> {
    Some(match *event {
        Event::SubPages { page, part, bytes } => Action::SubPages(page, part, bytes.to_vec()),
        Event::SubPagesCurrent { page, part } => Action::SubPagesCurrent(page, part),
        Event::SubPagesMissing { page, part } => Action::Tell(FromFetcher::SubPagesMissing { page: page.mpp(), part: part.number() }),
        Event::SubPagesFailed { page, part, failure } => {
            eprintln!("neoteletext-fetcher: part {} of the list of sub-pages of page {:03X} failed ({failure:?})", part.number(), page.mpp());
            Action::Tell(FromFetcher::SubPagesFailed { page: page.mpp(), part: part.number() })
        }
        _ => return None,
    })
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

/// A part of a page's list of sub-pages, `NNN/P.sub`, read by the engine's
/// bounded reader (REQ-1888), as the trusted core takes it: the page's
/// version, and its number of sub-pages, the part's first place and every
/// row the reader keeps, which are at most a part's and ascend. None when
/// the reader refused it whole, or it lies past the page's count.
fn list_part(part: PartNumber, bytes: &[u8]) -> Option<(String, SubPageRows)> {
    let list = subpages::read(bytes).ok()?;
    let first = u16::try_from(part.first_place()).ok().filter(|&first| first < list.total)?;
    let rows: Vec<(u16, String)> = list.entries.flatten().map(|entry| (entry.subcode.value(), entry.title.unwrap_or_default().to_owned())).collect();
    let fits = usize::from(first).saturating_add(rows.len()) <= usize::from(list.total);
    fits.then(|| (list.version.as_str().to_owned(), SubPageRows { count: list.total, first, rows }))
}

/// The parts of lists of sub-pages last read, the most recent last.
#[derive(Default)]
struct Parts(VecDeque<((PageAddress, PartNumber), Vec<u8>)>);

impl Parts {
    /// Keeps `bytes` as `page`'s part `part`, forgetting the oldest past
    /// [`PARTS_KEPT`].
    fn keep(&mut self, page: PageAddress, part: PartNumber, bytes: Vec<u8>) {
        self.0.retain(|(kept, _)| *kept != (page, part));
        self.0.push_back(((page, part), bytes));
        if self.0.len() > PARTS_KEPT {
            self.0.pop_front();
        }
    }

    /// The part kept of `page`.
    fn get(&self, page: PageAddress, part: PartNumber) -> Option<&[u8]> {
        self.0.iter().find(|(kept, _)| *kept == (page, part)).map(|(_, bytes)| bytes.as_slice())
    }
}

/// Sends part `part` of `page`'s list to the trusted core, or that it could
/// not be used.
fn send_part(output: &cookies::Output, page: PageAddress, part: PartNumber, bytes: Option<&[u8]>) -> Result<(), String> {
    match bytes.and_then(|bytes| list_part(part, bytes)) {
        Some((version, rows)) => send(output, &FromFetcher::SubPages { page: page.mpp(), version, part: rows }),
        None => send(output, &FromFetcher::SubPagesFailed { page: page.mpp(), part: part.number() }),
    }
}

/// The agent's settings for a channel that may reach the local addresses
/// `allowed`, through a proxy when `proxied`.
fn agent_config(allowed: &[IpAddr], proxied: bool) -> Config {
    // Proxied, local names are refused before the proxy is asked (REQ-WEB-032).
    let mut config = Config { tls_available: true, redirects_within_channel: true, product: product(), proxied, page_limit: PAGE_LIMIT, ..Config::default() };
    for (slot, &ip) in config.allowed_local.iter_mut().zip(allowed.iter().take(ALLOWED_LOCAL)) {
        *slot = Some(web_address(ip));
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
    let mut parts = Parts::default();
    let mut live = Live::default();
    // The status of the last response, for a refusal of its content.
    let mut last_status = None;

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
                Action::Ready(page, region) => {
                    service_ready(&agent, &output, page, region)?;
                    ask_for_channel(&mut agent, &clock, &mut actions);
                }
                Action::Contents(bytes) => {
                    let (entries, pages) = contents_entries(&bytes);
                    send(&output, &FromFetcher::Contents(entries))?;
                    queue.extend(pages);
                }
                Action::SubPages(page, part, bytes) => {
                    send_part(&output, page, part, Some(&bytes))?;
                    parts.keep(page, part, bytes);
                }
                // A part no longer kept, though the agent holds its
                // validators, is said to have failed: the trusted core asks
                // for it again when its page next opens.
                Action::SubPagesCurrent(page, part) => send_part(&output, page, part, parts.get(page, part))?,
                Action::Answered => waiting = false,
                Action::Refused(file, answer) => send(&output, &FromFetcher::Refused { file, status: answer.status(last_status) })?,
                Action::Push(open) => push_changed(&agent, &output, open)?,
                action @ (Action::Index { .. } | Action::IndexComplete(_) | Action::SubPageList(_)) => live_record(action, &mut live, &output)?,
                Action::Tell(message) => {
                    last_status = if let FromFetcher::Exchange { status, .. } = message { Some(status) } else { last_status };
                    send(&output, &message)?;
                }
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
        let mut ended = false;
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
                Wake::Ended => ended = true,
            }
        }
        if ended {
            send_last(&mut pump, actions);
            return Ok(());
        }
    }
}

/// What the trusted core's last messages produced leaves before the
/// fetcher ends, so a channel is told a tab left (SELECT of page 0,
/// neoteletext-common's DEC-1538 item 3). A network that does not take it
/// in [`protocol::fetcher::LAST_WRITES_WAIT`] holds nothing up: the channel sees the
/// connection close, which says as much.
fn send_last(pump: &mut Pump<Tcp>, actions: Vec<Action>) {
    for action in actions {
        if let Action::Send(connection, bytes) = action {
            pump.execute(NetOutput::Send(connection, &bytes));
        }
    }
    let begun = Clock::now();
    while pump.transport.unsent(ConnectionId::Push) > 0 && begun.elapsed() < protocol::fetcher::LAST_WRITES_WAIT {
        std::thread::sleep(LAST_WRITE_POLL);
    }
}

/// What a live channel's records have given so far.
#[derive(Default)]
struct Live {
    index: LiveIndex,
    lists: SubPageLists,
}

/// A live channel's index record or SUBPAGES record: held until whole,
/// then sent on as the contents and the catalogue a channel served as files
/// gives.
fn live_record(action: Action, live: &mut Live, output: &cookies::Output) -> Result<(), String> {
    match action {
        Action::Index { magazine, name, pages, generation } => {
            live.index.note(magazine, &name, &pages, generation);
            send(output, &FromFetcher::Contents(live.index.entries()))
        }
        Action::IndexComplete(generation) => {
            live.index.complete(generation);
            send(output, &FromFetcher::Contents(live.index.entries()))
        }
        Action::SubPageList(part) => match live.lists.note(part.page.mpp(), part.version, part.part, part.parts, part.count, part.entries) {
            Noted::Partial => Ok(()),
            Noted::Whole(page, version, parts) => {
                parts.into_iter().try_for_each(|part| send(output, &FromFetcher::SubPages { page, version: version.clone(), part }))
            }
            // Said where a channel's other faults are, the console; the
            // engine counted the records it refused alone.
            Noted::Refused(page, why) => {
                eprintln!("neoteletext-fetcher: the list of sub-pages of page {page:03X} broke its totals ({why:?}); discarded");
                send(output, &FromFetcher::Notice(format!("the list of sub-pages of page {page:03X} broke its totals ({why:?}); discarded")))
            }
        },
        _ => Ok(()),
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

/// Tells the trusted core what the open channel's descriptor says.
fn service_ready(agent: &Agent<'_>, output: &cookies::Output, page: u16, region: Option<u8>) -> Result<(), String> {
    // Before any page, so that none is read in another group.
    send(output, &FromFetcher::Region(region))?;
    send(output, &FromFetcher::InitialPage(page))?;
    // The descriptor's links, for the trusted core to decide
    // (`hyperlinks.md` section 6.4).
    #[cfg(feature = "links")]
    if let Some(links) = descriptor_links(agent) {
        send(output, &links)?;
    }
    // The interval for asking again for the page on screen, when no push
    // connection brings its changes (REQ-704).
    if let Some(seconds) = agent.descriptor().filter(|descriptor| descriptor.push().is_none()).and_then(|descriptor| descriptor.refresh_seconds) {
        send(output, &FromFetcher::Refresh(seconds))?;
    }
    Ok(())
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
        let secure = descriptor.push().is_some_and(neoteletext_web::url::Url::is_secure);
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
        ToFetcher::FetchPage { page } => PageAddress::from_mpp(page).map(|page| Input::FetchPage { page, subcode: None }),
        ToFetcher::FetchSubPages { page, part } => {
            PageAddress::from_mpp(page).zip(PartNumber::new(usize::from(part))).map(|(page, part)| Input::FetchSubPages { page, part })
        }
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
            asked @ (ToFetcher::FetchPage { .. } | ToFetcher::FetchSubPages { .. } | ToFetcher::FetchSubPage { .. }) => {
                commands.send(Wake::Fetch(asked)).is_ok()
            }
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
    use neoteletext_web::channel::SETTINGS_FILE;

    /// REQ-708: every failure of a whole file reaches the trusted core by
    /// its role, with the status when there was one and nothing a server
    /// wrote; a missing page is 404, and ends the wait for the next page.
    /// On the code before, a page's failures only ended that wait, and the
    /// rest were a debug line in the Log.
    #[test]
    fn a_failure_of_a_file_is_a_refusal() {
        let page = PageAddress::from_mpp(0x100).unwrap();
        let actions_of = |event: Event<'_>| {
            let mut actions = Vec::new();
            collect(Output::Event(event), &mut actions);
            actions.iter().filter_map(|action| if let Action::Refused(file, answer) = action { Some((*file, *answer)) } else { None }).collect::<Vec<_>>()
        };
        let here = RefusedFile::Page(0x100);
        assert_eq!(actions_of(Event::PageNotInService { page, subcode: None }), [(here, Answer::Status(404))]);
        assert_eq!(actions_of(Event::FetchFailed { page, subcode: None, failure: Failure::Status(500) }), [(here, Answer::Status(500))]);
        assert_eq!(actions_of(Event::FetchFailed { page, subcode: None, failure: Failure::Content }), [(here, Answer::Content)]);
        assert_eq!(actions_of(Event::FetchFailed { page, subcode: None, failure: Failure::Network }), [(here, Answer::Nothing)]);
        assert_eq!(actions_of(Event::DescriptorFailed(Failure::MediaType)), [(RefusedFile::Channel, Answer::Content)]);
        assert_eq!(actions_of(Event::BundleFailed(Failure::Status(403))), [(RefusedFile::Bundle, Answer::Status(403))]);
        assert_eq!(actions_of(Event::ContentsFailed(Failure::Timeout)), [(RefusedFile::Contents, Answer::Nothing)]);
        assert_eq!(actions_of(Event::PageNotInService { page, subcode: Some(SubCode::new(2)) }), [], "a sub-page's file is not the page's");
        let mut actions = Vec::new();
        collect(Output::Event(Event::PageNotInService { page, subcode: None }), &mut actions);
        assert!(actions.iter().any(|action| matches!(action, Action::Answered)), "the next page may go");
    }

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

    /// REQ-1888, DOC-1869: a part of a list of the standard's 8,190
    /// sub-pages, as `NNN/P.sub` holds it, reaches the trusted core as one
    /// message, at its part's first place, with the page's count and
    /// version; one the engine's reader refuses whole, or past the page's
    /// count, is a part that failed.
    #[test]
    fn a_part_of_a_list_of_8190_sub_pages_is_one_message() {
        use neoteletext_core::limits::from_rank;
        use std::fmt::Write as _;
        let part_file = |total: u16, part: usize| {
            let mut file = format!("{total}\t4be81f7a09c2d635\r\n");
            for place in part * subpages::ROWS_PER_PART..((part + 1) * subpages::ROWS_PER_PART).min(usize::from(total)) {
                let rank = u16::try_from(place + 1).unwrap();
                let _ = write!(file, "{:04X}\t{rank:016x}\tStory {rank}\r\n", from_rank(rank));
            }
            file
        };
        let last = PartNumber::new(subpages::MAX_PARTS - 1).unwrap();
        let (version, rows) = list_part(last, part_file(8_190, last.number().into()).as_bytes()).unwrap();
        assert_eq!(version, "4be81f7a09c2d635");
        assert_eq!((rows.count, rows.first, rows.rows.len()), (8_190, 8_128, 62), "the last part holds the rest");
        assert_eq!(rows.rows[61], (from_rank(8_190), String::from("Story 8190")));
        let mut written = Vec::new();
        write_to_core(&mut written, &FromFetcher::SubPages { page: 0x210, version, part: rows }).unwrap();
        let second = PartNumber::new(1).unwrap();
        assert_eq!(list_part(second, part_file(8_190, 1).as_bytes()).map(|(_, rows)| (rows.first, rows.rows.len())), Some((64, 64)));
        assert_eq!(list_part(second, part_file(10, 0).as_bytes()), None, "part 1 of a page of ten");
        assert_eq!(list_part(PartNumber::FIRST, &vec![b'x'; subpages::MAX_BYTES + 1]), None, "refused whole");
        let mut kept = Parts::default();
        let page = PageAddress::from_mpp(0x100).unwrap();
        for part in 0..=PARTS_KEPT {
            kept.keep(
                PageAddress::from_mpp(0x100 + u16::try_from(part / subpages::MAX_PARTS).unwrap()).unwrap(),
                PartNumber::new(part % subpages::MAX_PARTS).unwrap(),
                vec![1],
            );
        }
        assert!(kept.get(page, PartNumber::FIRST).is_none(), "the oldest forgotten");
        assert!(kept.get(page, second).is_some());
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
                _: Option<&str>,
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
            fn clear_site_data(&mut self, _: &str, _: Option<&str>) {}
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
        assert_eq!(asked.0, [format!("a.example/tt/{SETTINGS_FILE}")]);
        assert_eq!(SETTINGS_FILE, "channel.ttx", "DEC-1726: documents and channels name the file literally");
    }

    /// DOC-1960: a REDIRECT record is told as it is, for the
    /// trusted core to judge. Nothing here reads the address.
    #[test]
    fn a_redirect_record_is_told() {
        let mut actions = Vec::new();
        let page = PageAddress::from_mpp(0x302).unwrap();
        collect(Output::Event(Event::Redirect(RedirectRecord::Page { page, subcode: SubCode::new(2), parameters: None })), &mut actions);
        collect(Output::Event(Event::Redirect(RedirectRecord::Address("file:///etc/passwd"))), &mut actions);
        assert!(matches!(&actions[0], Action::Tell(FromFetcher::PageRedirect { page: 0x302, sub: 2, query: None })));
        assert!(matches!(&actions[1], Action::Tell(FromFetcher::ProgramRedirect(address)) if address == "file:///etc/passwd"));
        // A page the engine reads and this browser has no slot for goes to
        // the log, as any other event: the pipe's reader would refuse it.
        let lettered = PageAddress::from_mpp(0x1A0).unwrap();
        collect(Output::Event(Event::Redirect(RedirectRecord::Page { page: lettered, subcode: SubCode::new(2), parameters: None })), &mut actions);
        assert!(matches!(&actions[2], Action::Tell(FromFetcher::Notice(_))));
    }

    /// REQ-701: a channel's settings file redirected out of the channel is
    /// not followed, and the trusted core is told the folder it leads to.
    #[test]
    fn a_redirect_out_of_the_channel_names_the_destinations_folder() {
        use neoteletext_web::port::NetInput;
        let answer = |location: &str| format!("HTTP/1.1 301 Moved Permanently\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n");
        for (location, told) in [
            (format!("https://b.example/tt/{SETTINGS_FILE}"), "https://b.example/tt/"),
            ("https://b.example/elsewhere".to_owned(), "https://b.example/elsewhere"),
            (format!("http://a.example/other/{SETTINGS_FILE}"), "http://a.example/other/"),
        ] {
            let mut storage = Box::new(Storage::default());
            let config = Config { tls_available: true, redirects_within_channel: true, ..Config::default() };
            let mut agent: Box<Agent<'_>> = Box::new(UserAgent::new(config, storage.buffers(), Random::new()));
            let mut actions = Vec::new();
            agent.handle(Instant(0), Input::OpenChannel("http://a.example/tt/"), &mut |out| collect(out, &mut actions));
            let connected = NetInput::Connected(ConnectionId::Fetch, neoteletext_web::url::IpAddress::V4([93, 184, 216, 34]));
            agent.handle(Instant(0), Input::Net(connected), &mut |out| collect(out, &mut actions));
            let reply = answer(&location);
            agent.handle(Instant(0), Input::Net(NetInput::Received(ConnectionId::Fetch, reply.as_bytes())), &mut |out| collect(out, &mut actions));
            assert!(actions.iter().any(|action| matches!(action, Action::Tell(FromFetcher::Redirected(address)) if address == told)), "{location}");
        }
    }

    /// REQ-1686 item 1: a channel's `http` address redirected to its own
    /// `https` address stays in the channel, so the trusted core hears
    /// nothing of it and no notice shows.
    #[test]
    fn a_redirect_within_the_channel_is_not_told() {
        use neoteletext_web::port::NetInput;
        let reply = format!("HTTP/1.1 301 Moved Permanently\r\nLocation: https://a.example/tt/{SETTINGS_FILE}\r\nContent-Length: 0\r\n\r\n");
        let mut storage = Box::new(Storage::default());
        let config = Config { tls_available: true, redirects_within_channel: true, ..Config::default() };
        let mut agent: Box<Agent<'_>> = Box::new(UserAgent::new(config, storage.buffers(), Random::new()));
        let mut actions = Vec::new();
        agent.handle(Instant(0), Input::OpenChannel("http://a.example/tt/"), &mut |out| collect(out, &mut actions));
        let connected = NetInput::Connected(ConnectionId::Fetch, neoteletext_web::url::IpAddress::V4([93, 184, 216, 34]));
        agent.handle(Instant(0), Input::Net(connected), &mut |out| collect(out, &mut actions));
        agent.handle(Instant(0), Input::Net(NetInput::Received(ConnectionId::Fetch, reply.as_bytes())), &mut |out| collect(out, &mut actions));
        assert!(!actions.iter().any(|action| matches!(action, Action::Tell(FromFetcher::Redirected(_)))));
    }

    /// A public address for the test's channel, which the agent may reach.
    #[cfg(feature = "interaction")]
    const PUBLIC: neoteletext_web::url::IpAddress = neoteletext_web::url::IpAddress::V4([93, 184, 216, 34]);

    /// A MOUSE move to the dot `x`, and a KEY press of `a`, as the
    /// engine's writer writes them; the agent does not read them.
    #[cfg(feature = "interaction")]
    fn mouse_move(x: u8) -> Vec<u8> {
        vec![0xFF, 0x02, 0x80, 0x04, 0x00, 0x06, 0x00, 0x01, 0x05, x, 0x64]
    }
    #[cfg(feature = "interaction")]
    const KEY: [u8; 9] = [0xFF, 0x01, 0x80, 0x04, 0x00, 0x10, b'a', 1, b'a'];

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

    /// A channel's region goes on with its first page (the engine's
    /// REQ-WEB-002).
    #[test]
    fn a_channels_region_goes_with_its_initial_page() {
        let mut actions = Vec::new();
        let initial_page = PageAddress::from_mpp(0x100).unwrap();
        collect(Output::Event(Event::ServiceReady { name: "Test", initial_page, region: Some(6), secure: true }), &mut actions);
        assert!(matches!(actions[..], [Action::Ready(0x100, Some(6))]));
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
        let body = br#"{"name":"Test","push":{"input":true}}"#;
        let mut response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        response.extend_from_slice(body);
        handle(agent, Input::Net(NetInput::Received(ConnectionId::Fetch, &response)), &mut actions);
        assert!(actions.iter().any(|action| matches!(action, Action::Ready(..))), "the channel is ready");
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
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Protocol: {}\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            neoteletext_web::consts::PUSH_SUBPROTOCOL,
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

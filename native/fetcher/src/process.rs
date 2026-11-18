//! The fetcher's loop: the agent's output turned into actions on the
//! sockets and messages for the trusted core, each file the trusted core
//! asks for as it is wanted (REQ-703), and the trusted core's messages read
//! on a thread of their own. The input messages of one moment reach the agent
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
use neoteletext_web::agent::{Config, Event, Failure, Input, InputMessage, Output, Source, Subscription, UserAgent};
use neoteletext_web::channel::{BUNDLE_FILE, SETTINGS_FILE};
use neoteletext_web::consts::{ALLOWED_LOCAL, STATUS_GONE, STATUS_NOT_FOUND};
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
use crate::live::{LiveIndex, Noted, SubPageLists};
use crate::state::State;
use crate::transport::Tcp;

const _: () = assert!(protocol::fetcher::MAX_DENIED_PLACE == neoteletext_web::records::denied::MAX_PLACE, "the engine's longest place");
const _: () = assert!(protocol::fetcher::MIN_REFRESH_SECONDS == neoteletext_web::descriptor::MIN_REFRESH_SECONDS, "the engine's shortest refresh");
const _: () = assert!(protocol::fetcher::MAX_REFRESH_SECONDS == neoteletext_web::descriptor::MAX_REFRESH_SECONDS, "the engine's longest refresh");
// A channel's name is carried whole as the tab's title (REQ-6641).
const _: () = assert!(neoteletext_web::consts::MAX_SERVICE_NAME <= protocol::MAX_TAB_TITLE, "the engine's longest service name");

/// The fetcher's standard output, or a test's pipe in its place, shared by
/// its loop and its viewer state.
pub(crate) type CorePipe = Rc<RefCell<BufWriter<Box<dyn Write>>>>;
/// What the console is told when a name led only to addresses the channel
/// may not reach.
const LOCAL_REFUSED: &str = "Blocked: the server's name leads to this computer or the local network, which this channel may not reach";
/// The longest body the agent takes for a page or a bundle: the largest
/// page file with a picture (the engine's DEC-6810). A body longer than one
/// page message to the trusted core goes in pieces of one message each, or
/// not at all (`pictured::pieces`): `write_to_core` refuses more than a
/// message, and refusing ends this fetcher (finding F-74).
const PAGE_LIMIT: usize = crate::pictured::MAX_PICTURED_PAGE_FILE;
// A pushed message goes to the trusted core as one page message.
const _: () = assert!(neoteletext_web::consts::DEFAULT_PUSH_LIMIT <= crate::pictured::PAGE_MESSAGE);
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
    /// The trusted core asks for the contents file, a page, a part of a
    /// page's list of sub-pages, or one sub-page (REQ-703, decision 112).
    Fetch(ToFetcher),
    /// The trusted core asks for an unprompted `STATE_OFFER` or `STATE_NONE`
    /// (VS-111, VS-113).
    State(ToFetcher),
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
    /// The channel is open: its first page, its region, and the name its
    /// `channel.ttx` gives (REQ-6641).
    Ready(u16, Option<u8>, String),
    /// The channel has no `channel.ttx`: the engine names it by its host
    /// and folder, which is no name the channel gave, so none is sent
    /// (REQ-6641 item 2).
    Unnamed,
    Contents(Vec<u8>),
    /// A part of a page's list of sub-pages, as fetched.
    SubPages(PageAddress, PartNumber, Vec<u8>),
    /// A part of a page's list is current: the one kept is sent again.
    SubPagesCurrent(PageAddress, PartNumber),
    /// The bundle was answered, refused or found missing: what it held
    /// has gone to the trusted core (REQ-703).
    BundleSettled,
    /// A file was refused, for the trusted core to say (REQ-708).
    Refused(RefusedFile, Answer),
    /// A message for the trusted core as it is: a notice, or an exchange
    /// for the developer console (the engine's REQ-WEB-033).
    Tell(FromFetcher),
    /// The push connection opened or was lost.
    Push(bool),
    /// The push connection carried a `STATE_CHALLENGE` (VS-204), answered
    /// before anything else this turn sends.
    Challenge(neoteletext_web::records::state::StateChallenge),
    /// A part of a live channel's index: its magazine, the magazine's
    /// name, its pages, and the sending's generation.
    Index {
        magazine: u8,
        name: String,
        pages: Vec<(u16, String, String, u16)>,
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
            Failure::Content | Failure::MediaType(_) => Self::Content,
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
const NOT_FOUND: u16 = STATUS_NOT_FOUND;

/// Whether an exchange says the channel has no bundle: `index.t42` answered
/// 404 or 410, for which the agent says nothing more, as a channel without
/// a bundle is still a channel (the engine's REQ-WEB-018). A bundle
/// answered otherwise is said by its bytes or its refusal.
fn bundle_missing(url: &str, status: u16) -> bool {
    (status == STATUS_NOT_FOUND || status == STATUS_GONE) && url.rsplit('/').next() == Some(BUNDLE_FILE)
}

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

/// Passes a channel's bytes to the trusted core, for its decoder, and
/// tells it of each state query among what the channel pushed (RULE-47):
/// the trusted core passes a page's state only in answer to a query this
/// fetcher received (RULE-52 item 6).
fn pass_bytes(output: &CorePipe, queries: &mut neoteletext_web::records::RecordReader, origin: Origin, bytes: Vec<u8>) -> Result<(), String> {
    let asked = if matches!(origin, Origin::PushedPackets) { rules_queries(queries, &bytes) } else { Vec::new() };
    send(output, &FromFetcher::Page { origin, bytes })?;
    for query in &asked {
        send(output, query)?;
    }
    Ok(())
}

/// The state queries (`RULES_QUERY`, page-rules.md RULE-47) among pushed
/// packets, each as the message that tells the trusted core of it. Whole
/// 42-byte packets are read as the channel's record stream; a query whose
/// page the protocol would refuse is left out, never an error: the bytes
/// are a server's.
fn rules_queries(reader: &mut neoteletext_web::records::RecordReader, bytes: &[u8]) -> Vec<FromFetcher> {
    use neoteletext_web::records::{ChannelRecord, Read as RecordRead};
    let mut asked = Vec::new();
    let (packets, _) = bytes.as_chunks::<{ neoteletext_core::consts::PACKET_BYTES }>();
    for packet in packets {
        if let RecordRead::Record(ChannelRecord::RulesQuery(query)) = reader.push(&neoteletext_core::packet::Packet { bytes: *packet }) {
            let message =
                FromFetcher::RulesQuery { page: protocol::rules::PageSub::new(query.page.mpp(), query.subcode.value()), identifier: query.identifier };
            if write_to_core(&mut Vec::new(), &message).is_ok() {
                asked.push(message);
            }
        }
    }
    asked
}

/// A page file, a bundle or a pushed message, for the trusted core: in
/// pieces of one message when a page file with a picture passes one (PAR-8,
/// the browser's DEC-6817), and the bundle settled once its pages have gone.
fn page_bytes(page: &neoteletext_web::agent::PageBytes<'_>, actions: &mut Vec<Action>) {
    let origin = match (page.source, page.media) {
        (Source::Push, _) => Origin::PushedPackets,
        (_, MediaType::T42) => Origin::FetchedPackets,
        (_, MediaType::Tti) => Origin::FetchedPage,
        _ => Origin::Fetched,
    };
    let packets = matches!(origin, Origin::FetchedPackets);
    match crate::pictured::pieces(packets, matches!(page.source, Source::Page { .. }), page.bytes.to_vec()) {
        Ok(pieces) => actions.extend(pieces.into_iter().map(|bytes| Action::Bytes { origin, bytes })),
        Err(reason) => actions.push(Action::Tell(FromFetcher::Notice(String::from(reason)))),
    }
    // The bundle's pages have gone, so a page it lacks may be asked for.
    if matches!(page.source, Source::FrontDoor) {
        actions.push(Action::BundleSettled);
    }
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
        page_bytes(&page, actions);
        return;
    }
    // Said to the trusted core by its role and status; a refused bundle has
    // settled, and a refused contents file lists nothing.
    if let Output::Event(event) = output
        && let Some((file, answer)) = refusal(&event)
    {
        actions.push(Action::Refused(file, answer));
        match file {
            RefusedFile::Bundle => actions.push(Action::BundleSettled),
            RefusedFile::Contents => actions.push(Action::Tell(FromFetcher::NoContents)),
            RefusedFile::Channel | RefusedFile::Page(_) => {}
        }
        return;
    }
    if let Output::Event(Event::Exchanged { method, url, status, bytes, elapsed_us }) = output {
        exchanged(method, url, status, bytes, elapsed_us, actions);
        return;
    }
    if let Some(action) = state_action(&output) {
        actions.push(action);
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
        Output::Event(Event::ServiceReady { name, initial_page, region, .. }) => Action::Ready(initial_page.mpp(), region, name.to_owned()),
        Output::Event(Event::ChannelDefaults) => Action::Unnamed,
        Output::Event(Event::Contents(bytes)) => Action::Contents(bytes.to_vec()),
        Output::Event(Event::ContentsMissing) => Action::Tell(FromFetcher::NoContents),
        // A page current, or a sub-page's file not there or failed, changes
        // nothing the trusted core holds.
        Output::Event(Event::PageCurrent { .. } | Event::PageNotInService { .. } | Event::FetchFailed { .. }) => return,
        Output::Event(Event::PushConnected) => Action::Push(true),
        Output::Event(Event::Denied(denied)) => Action::Tell(FromFetcher::Denied { status: denied.status, place: denied.place.to_owned() }),
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
            let pages = record.pages().map(|page| (page.page, page.title.to_owned(), page.section.to_owned(), page.subpages)).collect();
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

/// Viewer state's records (VS-204), for the trusted core as typed, fixed
/// fields, and never in a notice or a log line (VS-209).
const fn state_action(output: &Output<'_>) -> Option<Action> {
    Some(match output {
        Output::Event(Event::StateChallenge(challenge)) => Action::Challenge(*challenge),
        Output::Event(Event::StateEnd(end)) => Action::Tell(FromFetcher::StateEndReceived { key_id: end.key_id }),
        Output::Event(Event::StateKeepOffer) => Action::Tell(FromFetcher::StateKeepOffered),
        _ => return None,
    })
}

/// Writes a viewer state message on the push connection, as the input
/// protocol's event, straight to the agent (VS-204).
fn send_state(agent: &mut Agent<'_>, now: Instant, bytes: &[u8], actions: &mut Vec<Action>) {
    agent.handle(now, Input::Send(InputMessage::alone(bytes)), &mut |out| collect(out, actions));
}

/// When the loop next wakes of itself: the agent's timer, or the held
/// unprompted state message's turn, whichever is first.
fn wake_at(next_tick: Option<Instant>, state_due: Option<Clock>, now: Instant) -> Option<Instant> {
    let due = state_due.map(|due| Instant(now.0.saturating_add(u64::try_from(due.saturating_duration_since(Clock::now()).as_micros()).unwrap_or(u64::MAX))));
    match (next_tick, due) {
        (Some(tick), Some(due)) => Some(tick.min(due)),
        (tick, due) => tick.or(due),
    }
}

/// An exchange, for the developer console (the engine's REQ-WEB-033), and
/// the bundle settled when it says there is none.
fn exchanged(method: neoteletext_web::http::Method, url: &str, status: u16, bytes: usize, elapsed_us: u64, actions: &mut Vec<Action>) {
    actions.push(Action::Tell(FromFetcher::Exchange {
        method: String::from_utf8_lossy(method.token()).into_owned(),
        url: url.to_owned(),
        status,
        bytes: u64::try_from(bytes).unwrap_or(u64::MAX),
        elapsed_us,
    }));
    if bundle_missing(url, status) {
        actions.push(Action::BundleSettled);
    }
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

/// The contents file's sections and pages, as the engine's reader reads
/// them. A page it lists is not fetched for being listed (REQ-703).
fn contents_entries(bytes: &[u8]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let Ok(read) = contents::read(bytes) else { return entries };
    for entry in read.flatten() {
        match entry {
            ContentsEntry::Section { title } => entries.push(Entry::Section(title.to_owned())),
            ContentsEntry::Page { page, title, .. } => entries.push(Entry::Page { page: page.mpp(), title: title.to_owned(), subpages: 0 }),
            ContentsEntry::Channel { .. } => {}
        }
    }
    entries
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
fn send_part(output: &CorePipe, page: PageAddress, part: PartNumber, bytes: Option<&[u8]>) -> Result<(), String> {
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
/// standard input: the engine's agent opens it, which fetches `channel.ttx`
/// and then a static channel's bundle, holds a live channel's push
/// connection, and fetches each file the trusted core asks for as it is
/// wanted (REQ-703); what arrives goes to the trusted core on standard
/// output. A server on this computer
/// or the local network is reached only when `allowed` names it. Its
/// first message says it is confined, as `main.rs` has made it.
///
/// # Errors
/// When writing to the trusted core fails.
pub fn run(channel: &str, allowed: &[IpAddr], proxy: Option<Proxy>) -> Result<(), String> {
    let config = agent_config(allowed, proxy.is_some());
    // The transport refuses the same local addresses before connecting
    // (finding F-62).
    let allowed_local = config.allowed_local;
    run_with(channel, config, |wake| Tcp::new(wake, proxy, allowed_local), Box::new(io::stdin()), Box::new(io::stdout()))
}

/// [`run`] with the agent's settings, the transport `transport` makes with
/// the loop's waker, and the trusted core's pipes, which a test replaces.
fn run_with(
    channel: &str,
    config: Config,
    transport: impl FnOnce(crate::transport::Wake) -> Tcp,
    input: Box<dyn Read + Send>,
    output: Box<dyn Write>,
) -> Result<(), String> {
    // Before anything of the channel, so the trusted core can tell a
    // fetcher that could not start from one its server ended (finding
    // F-12).
    let output: CorePipe = Rc::new(RefCell::new(BufWriter::new(output)));
    send(&output, &FromFetcher::Confined)?;
    output.borrow_mut().flush().map_err(|error| error.to_string())?;
    // The body buffer holds the largest page file with a picture (PAGE_LIMIT).
    let mut storage = crate::pictured::Storage::new();
    let random = Random::new();
    let (wakes, woken) = mpsc::channel::<Wake>();
    // The trusted core's answers to viewer state's challenges, read by the
    // same thread that reads its input messages.
    let (answers, answered) = mpsc::channel();
    let mut state = State::new(answered);
    let mut agent: Box<Agent<'_>> = Box::new(UserAgent::new(config, storage.buffers(), random));
    let network = wakes.clone();
    let mut pump = Pump::new(transport(Arc::new(move || {
        let _ = network.send(Wake::Network);
    })));
    let start = Clock::now();
    let clock = || Instant(u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX));
    let mut buffer = vec![0_u8; READ_CHUNK];
    let mut next_tick: Option<Instant> = None;
    // A static channel's bundle not yet settled: the trusted core is told
    // once when it has (REQ-703).
    let mut bundle_pending = false;
    let mut parts = Parts::default();
    let mut live = Live::default();
    // The status of the last response, for a refusal of its content.
    let mut last_status = None;
    // The reader of the records the channel pushes, for the state queries
    // among them; every packet goes on to the decoder unread all the same.
    let mut queries = neoteletext_web::records::RecordReader::new();

    // The trusted core closes standard input to end the fetcher. Before
    // then it sends only what `ToFetcher` holds (REQ-1381 version 2):
    // input messages, the round trip's switch, viewer state's answers and
    // unprompted messages, and requests for pages, lists of sub-pages and
    // sub-pages; anything else ends the fetcher too.
    std::thread::spawn(move || {
        read_commands(&mut io::BufReader::new(input), &wakes, &answers);
        let _ = wakes.send(Wake::Ended);
    });

    let mut actions = Vec::new();
    // The channel has no `channel.ttx`, so its name is not its own.
    let mut unnamed = false;
    agent.handle(clock(), Input::OpenChannel(channel), &mut |out| collect(out, &mut actions));
    loop {
        let mut busy = !actions.is_empty();
        for action in std::mem::take(&mut actions) {
            match action {
                Action::Connect { connection, host, port, security } => pump.execute(NetOutput::Connect { connection, host: &host, port, security }),
                Action::Send(connection, bytes) => pump.execute(NetOutput::Send(connection, &bytes)),
                Action::Close(connection) => pump.execute(NetOutput::Close(connection)),
                Action::Timer(at) => next_tick = Some(next_tick.map_or(at, |current| current.min(at))),
                Action::Bytes { origin, bytes } => pass_bytes(&output, &mut queries, origin, bytes)?,
                Action::Unnamed => unnamed = true,
                Action::Ready(page, region, name) => {
                    service_ready(&agent, &output, page, region, (!unnamed).then_some(name))?;
                    bundle_pending = agent.descriptor().is_some_and(|descriptor| descriptor.push().is_none());
                    ask_for_channel(&mut agent, &clock, &mut actions);
                }
                Action::Contents(bytes) => send(&output, &FromFetcher::Contents(contents_entries(&bytes)))?,
                Action::BundleSettled => {
                    if core::mem::take(&mut bundle_pending) {
                        send(&output, &FromFetcher::BundleSettled)?;
                    }
                }
                Action::SubPages(page, part, bytes) => {
                    send_part(&output, page, part, Some(&bytes))?;
                    parts.keep(page, part, bytes);
                }
                // A part no longer kept, though the agent holds its
                // validators, is said to have failed: the trusted core asks
                // for it again when its page next opens.
                Action::SubPagesCurrent(page, part) => send_part(&output, page, part, parts.get(page, part))?,
                Action::Refused(file, answer) => send(&output, &FromFetcher::Refused { file, status: answer.status(last_status) })?,
                Action::Push(open) => push_changed(&agent, &output, open, &mut state)?,
                // Answered now, before the input messages the trusted core
                // sent meanwhile, which wait in the wakes below (VS-202).
                Action::Challenge(challenge) => send_state(&mut agent, clock(), &state.challenged(&output, challenge), &mut actions),
                action @ (Action::Index { .. } | Action::IndexComplete(_) | Action::SubPageList(_)) => live_record(action, &mut live, &output)?,
                Action::Tell(message) => {
                    last_status = if let FromFetcher::Exchange { status, .. } = message { Some(status) } else { last_status };
                    send(&output, &message)?;
                }
            }
        }
        tell_failures(&mut pump.transport, &output)?;
        busy |= poll_sockets(&mut pump, &mut buffer, &mut agent, &clock, &mut actions);
        if next_tick.is_some_and(|at| at <= clock()) {
            next_tick = None;
            busy = true;
            agent.handle(clock(), Input::Tick, &mut |out| collect(out, &mut actions));
        }
        if let Some(bytes) = state.due(Clock::now()) {
            busy = true;
            send_state(&mut agent, clock(), &bytes, &mut actions);
        }
        // What this turn produced goes before waiting.
        output.borrow_mut().flush().map_err(|error| error.to_string())?;
        let first = wait_for_wake(&woken, busy, wake_at(next_tick, state.next_due(), clock()), clock());
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
                // Sent once the interval allows, at the top of the loop.
                Wake::State(message) => state.hold(&message),
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
fn live_record(action: Action, live: &mut Live, output: &CorePipe) -> Result<(), String> {
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

/// The agent, with the fetcher's randomness.
type Agent<'a> = UserAgent<'a, Random>;

/// Writes `message` to the trusted core.
fn send(output: &CorePipe, message: &FromFetcher) -> Result<(), String> {
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

/// Tells the trusted core what the open channel's descriptor says, with
/// the name it gives when it is the channel's own.
fn service_ready(agent: &Agent<'_>, output: &CorePipe, page: u16, region: Option<u8>, name: Option<String>) -> Result<(), String> {
    // Before any page, so that none is read in another group.
    send(output, &FromFetcher::Region(region))?;
    send(output, &FromFetcher::InitialPage(page))?;
    // The tab's title (REQ-6641); none for a channel without its own.
    if let Some(name) = name {
        send(output, &FromFetcher::ChannelName(name))?;
    }
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

/// Asks for a live channel's push connection, for all its pages. Nothing
/// else is asked as a channel opens: the agent fetches a static channel's
/// bundle by itself, and its contents file and pages only as the trusted
/// core says they are wanted (REQ-703).
fn ask_for_channel(agent: &mut Agent<'_>, clock: &dyn Fn() -> Instant, actions: &mut Vec<Action>) {
    agent.handle(clock(), Input::Subscribe(Subscription::All), &mut |out| collect(out, actions));
}

/// The push connection opened or closed. Keys may go to the channel while
/// its WebSocket is open, if its descriptor says it takes them
/// (`interaction.md`).
fn push_changed(agent: &Agent<'_>, output: &CorePipe, open: bool, state: &mut State) -> Result<(), String> {
    if !open {
        state.lost();
    }
    // The trusted core keeps a sleeping tab's fetcher while it holds a
    // push connection (decision 47).
    send(output, &FromFetcher::Pushing(open))?;
    #[cfg(feature = "interaction")]
    if let Some(descriptor) = agent.descriptor().filter(|descriptor| descriptor.push_takes_messages()) {
        let secure = descriptor.push().is_some_and(neoteletext_web::url::Url::is_secure);
        send(output, &FromFetcher::Input { open, secure })?;
    }
    #[cfg(not(feature = "interaction"))]
    let _ = agent;
    Ok(())
}

/// Tells the trusted core what the transport refused since it was last
/// asked: a proxy's failure, a certificate, an opening out of time, a local
/// address.
fn tell_failures(transport: &mut Tcp, output: &CorePipe) -> Result<(), String> {
    let send = |message: FromFetcher| write_to_core(&mut *output.borrow_mut(), &message).map_err(|error| error.to_string());
    if transport.take_proxy_failure() {
        send(FromFetcher::ProxyFailed)?;
    }
    if let Some(problem) = transport.take_certificate_refusal() {
        send(FromFetcher::CertificateRefused(problem))?;
    }
    if transport.take_timeout() {
        send(FromFetcher::TimedOut)?;
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
        ToFetcher::FetchContents => Some(Input::FetchContents),
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
/// protocol: input messages and requests wake the loop, and viewer state's
/// answers go to the loop waiting for them. An input message the trusted core
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
            answer @ ToFetcher::StateAnswer { .. } => answers.send(answer).is_ok(),
            unprompted @ (ToFetcher::StateSendOffer | ToFetcher::StateSendNone(_)) => commands.send(Wake::State(unprompted)).is_ok(),
            asked @ (ToFetcher::FetchContents | ToFetcher::FetchPage { .. } | ToFetcher::FetchSubPages { .. } | ToFetcher::FetchSubPage { .. }) => {
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
    use neoteletext_web::agent::Storage;
    use neoteletext_web::channel::SETTINGS_FILE;

    /// REQ-708: every failure of a whole file reaches the trusted core by
    /// its role, with the status when there was one and nothing a server
    /// wrote; a missing page is 404. On the code before, a page's failures
    /// only ended the wait for the next page, and the rest were a debug
    /// line in the Log.
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
        assert_eq!(actions_of(Event::DescriptorFailed(Failure::MediaType(MediaType::TextPlain))), [(RefusedFile::Channel, Answer::Content)]);
        assert_eq!(actions_of(Event::BundleFailed(Failure::Status(403))), [(RefusedFile::Bundle, Answer::Status(403))]);
        assert_eq!(actions_of(Event::ContentsFailed(Failure::Timeout)), [(RefusedFile::Contents, Answer::Nothing)]);
        assert_eq!(actions_of(Event::PageNotInService { page, subcode: Some(SubCode::new(2)) }), [], "a sub-page's file is not the page's");
    }

    /// REQ-703: the bundle settles when its pages have gone, when it is
    /// refused, and when it is missing, which the agent says only by the
    /// exchange's 404 or 410; nothing else settles it.
    #[test]
    fn the_bundle_settles_however_it_is_answered() {
        let settles = |output: Output<'_>| {
            let mut actions = Vec::new();
            collect(output, &mut actions);
            actions.iter().any(|action| matches!(action, Action::BundleSettled))
        };
        let exchange = |url, status| Output::Event(Event::Exchanged { method: neoteletext_web::http::Method::Get, url, status, bytes: 0, elapsed_us: 0 });
        let bundle = format!("http://a.example/tt/{BUNDLE_FILE}");
        assert!(settles(exchange(&bundle, 404)));
        assert!(settles(exchange(&bundle, 410)));
        assert!(!settles(exchange(&bundle, 200)), "its bytes or its refusal follow");
        assert!(!settles(exchange(&bundle, 301)), "a redirect is followed");
        assert!(!settles(exchange("http://a.example/tt/100.tti", 404)));
        assert!(settles(Output::Event(Event::BundleFailed(Failure::Status(500)))));
        let bytes = neoteletext_web::agent::PageBytes { source: Source::FrontDoor, media: MediaType::T42, url: &bundle, secure: false, bytes: &[] };
        assert!(settles(Output::PageBytes(bytes)));
        assert!(!settles(Output::Event(Event::ContentsMissing)));
    }

    /// Finding F-74: the largest page or bundle the agent
    /// accepts is one message to the trusted core. Before this the agent
    /// took a body of its default limit, 262,144 bytes, which with the
    /// origin byte is one byte too many for a message, so writing it
    /// failed and ended the channel's fetcher.
    #[test]
    fn the_largest_page_the_agent_accepts_reaches_the_core() {
        let config = agent_config(&[], false);
        assert_eq!(config.page_limit, crate::pictured::MAX_PICTURED_PAGE_FILE, "the largest page file with a picture (DEC-6810)");
        let page = FromFetcher::Page { origin: Origin::FetchedPage, bytes: vec![b'x'; crate::pictured::PAGE_MESSAGE] };
        let mut written = Vec::new();
        write_to_core(&mut written, &page).unwrap();
        let read = protocol::fetcher::read_from_fetcher(&mut written.as_slice()).unwrap();
        assert!(matches!(read, Some(FromFetcher::Page { bytes, .. }) if bytes.len() == crate::pictured::PAGE_MESSAGE));
        // Past one message, a TTI page is dropped with a notice, never
        // written whole (which would end this fetcher).
        let tti = neoteletext_web::agent::PageBytes {
            source: Source::Page { page: PageAddress::from_mpp(0x100).unwrap(), subcode: None },
            media: MediaType::Tti,
            url: "http://a.invalid/100.tti",
            secure: false,
            bytes: &vec![b'x'; crate::pictured::PAGE_MESSAGE + 1],
        };
        let mut actions = Vec::new();
        collect(Output::PageBytes(tti), &mut actions);
        assert!(matches!(actions.as_slice(), [Action::Tell(FromFetcher::Notice(_))]));
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
        // REQ-300: a page with a hexadecimal digit is a page like any
        // other. A number the standard reserves goes to the log, as any
        // other event: the pipe's reader would refuse it.
        let lettered = PageAddress::from_mpp(0x1A0).unwrap();
        collect(Output::Event(Event::Redirect(RedirectRecord::Page { page: lettered, subcode: SubCode::new(2), parameters: None })), &mut actions);
        assert!(matches!(&actions[2], Action::Tell(FromFetcher::PageRedirect { page: 0x1A0, sub: 2, query: None })));
        let reserved = PageAddress::from_mpp(0x1FE).unwrap();
        collect(Output::Event(Event::Redirect(RedirectRecord::Page { page: reserved, subcode: SubCode::new(2), parameters: None })), &mut actions);
        assert!(matches!(&actions[3], Action::Tell(FromFetcher::Notice(_))));
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

    /// REQ-701 item 5 and the engine's REQ-420 (REQ-SEC-007), as DEC-2378
    /// settled: an `https` channel redirected to a plain `http` address,
    /// its own folder or another channel's, is refused. The trusted core
    /// is told of no redirect to follow, and nothing is asked for over
    /// plain HTTP. This runs the fetcher's own handling of the engine's
    /// agent, which is what a real fetcher sends; the trusted core's
    /// stand-in fetcher cannot show it.
    #[test]
    fn a_redirect_from_https_to_http_is_refused_and_nothing_is_fetched_plain() {
        use neoteletext_web::port::{NetInput, Security};
        for location in [format!("http://a.example/tt/{SETTINGS_FILE}"), format!("http://b.example/tt/{SETTINGS_FILE}")] {
            let reply = format!("HTTP/1.1 301 Moved Permanently\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n");
            let mut storage = Box::new(Storage::default());
            let config = Config { tls_available: true, redirects_within_channel: true, ..Config::default() };
            let mut agent: Box<Agent<'_>> = Box::new(UserAgent::new(config, storage.buffers(), Random::new()));
            let mut actions = Vec::new();
            agent.handle(Instant(0), Input::OpenChannel("https://a.example/tt/"), &mut |out| collect(out, &mut actions));
            assert!(matches!(&actions[..], [Action::Connect { security: Security::Tls, .. }, ..]), "opened encrypted");
            let opened = actions.len();
            let connected = NetInput::Connected(ConnectionId::Fetch, neoteletext_web::url::IpAddress::V4([93, 184, 216, 34]));
            agent.handle(Instant(0), Input::Net(connected), &mut |out| collect(out, &mut actions));
            agent.handle(Instant(0), Input::Net(NetInput::Received(ConnectionId::Fetch, reply.as_bytes())), &mut |out| collect(out, &mut actions));
            let after = actions.get(opened..).unwrap_or_default();
            assert!(!after.iter().any(|action| matches!(action, Action::Tell(FromFetcher::Redirected(_)))), "{location}: not followed");
            assert!(!after.iter().any(|action| matches!(action, Action::Connect { security: Security::Plain, .. })), "{location}: nothing plain");
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

    /// The report of a rule that carries `input`, a viewer's input event as
    /// the engine's writer writes it: the only form input goes in under
    /// push protocol 10 (page-rules.md RULE-46). The agent does not read it.
    #[cfg(feature = "interaction")]
    fn reported(input: &[u8]) -> Vec<u8> {
        use neoteletext_input::event::{Event, Item, Stamp, write};
        use neoteletext_input::rules::{RuleBody, RuleEvent};
        let on = Stamp::new(0x200, 0).unwrap();
        let target = neoteletext_rules::table::Target::NONE;
        let event = RuleEvent { on, generation: 1, rule: 0, row: neoteletext_rules::limits::NO_RULE, tick: 1, target, body: RuleBody::Input(input) };
        let mut out = [0_u8; 64];
        let length = write(&Item::Event(Event::RuleEvent(event)), &mut out).unwrap();
        out[..length].to_vec()
    }

    /// The report of a MOUSE move to the dot `x`.
    #[cfg(feature = "interaction")]
    fn mouse_move(x: u8) -> Vec<u8> {
        reported(&[0xFF, 0x02, 0x80, 0x04, 0x00, 0x06, 0x00, 0x01, 0x05, x, 0x64])
    }

    /// The report of a KEY press of `a`.
    #[cfg(feature = "interaction")]
    fn key_report() -> Vec<u8> {
        reported(&[0xFF, 0x01, 0x80, 0x04, 0x00, 0x10, b'a', 1, b'a'])
    }

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
        assert!(matches!(&actions[..], [Action::Ready(0x100, Some(6), name)] if name == "Test"));
    }

    /// REQ-6641 item 2: a channel with no `channel.ttx` gave no name of its
    /// own, so the engine's name for it, its host and folder, is not sent.
    #[test]
    fn a_channel_without_its_settings_file_is_unnamed() {
        let mut actions = Vec::new();
        collect(Output::Event(Event::ChannelDefaults), &mut actions);
        assert!(matches!(actions[..], [Action::Unnamed]));
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
            send(&mouse_move(1), None, true),
            send(&key_report(), None, false),
            send(&key_report(), None, false),
            send(&key_report(), None, true),
            ToFetcher::MeasureRoundTrip(true),
            send(&mouse_move(2), None, true),
        ]);
        assert_eq!(wakes.len(), 5);
        assert_eq!(records(&wakes[0]), [(mouse_move(1), None), (key_report(), None)], "one moment, in order");
        assert_eq!(records(&wakes[1]), [(key_report(), None)], "a message alone, at once");
        assert_eq!(records(&wakes[2]), [(key_report(), None)], "another message hands on the moment");
        assert!(matches!(wakes[3], Wake::RoundTrip(true)));
        assert_eq!(records(&wakes[4]), [(mouse_move(2), None)], "the pipe's end hands on the moment");
        let many: Vec<ToFetcher> = (0..=MOMENT_MESSAGES).map(|_| send(&key_report(), None, true)).collect();
        let wakes = wakes_for(&many);
        assert_eq!(wakes.iter().map(|wake| records(wake).len()).collect::<Vec<_>>(), [MOMENT_MESSAGES, 1], "bounded");

        let mut storage = Box::new(Storage::default());
        let mut agent = agent(&mut storage);
        open_push(&mut agent, 0);
        let mut actions = Vec::new();
        let mut handle = |input: Input<'_>| agent.handle(Instant(0), input, &mut |out| collect(out, &mut actions));
        let moment = [Record { bytes: mouse_move(1), move_of: Some(0) }, Record { bytes: key_report(), move_of: None }];
        give_records(&moment, false, &mut handle);
        give_records(&[Record { bytes: key_report(), move_of: None }], true, &mut handle);
        give_records(&[Record { bytes: mouse_move(3), move_of: Some(0) }], false, &mut handle);
        give_records(&[Record { bytes: key_report(), move_of: None }], false, &mut handle);
        let binary = |bytes: &[u8]| (neoteletext_web::consts::OPCODE_BINARY, bytes.to_vec());
        assert_eq!(
            push_sends(&mut actions),
            [vec![binary(&mouse_move(1)), binary(&key_report())], vec![binary(&key_report()), binary(&mouse_move(3))], vec![binary(&key_report())]],
            "a moment in one send, in order; one already here joins it; a lone message at once"
        );
    }

    /// The frames a WebSocket client wrote, read from `stream` until it ends:
    /// opcode and unmasked payload each (RFC 6455 section 5.2).
    #[cfg(feature = "interaction")]
    #[allow(clippy::arithmetic_side_effects, reason = "a test's frames, within their lengths")]
    fn read_client_frame(stream: &mut dyn Read) -> Option<(u8, Vec<u8>)> {
        let mut head = [0_u8; 2];
        stream.read_exact(&mut head).ok()?;
        let length = match head[1] & 0x7F {
            126 => {
                let mut length = [0_u8; 2];
                stream.read_exact(&mut length).ok()?;
                usize::from(u16::from_be_bytes(length))
            }
            127 => return None,
            length => usize::from(length),
        };
        let mut mask = [0_u8; 4];
        stream.read_exact(&mut mask).ok()?;
        let mut payload = vec![0_u8; length];
        stream.read_exact(&mut payload).ok()?;
        payload.iter_mut().zip(mask.iter().cycle()).for_each(|(byte, key)| *byte ^= key);
        Some((head[0] & 0x0F, payload))
    }

    /// Row 1 of a page of magazine 1, blank, as 42 bytes: its address
    /// Hamming 8/4 coded, magazine and the row's low bit in the first byte
    /// and the row's other bits in the second (EN 300 706 clause 7.1.2).
    #[cfg(feature = "interaction")]
    fn blank_row() -> Vec<u8> {
        use neoteletext_core::ecc::hamming84_encode;
        const MAGAZINE: u8 = 1;
        const ROW: u8 = 1;
        // A space has one bit set, so it is its own odd parity (clause 8.2).
        const SPACE: u8 = b' ';
        let mut packet = vec![hamming84_encode(MAGAZINE | ((ROW & 1) << 3)), hamming84_encode(ROW >> 1)];
        packet.resize(neoteletext_core::consts::PACKET_BYTES, SPACE);
        packet
    }

    /// TODO-2075 item 1, REQ-696, REQ-814: a live channel served only over
    /// TLS, its push connection `wss`: the fetcher's own loop, as `run`
    /// runs it but trusting the test's certificate, opens `channel.ttx`
    /// over `https`, completes the WebSocket handshake over TLS, passes the
    /// pushed packets to the trusted core as pushed, says input goes over a
    /// secure connection, and writes an input message the trusted core
    /// gives it to the channel over `wss`. No test opened a `wss`
    /// connection before; its parts were each tested over plain WebSocket.
    /// VS-202 and VS-204: the channel's `STATE_CHALLENGE`, pushed before its
    /// page, is asked of the trusted core, and the core's proof reaches the
    /// channel before the input message the core sent while it was asked.
    #[cfg(feature = "interaction")]
    #[test]
    #[allow(clippy::too_many_lines, reason = "one server and one exchange, read in order")]
    fn a_secure_push_connection_carries_pages_and_input() {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
        use std::io::{BufRead, BufReader};
        use std::net::{Ipv4Addr, TcpListener};
        // The transport's test certificate for `localhost`, valid to 2126.
        let certificate = CertificateDer::from(include_bytes!("transport/testdata/localhost-certificate.der").to_vec());
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(include_bytes!("transport/testdata/localhost-key.der").to_vec()));
        let server_config = Arc::new(rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![certificate.clone()], key).unwrap());
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let page = blank_row();
        let (heard, hearing) = mpsc::channel::<Vec<u8>>();
        let pushed = page.clone();
        std::thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                let (server_config, pushed, heard) = (Arc::clone(&server_config), pushed.clone(), heard.clone());
                std::thread::spawn(move || {
                    let connection = rustls::ServerConnection::new(server_config).unwrap();
                    let mut stream = BufReader::new(rustls::StreamOwned::new(connection, socket));
                    loop {
                        let mut lines = Vec::new();
                        loop {
                            let mut line = String::new();
                            if stream.read_line(&mut line).unwrap_or(0) == 0 {
                                return;
                            }
                            if line.trim().is_empty() {
                                break;
                            }
                            lines.push(line.trim().to_owned());
                        }
                        let path = lines[0].split(' ').nth(1).unwrap_or_default().to_owned();
                        let writer = stream.get_mut();
                        if path == "/tt/push" {
                            let key = lines.iter().find_map(|line| line.strip_prefix("Sec-WebSocket-Key: ")).unwrap().to_owned();
                            let accept = neoteletext_web::ws::accept_for(key.as_bytes());
                            let upgrade = format!(
                                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Protocol: {}\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                                neoteletext_web::consts::PUSH_SUBPROTOCOL,
                                std::str::from_utf8(&accept).unwrap()
                            );
                            writer.write_all(upgrade.as_bytes()).unwrap();
                            // The challenge first, as a server sends it after
                            // WELCOME (VS-202), in a frame of its own.
                            let mut challenge = Vec::new();
                            let state = neoteletext_web::records::state::StateChallenge { challenge: [7; 32], create: true };
                            neoteletext_web::records::state::encode_challenge(state, 0, 1, &mut |packet| challenge.extend_from_slice(&packet.bytes)).unwrap();
                            let mut frame = vec![0x80 | neoteletext_web::consts::OPCODE_BINARY, u8::try_from(challenge.len()).unwrap()];
                            frame.extend_from_slice(&challenge);
                            writer.write_all(&frame).unwrap();
                            // One binary frame, unmasked as a server's is.
                            let mut frame = vec![0x80 | neoteletext_web::consts::OPCODE_BINARY, u8::try_from(pushed.len()).unwrap()];
                            frame.extend_from_slice(&pushed);
                            writer.write_all(&frame).unwrap();
                            writer.flush().unwrap();
                            while let Some((opcode, payload)) = read_client_frame(&mut stream) {
                                if opcode == neoteletext_web::consts::OPCODE_BINARY && heard.send(payload).is_err() {
                                    return;
                                }
                            }
                            return;
                        }
                        let body = br#"{"name":"Secure","push":{"input":true}}"#;
                        let answer = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len());
                        writer.write_all(answer.as_bytes()).unwrap();
                        writer.write_all(body).unwrap();
                        writer.flush().unwrap();
                    }
                });
            }
        });

        let (core_reads, fetcher_writes) = io::pipe().unwrap();
        let (fetcher_reads, mut core_writes) = io::pipe().unwrap();
        let channel = format!("https://localhost:{port}/tt/");
        std::thread::spawn(move || {
            let mut roots = rustls::RootCertStore::empty();
            roots.add(certificate).unwrap();
            let local = [IpAddr::from(Ipv4Addr::LOCALHOST)];
            let config = agent_config(&local, false);
            let allowed_local = config.allowed_local;
            run_with(&channel, config, |wake| Tcp::with_roots(roots, wake, None, allowed_local), Box::new(fetcher_reads), Box::new(fetcher_writes))
        });
        let mut from_fetcher = BufReader::new(core_reads);
        let mut until = |core_writes: &mut io::PipeWriter, wanted: &dyn Fn(&FromFetcher) -> bool| loop {
            let message = protocol::fetcher::read_from_fetcher(&mut from_fetcher).unwrap().expect("the fetcher ended");
            // An input message first, as SELECT would be sent while the
            // core is asked, then the proof, which must still go first.
            if let FromFetcher::StateChallenged { request, challenge, create: true } = &message {
                assert_eq!(*challenge, [7; 32]);
                protocol::fetcher::write_to_fetcher(core_writes, &ToFetcher::Send { bytes: key_report(), move_of: None, more: false }).unwrap();
                let reply = protocol::fetcher::StateReply::Proof {
                    algorithm: protocol::fetcher::Algorithm::Ed25519,
                    lifetime: protocol::fetcher::Lifetime::Visit,
                    public_key: [1; 32],
                    signature: [2; 64],
                };
                protocol::fetcher::write_to_fetcher(core_writes, &ToFetcher::StateAnswer { request: *request, reply }).unwrap();
            }
            if wanted(&message) {
                return message;
            }
        };
        // The handshake: the connection is open, and said secure.
        let input = until(&mut core_writes, &|message| matches!(message, FromFetcher::Input { .. }));
        assert_eq!(input, FromFetcher::Input { open: true, secure: true }, "keys go over a secure connection");
        // The challenge's own frame is passed on as pushed too, as every
        // record is, for the decoder; the page is the next frame's.
        let page_message =
            until(&mut core_writes, &|message| matches!(message, FromFetcher::Page { origin: Origin::PushedPackets, bytes } if bytes.len() == page.len()));
        assert_eq!(page_message, FromFetcher::Page { origin: Origin::PushedPackets, bytes: page }, "pushed over wss");
        // A ceiling, not a wait: each arrives as soon as it is written.
        let deadline = Duration::from_secs(10);
        let proof = hearing.recv_timeout(deadline).expect("the proof reached the channel");
        assert_eq!(proof[..5], [0xFF, 0x48, 98, 1, 0], "STATE_PROOF, Ed25519, a visit key, first");
        assert_eq!(proof.len(), 3 + 98);
        let key = hearing.recv_timeout(deadline).expect("the input message reached the channel");
        assert_eq!(key, key_report(), "written over wss, after the proof");
    }

    /// REQ-814: the push WebSocket has the engine's heartbeat in the
    /// fetcher: silent for `PUSH_PING_AFTER_US`, it is pinged, and a ping
    /// not answered within `PUSH_PONG_WITHIN_US` loses the connection,
    /// which is tried again.
    #[cfg(feature = "interaction")]
    #[test]
    fn a_silent_push_connection_is_pinged_and_lost_when_it_does_not_answer() {
        use neoteletext_web::consts::{OPCODE_PING, PUSH_PING_AFTER_US, PUSH_PONG_WITHIN_US};
        let mut storage = Box::new(Storage::default());
        let mut agent = agent(&mut storage);
        open_push(&mut agent, 0);
        let mut actions = Vec::new();
        agent.handle(Instant(PUSH_PING_AFTER_US - 1), Input::Tick, &mut |out| collect(out, &mut actions));
        assert!(push_sends(&mut actions).concat().iter().all(|(opcode, _)| *opcode != OPCODE_PING), "not yet");
        agent.handle(Instant(PUSH_PING_AFTER_US), Input::Tick, &mut |out| collect(out, &mut actions));
        assert!(push_sends(&mut actions).concat().iter().any(|(opcode, _)| *opcode == OPCODE_PING), "the heartbeat");
        agent.handle(Instant(PUSH_PING_AFTER_US + PUSH_PONG_WITHIN_US), Input::Tick, &mut |out| collect(out, &mut actions));
        assert!(actions.iter().any(|action| matches!(action, Action::Push(false))), "unanswered, the connection is lost");
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

    /// RULE-47, RULE-52 item 6 (REQ-6534): a `RULES_QUERY` record among
    /// what a channel pushes is told the trusted core, by its page and
    /// its identifier; other records and packets are not, a record cut
    /// across two messages is read whole, and a query for a page the
    /// protocol refuses is left out without ending the fetcher.
    #[test]
    fn a_state_query_the_channel_pushes_is_told_the_trusted_core() {
        use neoteletext_web::records::rules::{RulesQueryRecord, encode_query};
        // Each record under its own record id, as a server numbers them.
        let packets = |page: u16, identifier: u32| {
            let mut bytes = Vec::new();
            let record = RulesQueryRecord { page: PageAddress::from_mpp(page).unwrap(), subcode: SubCode::new(3), identifier };
            encode_query(record, 0, u8::try_from(identifier & 0xFF).unwrap(), &mut |packet| bytes.extend_from_slice(&packet.bytes)).unwrap();
            bytes
        };
        let mut reader = neoteletext_web::records::RecordReader::new();
        let pushed = packets(0x101, 0x0102_0304);
        assert_eq!(rules_queries(&mut reader, &pushed), [FromFetcher::RulesQuery { page: protocol::rules::PageSub::new(0x101, 3), identifier: 0x0102_0304 }]);
        assert_eq!(rules_queries(&mut reader, &[0x15; 84]), [], "packets that are no record");
        let reserved = packets(0x1FF, 7);
        assert_eq!(rules_queries(&mut reader, &reserved), [], "a page no channel has");
        assert_eq!(
            rules_queries(&mut reader, &packets(0x200, 8)),
            [FromFetcher::RulesQuery { page: protocol::rules::PageSub::new(0x200, 3), identifier: 8 }],
            "and the next is read"
        );
    }
}

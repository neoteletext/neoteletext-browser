//! The web client process's loop: the trusted core's messages in, each
//! request a task of its own on the one thread, and the events out, each
//! with its tab before it as on a channel fetcher's pipe (REQ-6980). A
//! process serves one partition (the browser's DEC-7268): a start with
//! another partition's token ends it, as a message that is no message
//! does. The messages are `protocol::web`'s, the one definition the
//! script host, the trusted core and this process share (DEC-7317).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::IpAddr;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hyper::body::Bytes;
use neoteletext_web::proxy::Proxy;
use protocol::fetcher::PROCESS_TAB;
use protocol::script::ContextId;
use protocol::web::{
    AddressSpace, BodySource, CacheMode, ErrorKind, FETCH_WINDOW_BYTES, FromWebClient, Head, JarPiece, MAX_CHUNK_BYTES, MAX_JAR_BYTES,
    MAX_NETWORK_BUFFER_BYTES, MAX_REQUEST_BODY_BYTES, Mode, PartitionToken, Reason, RedirectMode, RequestKind, Start, ToWebClient, is_header_value,
    read_to_client, write_from_client,
};
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio::task::JoinHandle;

use super::connect::Settings;
use super::cookies::{Jar, Suffixes};
use super::cors::PreflightCache;
use super::fetch::{Engine, Fetched, fetch};
use super::limits::{
    BODY_IDLE_TIMEOUT, DEFAULT_RETRY, KEEPALIVE_DEADLINE, MAX_KEEPALIVE_BODY_BYTES, MAX_REQUESTS_PER_TAB, MAX_RETRY, MAX_WAITING_PER_TAB, MIN_RETRY,
    QUEUED_MESSAGES, START_BURST, STARTS_PER_SECOND,
};
use super::policy::ChannelPolicy;
use super::pool::Pool;
use super::socket::{self, Command, Told};
use super::sse::EventStream;

/// What the reader thread found on the pipe.
enum FromCore {
    /// A message, and the tab it is for.
    Message(u32, ToWebClient),
    /// The pipe ended.
    End,
    /// The pipe broke, or carried what is no message.
    Broken(String),
}

/// The room a request's answer has: the bytes the trusted core will take.
struct Room {
    bytes: Cell<u32>,
    more: Notify,
    /// Whether the request outlived its page (a keepalive request whose
    /// context or tab closed): its answer is then told to no one, and
    /// waits for no room.
    detached: Cell<bool>,
}

impl Room {
    /// Waits for room for `count` bytes, and takes it.
    async fn take(&self, count: usize) {
        let count = u32::try_from(count).unwrap_or(u32::MAX).clamp(1, FETCH_WINDOW_BYTES);
        loop {
            let notified = self.more.notified();
            if self.detached.get() {
                return;
            }
            if let Some(left) = self.bytes.get().checked_sub(count) {
                self.bytes.set(left);
                return;
            }
            notified.await;
        }
    }

    /// A new request's room.
    fn new() -> Rc<Self> {
        Rc::new(Self { bytes: Cell::new(FETCH_WINDOW_BYTES), more: Notify::new(), detached: Cell::new(false) })
    }

    /// Lets the request go on with no one to tell.
    fn detach(&self) {
        self.detached.set(true);
        self.more.notify_waiters();
    }

    fn add(&self, count: u32) {
        self.bytes.set(self.bytes.get().saturating_add(count).min(MAX_NETWORK_BUFFER_BYTES));
        self.more.notify_waiters();
    }
}

/// A tab's requests: those under way counted, those waiting counted, and
/// its starts held to a rate.
struct Tab {
    under_way: Semaphore,
    waiting: Cell<usize>,
    /// Starts it may still make, in thousandths, and when that was last
    /// added to.
    starts: Cell<u64>,
    refilled: Cell<Instant>,
}

/// A start's worth of [`Tab::starts`].
const START: u64 = 1000;

/// A request counted as waiting until it is dropped.
struct Waiting<'a>(&'a Tab);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.waiting.set(self.0.waiting.get().saturating_sub(1));
    }
}

impl Tab {
    fn new() -> Self {
        Self {
            under_way: Semaphore::new(MAX_REQUESTS_PER_TAB),
            waiting: Cell::new(0),
            starts: Cell::new(u64::from(START_BURST).saturating_mul(START)),
            refilled: Cell::new(Instant::now()),
        }
    }

    /// Whether the tab may start a request now: [`STARTS_PER_SECOND`] a
    /// second after a burst of [`START_BURST`].
    fn admits(&self, now: Instant) -> bool {
        let since = u64::try_from(now.saturating_duration_since(self.refilled.get()).as_millis()).unwrap_or(u64::MAX);
        // A thousandth of a start each millisecond, for each start a second.
        let starts = self.starts.get().saturating_add(since.saturating_mul(u64::from(STARTS_PER_SECOND))).min(u64::from(START_BURST).saturating_mul(START));
        self.refilled.set(now);
        let left = starts.checked_sub(START);
        self.starts.set(left.unwrap_or(starts));
        left.is_some()
    }

    /// Waits for one of the tab's [`MAX_REQUESTS_PER_TAB`] places.
    async fn enter(&self) -> Result<tokio::sync::SemaphorePermit<'_>, Reason> {
        if self.waiting.get() >= MAX_WAITING_PER_TAB {
            return Err(Reason::Busy);
        }
        self.waiting.set(self.waiting.get().saturating_add(1));
        let _waiting = Waiting(self);
        self.under_way.acquire().await.map_err(|_| Reason::Busy)
    }
}

/// A socket's task's end of its commands, and the count of bytes handed
/// over to it.
type Socket = (mpsc::UnboundedReceiver<Command>, Rc<Cell<usize>>);

/// A request, known by its tab and the number its script host gave it.
type Key = (u32, u32);

/// A request that has started and not ended.
struct Live {
    /// The context that started it, with which it ends.
    context: ContextId,
    room: Rc<Room>,
    /// Its task, once its body is whole.
    task: Option<JoinHandle<()>>,
    /// Its start and the body so far, while the body is arriving.
    upload: Option<(Box<Start>, Vec<u8>)>,
    /// Whether it is a keepalive request, which outlives its context.
    keepalive: bool,
    /// A socket's way in for what the script sends, and the bytes handed
    /// over that the socket has not yet taken.
    socket: Option<(mpsc::UnboundedSender<Command>, Rc<Cell<usize>>)>,
}

/// A request's task, telling its events and waiting for its room.
struct Teller<'a> {
    shared: &'a Shared,
    room: &'a Room,
    tab: u32,
}

impl Told for Teller<'_> {
    async fn tell(&self, event: FromWebClient) {
        self.shared.tell(self.room, self.tab, event).await;
    }

    async fn room(&self, count: usize) {
        self.room.take(count).await;
    }

    async fn jar(&self) {
        self.shared.send_jar().await;
    }
}

/// What the loop and every request's task share.
struct Shared {
    engine: Engine,
    live: RefCell<HashMap<Key, Live>>,
    tabs: RefCell<HashMap<u32, Rc<Tab>>>,
    events: mpsc::Sender<(u32, FromWebClient)>,
    /// The bytes of the bodies of the keepalive requests under way.
    keepalive_bytes: Cell<usize>,
}

/// A keepalive request's share of the quota, given back when its task
/// ends, however it ends.
struct Quota(Rc<Shared>, usize);

impl Drop for Quota {
    fn drop(&mut self) {
        self.0.keepalive_bytes.set(self.0.keepalive_bytes.get().saturating_sub(self.1));
    }
}

impl Shared {
    /// Sends an event of the tab `tab` to the trusted core, waiting while
    /// the pipe is full. After the pipe has gone there is no one to tell.
    async fn send(&self, tab: u32, event: FromWebClient) {
        let _ = self.events.send((tab, event)).await;
    }

    /// Writes the jar out for the trusted core to keep, if it changed.
    async fn send_jar(&self) {
        if !self.engine.jar.borrow_mut().take_changed() {
            return;
        }
        for session in [false, true] {
            let written = self.engine.jar.borrow().write(session);
            let mut pieces = written.chunks(MAX_CHUNK_BYTES).peekable();
            if pieces.peek().is_none() {
                self.send(PROCESS_TAB, FromWebClient::Jar(JarPiece { session, last: true, bytes: Vec::new() })).await;
            }
            while let Some(piece) = pieces.next() {
                self.send(PROCESS_TAB, FromWebClient::Jar(JarPiece { session, last: pieces.peek().is_none(), bytes: piece.to_vec() })).await;
            }
        }
    }

    /// The request `key` has ended: it is no longer live. Its tab is kept,
    /// with what it has left of its rate, until the tab closes.
    fn ended(&self, key: Key) {
        self.live.borrow_mut().remove(&key);
    }

    /// Ends, unanswered, every request `which` picks. A keepalive request
    /// under way is not ended (the Fetch standard's keepalive flag, the
    /// script API's departure D9): it goes on for at most
    /// [`KEEPALIVE_DEADLINE`], and its answer is told to no one.
    fn end_all(&self, which: impl Fn(Key, &Live) -> bool) {
        self.live.borrow_mut().retain(|&key, live| {
            if !which(key, live) {
                return true;
            }
            match live.task.take() {
                Some(task) if live.keepalive => {
                    live.room.detach();
                    tokio::task::spawn_local(async move {
                        tokio::time::sleep(KEEPALIVE_DEADLINE).await;
                        task.abort();
                    });
                }
                Some(task) => task.abort(),
                None => {}
            }
            false
        });
    }

    /// Tells the trusted core an event of a request, unless the request
    /// outlived its page.
    async fn tell(&self, room: &Room, tab: u32, event: FromWebClient) {
        if !room.detached.get() {
            self.send(tab, event).await;
        }
    }
}

/// Whether trying an event stream again cannot help: the failure was a
/// rule's, not the network's (the HTML standard, section 9.2.3: a network
/// error is retried "unless the user agent knows that to be futile").
fn is_futile(reason: Reason) -> bool {
    !matches!(reason, Reason::Connection | Reason::Timeout | Reason::Protocol | Reason::Proxy | Reason::Coding)
}

/// A script's `fetch`: what was written of its body, the answer's head,
/// then its body as far as there is room for it.
async fn fetch_one(shared: &Shared, (tab, id): Key, start: &Start, body: Option<Bytes>, room: &Room) -> Result<(), Reason> {
    let fetched = fetch(&shared.engine, start, body).await;
    shared.send_jar().await;
    let Fetched { response_type, status, status_text, url, redirected, headers, body, sent } = fetched?;
    if let Some(bytes) = u32::try_from(sent).ok().filter(|&bytes| bytes > 0) {
        shared.tell(room, tab, FromWebClient::Sent { id, bytes }).await;
    }
    shared.tell(room, tab, FromWebClient::Head(Box::new(Head { id, response_type, status, status_text, url, redirected, headers }))).await;
    let Some(mut reader) = body else { return Ok(()) };
    // The time the trusted core takes to make room is not the server's.
    while let Some(bytes) = tokio::time::timeout(BODY_IDLE_TIMEOUT, reader.piece()).await.map_err(|_| Reason::Timeout)?? {
        room.take(bytes.len()).await;
        shared.tell(room, tab, FromWebClient::Body { id, bytes }).await;
    }
    Ok(())
}

/// Whether an answer's `Content-Type` is `text/event-stream`, whatever its
/// parameters.
fn is_event_stream(fetched: &Fetched) -> bool {
    fetched
        .headers
        .iter()
        .find(|(name, _)| name == "content-type")
        .is_some_and(|(_, value)| value.split(|&byte| byte == b';').next().unwrap_or_default().trim_ascii().eq_ignore_ascii_case(b"text/event-stream"))
}

/// A script's `EventSource` (the HTML standard, section 9.2): the stream
/// opened, its events sent on, and after a failure of the network opened
/// again with the last event's ID. It ends only by failing for good; the
/// script closes it by aborting.
async fn event_source(shared: &Shared, (tab, id): Key, start: &Start, room: &Room) -> Result<(), Reason> {
    let mut stream = EventStream::after(&start.last_event_id);
    let mut request = start.clone();
    // The standard's request: a `GET` that is never cached, read across
    // origins only by CORS. Seen from `fetch` it is a fetch.
    "GET".clone_into(&mut request.method);
    (request.kind, request.mode, request.cache, request.redirect, request.body) =
        (RequestKind::Fetch, Mode::Cors, CacheMode::NoStore, RedirectMode::Follow, BodySource::None);
    request.integrity = String::new();
    request.last_event_id = String::new();
    request.headers = vec![("accept".to_owned(), b"text/event-stream".to_vec())];
    loop {
        request.headers.retain(|(name, _)| name != "last-event-id");
        if !stream.last_id().is_empty() && is_header_value(stream.last_id().as_bytes()) {
            request.headers.push(("last-event-id".to_owned(), stream.last_id().as_bytes().to_vec()));
        }
        let fetched = fetch(&shared.engine, &request, None).await;
        shared.send_jar().await;
        match fetched {
            Ok(fetched) => {
                let streams = fetched.status == 200 && is_event_stream(&fetched);
                let Some(mut reader) = fetched.body.filter(|_| streams) else { return Err(Reason::NotEventStream) };
                // The address that answered, after redirects: its origin is
                // each event's (the HTML standard, section 9.2.6).
                shared.send(tab, FromWebClient::Opened { id, protocol: String::new(), url: fetched.url }).await;
                // Until the connection ends, however it ends.
                while let Ok(Some(bytes)) = reader.piece().await {
                    let mut events = Vec::new();
                    stream.push(&bytes, &mut |event| events.push(event)).map_err(|_| Reason::TooLarge)?;
                    for event in events {
                        room.take(event.data.len().saturating_add(event.kind.len()).saturating_add(event.last_id.len())).await;
                        shared.send(tab, FromWebClient::Event { id, event: event.kind, data: event.data, last_id: event.last_id }).await;
                    }
                }
            }
            Err(reason) if is_futile(reason) => return Err(reason),
            Err(_) => {}
        }
        stream.connection_ended();
        shared.send(tab, FromWebClient::Retrying(id)).await;
        let wait = stream.retry().map_or(DEFAULT_RETRY, Duration::from_millis).clamp(MIN_RETRY, MAX_RETRY);
        tokio::time::sleep(wait).await;
    }
}

/// The failure a script is told of for `reason`: a time limit's is the
/// standard's `TimeoutError`, any other a network error.
const fn failure(id: u32, reason: Reason) -> FromWebClient {
    let error = if matches!(reason, Reason::Timeout) { ErrorKind::Timeout } else { ErrorKind::Network };
    FromWebClient::Failed { id, error, reason }
}

/// One request, from its place among its tab's to its last event.
async fn serve(shared: Rc<Shared>, tab: Rc<Tab>, key: Key, start: Box<Start>, body: Option<Bytes>, room: Rc<Room>, socket: Option<Socket>) {
    let outcome = async {
        // The keepalive requests under way together have a bound on their
        // bodies, which the script host held first and may not have.
        let _quota = if start.keepalive && start.kind == RequestKind::Fetch {
            let bytes = body.as_ref().map_or(0, Bytes::len);
            let under_way = shared.keepalive_bytes.get().saturating_add(bytes);
            if under_way > MAX_KEEPALIVE_BODY_BYTES {
                return Err(Reason::TooLarge);
            }
            shared.keepalive_bytes.set(under_way);
            Some(Quota(Rc::clone(&shared), bytes))
        } else {
            None
        };
        let _place = tab.enter().await?;
        match (start.kind, socket) {
            (RequestKind::EventSource, _) => event_source(&shared, key, &start, &room).await.map(|()| FromWebClient::End(key.1)),
            (RequestKind::Fetch, _) => fetch_one(&shared, key, &start, body, &room).await.map(|()| FromWebClient::End(key.1)),
            // A socket's last event is its closing, which it returns.
            (RequestKind::WebSocket, Some((mut commands, queued))) => {
                let told = Teller { shared: &shared, room: &room, tab: key.0 };
                socket::run(&shared.engine, &start, &told, &mut commands, &queued).await
            }
            (RequestKind::WebSocket, None) => Err(Reason::Request),
        }
    }
    .await;
    // A request that outlived its page ends in silence, and its number
    // may by now be another request's.
    if room.detached.get() {
        return;
    }
    // No longer live before its last event is said, so an abort that
    // crosses it is answered by nothing more.
    shared.ended(key);
    shared.send(key.0, outcome.unwrap_or_else(|reason| failure(key.1, reason))).await;
}

/// The tab `tab`'s count and rate.
fn tab_of(shared: &Shared, tab: u32) -> Rc<Tab> {
    Rc::clone(shared.tabs.borrow_mut().entry(tab).or_insert_with(|| Rc::new(Tab::new())))
}

/// Starts the task of a request whose body, if it has one, is whole.
fn spawn(shared: &Rc<Shared>, key: Key, start: Box<Start>, body: Option<Vec<u8>>) {
    let tab = tab_of(shared, key.0);
    let mut live = shared.live.borrow_mut();
    let Some(entry) = live.get_mut(&key) else { return };
    // A socket's commands reach its task by a queue of its own.
    let socket = (start.kind == RequestKind::WebSocket).then(|| {
        let (commands, taken) = mpsc::unbounded_channel();
        let queued = Rc::new(Cell::new(0));
        entry.socket = Some((commands, Rc::clone(&queued)));
        (taken, queued)
    });
    entry.task = Some(tokio::task::spawn_local(serve(Rc::clone(shared), tab, key, start, body.map(Bytes::from), Rc::clone(&entry.room), socket)));
}

/// Fails a request that never became a task, or was stopped as one.
async fn refuse(shared: &Shared, key: Key, event: FromWebClient) {
    shared.ended(key);
    shared.send(key.0, event).await;
}

/// The bytes a request's body may still take: its stated length for
/// bytes, the bound for a stream.
fn stated(body: BodySource) -> usize {
    match body {
        BodySource::None => 0,
        BodySource::Bytes(length) => usize::try_from(length).unwrap_or(usize::MAX).min(MAX_REQUEST_BODY_BYTES),
        BodySource::Stream => MAX_REQUEST_BODY_BYTES,
    }
}

/// Does what one message for the tab `tab` asks.
///
/// # Errors
/// What ends the process: a kept jar past its bound.
async fn handle(shared: &Rc<Shared>, kept: &mut [Vec<u8>; 2], tab: u32, message: ToWebClient) -> Result<(), String> {
    match message {
        ToWebClient::Start(start) => {
            let key = (tab, start.id);
            // A number is its script host's choice, so one in use twice is
            // that host's fault alone and costs no one else: the second
            // start is not served, and the first goes on.
            if shared.live.borrow().contains_key(&key) {
                return Ok(());
            }
            let admitted = tab_of(shared, tab).admits(Instant::now());
            let room = Room::new();
            shared.live.borrow_mut().insert(
                key,
                Live { context: start.context, room, task: None, upload: None, keepalive: start.keepalive && start.kind == RequestKind::Fetch, socket: None },
            );
            if !admitted {
                refuse(shared, key, failure(start.id, Reason::Busy)).await;
            } else if start.body == BodySource::None {
                spawn(shared, key, start, None);
            } else if let Some(live) = shared.live.borrow_mut().get_mut(&key) {
                live.upload = Some((start, Vec::new()));
            }
        }
        ToWebClient::RequestBody { id, bytes } => {
            let key = (tab, id);
            let too_large = shared.live.borrow_mut().get_mut(&key).and_then(|live| live.upload.as_mut()).is_some_and(|(start, body)| {
                body.extend_from_slice(&bytes);
                body.len() > stated(start.body)
            });
            if too_large {
                refuse(shared, key, failure(id, Reason::TooLarge)).await;
            }
        }
        ToWebClient::RequestBodyEnd(id) => {
            let key = (tab, id);
            let upload = shared.live.borrow_mut().get_mut(&key).and_then(|live| live.upload.take());
            match upload {
                // Bytes of a stated length are that many, no fewer.
                Some((start, body)) if matches!(start.body, BodySource::Bytes(_)) && body.len() != stated(start.body) => {
                    refuse(shared, key, failure(id, Reason::Request)).await;
                }
                Some((start, body)) => spawn(shared, key, start, Some(body)),
                None => {}
            }
        }
        ToWebClient::Abort(id) => {
            let key = (tab, id);
            let task = shared.live.borrow_mut().get_mut(&key).map(|live| live.task.take());
            if let Some(task) = task {
                if let Some(task) = task {
                    task.abort();
                }
                refuse(shared, key, FromWebClient::Failed { id, error: ErrorKind::Abort, reason: Reason::Aborted }).await;
            }
        }
        ToWebClient::More { id, bytes } => {
            if let Some(live) = shared.live.borrow().get(&(tab, id)) {
                live.room.add(bytes);
            }
        }
        // A context's end, and a tab's, end their requests and say nothing
        // more of them; a keepalive request goes on, in silence.
        ToWebClient::ContextCloses(context) => shared.end_all(|key, live| key.0 == tab && live.context == context),
        ToWebClient::TabCloses => {
            shared.end_all(|key, _| key.0 == tab);
            shared.tabs.borrow_mut().remove(&tab);
        }
        ToWebClient::JarKept(piece) => {
            let part = kept.get_mut(usize::from(piece.session)).ok_or("jar part")?;
            part.extend_from_slice(&piece.bytes);
            if part.len() > MAX_JAR_BYTES {
                return Err("a kept jar past its bound".to_owned());
            }
            if piece.last {
                shared.engine.jar.borrow_mut().read_written(&core::mem::take(part));
            }
        }
        ToWebClient::ForgetJar => {
            // The jar and every state kept with it: the remembered
            // preflights, and the connections, which a server could know
            // the viewer by.
            shared.engine.jar.borrow_mut().forget();
            *shared.engine.preflights.borrow_mut() = PreflightCache::default();
            shared.engine.pool.forget();
            shared.send_jar().await;
        }
        ToWebClient::SocketSend { id, binary, last, bytes } => {
            let key = (tab, id);
            // What a script hands over faster than its socket takes it is
            // bounded by its host; a host that does not keep to that
            // loses the socket.
            let flooded = shared.live.borrow().get(&key).and_then(|live| live.socket.as_ref()).is_some_and(|(commands, queued)| {
                queued.set(queued.get().saturating_add(bytes.len()));
                queued.get() > socket::MAX_QUEUED_BYTES || commands.send(Command::Send { binary, last, bytes }).is_err()
            });
            if flooded {
                if let Some(task) = shared.live.borrow_mut().get_mut(&key).and_then(|live| live.task.take()) {
                    task.abort();
                }
                refuse(shared, key, FromWebClient::SocketClosed { id, code: neoteletext_web::consts::CLOSE_TOO_BIG, reason: String::new(), clean: false })
                    .await;
            }
        }
        ToWebClient::SocketClose { id, code, reason } => {
            if let Some((commands, _)) = shared.live.borrow().get(&(tab, id)).and_then(|live| live.socket.as_ref()) {
                // A socket that has ended by itself takes no more.
                let _ = commands.send(Command::Close { code, reason });
            }
        }
    }
    Ok(())
}

/// The loop: each message from the trusted core, until its pipe ends.
async fn serve_core(shared: Rc<Shared>, mut from_core: mpsc::Receiver<FromCore>) -> Result<(), String> {
    shared.send(PROCESS_TAB, FromWebClient::Confined).await;
    let mut kept = [Vec::new(), Vec::new()];
    while let Some(read) = from_core.recv().await {
        match read {
            FromCore::Message(tab, message) => handle(&shared, &mut kept, tab, message).await?,
            FromCore::End => break,
            FromCore::Broken(error) => return Err(error),
        }
    }
    Ok(())
}

/// What a web client process is told once, at its start, by the trusted
/// core alone: its partition's channel, the token every `Start` relayed
/// to it carries, and the address space its channel was loaded from.
/// Nothing of it is ever a script host's word.
pub struct Partition<'a> {
    /// The channel identity: the address of its folder.
    pub channel: &'a str,
    /// The partition's token.
    pub token: PartitionToken,
    /// The address space the channel was loaded from.
    pub space: AddressSpace,
}

/// Runs the web client for `partition` on the process's pipes, with the
/// addresses on this computer or the local network in `allowed` open to
/// any page (an address-space override, DEC-7315), and `proxy`.
///
/// # Errors
/// When the channel is no channel identity, the trusted core sends what
/// is no message or a start with another partition's token, or the
/// runtime cannot start.
pub fn run(allowed: &[IpAddr], proxy: Option<Proxy>, partition: &Partition<'_>) -> Result<(), String> {
    let settings = Settings::new(allowed, proxy, neoteletext_transport::mozilla_roots());
    run_with(settings, partition, None, Box::new(std::io::stdin()), Box::new(std::io::stdout()))
}

/// [`run`] with the connection settings, the public suffix list and the
/// pipes, which a test replaces.
pub(crate) fn run_with(
    settings: Settings,
    partition: &Partition<'_>,
    suffixes: Suffixes,
    input: Box<dyn Read + Send>,
    output: Box<dyn Write + Send>,
) -> Result<(), String> {
    let policy = ChannelPolicy::new(partition.channel).ok_or_else(|| format!("{}: not a channel identity", partition.channel))?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().map_err(|error| error.to_string())?;
    let (to_loop, from_core) = mpsc::channel(QUEUED_MESSAGES);
    let (events, mut to_core) = mpsc::channel::<(u32, FromWebClient)>(QUEUED_MESSAGES);
    let token = partition.token;
    // The pipes block, so each has a thread; the queues between hold a
    // bounded number of messages, and a full one holds its sender back.
    std::thread::spawn(move || {
        let mut input = BufReader::new(input);
        loop {
            // A start with another partition's token is refused by the
            // reader itself, and ends the process as any error does.
            let read = match read_to_client(&mut input, &token) {
                Ok(Some((tab, message))) => FromCore::Message(tab, message),
                Ok(None) => FromCore::End,
                Err(error) => FromCore::Broken(error.to_string()),
            };
            let last = !matches!(read, FromCore::Message(..));
            if to_loop.blocking_send(read).is_err() || last {
                break;
            }
        }
    });
    let writer = std::thread::spawn(move || {
        let mut output = BufWriter::new(output);
        while let Some((tab, event)) = to_core.blocking_recv() {
            // The events of one burst go in one write.
            if write_from_client(&mut output, tab, &event).is_err() || (to_core.is_empty() && output.flush().is_err()) {
                break;
            }
        }
        let _ = output.flush();
    });
    let shared = Rc::new(Shared {
        engine: Engine {
            policy,
            pool: Pool::new(settings),
            jar: RefCell::new(Jar::new(suffixes)),
            preflights: RefCell::new(PreflightCache::default()),
            space: partition.space,
        },
        live: RefCell::default(),
        tabs: RefCell::default(),
        events,
        keepalive_bytes: Cell::new(0),
    });
    let local = tokio::task::LocalSet::new();
    let ran = local.block_on(&runtime, serve_core(shared, from_core));
    // Every task ends with the set, and with them the last senders of
    // events, so the writer writes what is queued and ends.
    drop(local);
    drop(runtime);
    let _ = writer.join();
    ran
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Room is never more than its bound at once, however much is given,
    /// and a tab's starts are a burst and then a rate.
    #[test]
    fn room_and_starts_are_bounded() {
        let room = Room::new();
        for _ in 0..4 {
            room.add(MAX_NETWORK_BUFFER_BYTES);
        }
        assert_eq!(room.bytes.get(), MAX_NETWORK_BUFFER_BYTES);
        let tab = Tab::new();
        let now = Instant::now();
        assert_eq!((0..START_BURST * 2).filter(|_| tab.admits(now)).count(), START_BURST as usize, "the burst, and no more at once");
        let later = now + Duration::from_secs(1);
        assert_eq!((0..START_BURST).filter(|_| tab.admits(later)).count(), STARTS_PER_SECOND as usize, "a second's worth after a second");
        assert_eq!(
            (0..START_BURST * 2).filter(|_| tab.admits(now + Duration::from_secs(3600))).count(),
            START_BURST as usize,
            "never more than the burst saved up"
        );
    }
}

//! The web client process's loop: the trusted core's messages in, each
//! request a task of its own on the one thread, and the events out. A
//! process serves one partition (the browser's DEC-7268): a request for
//! another ends it, as a message that is no message does.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::IpAddr;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hyper::body::Bytes;
use neoteletext_web::proxy::Proxy;
use protocol::web::{
    BodySource, CacheMode, ErrorKind, FromWebClient, Head, INITIAL_ROOM, JarPiece, MAX_BODY_PIECE, MAX_JAR_BYTES, MAX_REQUEST_BODY, MAX_ROOM, Mode, Reason,
    RedirectMode, RequestKind, Start, ToWebClient, is_header_value, read_to_client, write_to_core,
};
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio::task::JoinHandle;

use super::connect::Settings;
use super::cookies::{Jar, Suffixes};
use super::cors::PreflightCache;
use super::fetch::{Engine, Fetched, fetch};
use super::limits::{
    BODY_IDLE_TIMEOUT, DEFAULT_RETRY, MAX_REQUESTS_PER_TAB, MAX_RETRY, MAX_WAITING_PER_TAB, MIN_RETRY, QUEUED_MESSAGES, START_BURST, STARTS_PER_SECOND,
};
use super::policy::ChannelPolicy;
use super::pool::Pool;
use super::sse::EventStream;

/// What the reader thread found on the pipe.
enum FromCore {
    Message(ToWebClient),
    /// The pipe ended.
    End,
    /// The pipe broke, or carried what is no message.
    Broken(String),
}

/// The room a request's answer has: the bytes the trusted core will take.
struct Room {
    bytes: Cell<u32>,
    more: Notify,
}

impl Room {
    /// Waits for room for `count` bytes, and takes it.
    async fn take(&self, count: usize) {
        let count = u32::try_from(count).unwrap_or(u32::MAX).clamp(1, INITIAL_ROOM);
        loop {
            let notified = self.more.notified();
            if let Some(left) = self.bytes.get().checked_sub(count) {
                self.bytes.set(left);
                return;
            }
            notified.await;
        }
    }

    fn add(&self, count: u32) {
        self.bytes.set(self.bytes.get().saturating_add(count).min(MAX_ROOM));
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

/// A request that has started and not ended.
struct Live {
    tab: u32,
    room: Rc<Room>,
    /// Its task, once its body is whole.
    task: Option<JoinHandle<()>>,
    /// Its start and the body so far, while the body is arriving.
    upload: Option<(Box<Start>, Vec<u8>)>,
}

/// What the loop and every request's task share.
struct Shared {
    engine: Engine,
    live: RefCell<HashMap<u32, Live>>,
    tabs: RefCell<HashMap<u32, Rc<Tab>>>,
    events: mpsc::Sender<FromWebClient>,
}

impl Shared {
    /// Sends an event to the trusted core, waiting while the pipe is full.
    /// After the pipe has gone there is no one to tell.
    async fn send(&self, event: FromWebClient) {
        let _ = self.events.send(event).await;
    }

    /// Writes the jar out for the trusted core to keep, if it changed.
    async fn send_jar(&self) {
        if !self.engine.jar.borrow_mut().take_changed() {
            return;
        }
        for session in [false, true] {
            let written = self.engine.jar.borrow().write(session);
            let mut pieces = written.chunks(MAX_BODY_PIECE).peekable();
            if pieces.peek().is_none() {
                self.send(FromWebClient::Jar(JarPiece { session, last: true, bytes: Vec::new() })).await;
            }
            while let Some(piece) = pieces.next() {
                self.send(FromWebClient::Jar(JarPiece { session, last: pieces.peek().is_none(), bytes: piece.to_vec() })).await;
            }
        }
    }

    /// The request `id` has ended: it is no longer live. Its tab is kept,
    /// with what it has left of its rate, until the tab closes.
    fn ended(&self, id: u32) {
        self.live.borrow_mut().remove(&id);
    }
}

/// Whether trying an event stream again cannot help: the failure was a
/// rule's, not the network's (the HTML standard, section 9.2.3: a network
/// error is retried "unless the user agent knows that to be futile").
fn is_futile(reason: Reason) -> bool {
    !matches!(reason, Reason::Connection | Reason::Timeout | Reason::Protocol | Reason::Proxy | Reason::Coding)
}

/// A script's `fetch`, or the browser's page load: the answer's head, then
/// its body as far as there is room for it.
async fn fetch_one(shared: &Shared, start: &Start, body: Option<Bytes>, room: &Room) -> Result<(), Reason> {
    let fetched = fetch(&shared.engine, start, body).await;
    shared.send_jar().await;
    let Fetched { response_type, status, status_text, url, redirected, headers, body } = fetched?;
    shared.send(FromWebClient::Head(Box::new(Head { id: start.id, response_type, status, status_text, url, redirected, headers }))).await;
    let Some(mut reader) = body else { return Ok(()) };
    // The time the trusted core takes to make room is not the server's.
    while let Some(bytes) = tokio::time::timeout(BODY_IDLE_TIMEOUT, reader.piece()).await.map_err(|_| Reason::Timeout)?? {
        room.take(bytes.len()).await;
        shared.send(FromWebClient::Body { id: start.id, bytes }).await;
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
async fn event_source(shared: &Shared, start: &Start, room: &Room) -> Result<(), Reason> {
    let mut stream = EventStream::default();
    let mut request = start.clone();
    // The standard's request: a `GET` that is never cached, read across
    // origins only by CORS.
    "GET".clone_into(&mut request.method);
    (request.kind, request.mode, request.cache, request.redirect, request.body) =
        (RequestKind::Fetch, Mode::Cors, CacheMode::NoStore, RedirectMode::Follow, BodySource::None);
    request.integrity = String::new();
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
                shared.send(FromWebClient::Opened { id: start.id, text: fetched.url }).await;
                // Until the connection ends, however it ends.
                while let Ok(Some(bytes)) = reader.piece().await {
                    let mut events = Vec::new();
                    stream.push(&bytes, &mut |event| events.push(event)).map_err(|_| Reason::TooLarge)?;
                    for event in events {
                        room.take(event.data.len().saturating_add(event.kind.len()).saturating_add(event.last_id.len())).await;
                        shared.send(FromWebClient::Event { id: start.id, event: event.kind, data: event.data, last_id: event.last_id }).await;
                    }
                }
            }
            Err(reason) if is_futile(reason) => return Err(reason),
            Err(_) => {}
        }
        stream.connection_ended();
        shared.send(FromWebClient::Retrying(start.id)).await;
        let wait = stream.retry().map_or(DEFAULT_RETRY, Duration::from_millis).clamp(MIN_RETRY, MAX_RETRY);
        tokio::time::sleep(wait).await;
    }
}

/// One request, from its place among its tab's to its last event.
async fn serve(shared: Rc<Shared>, tab: Rc<Tab>, start: Box<Start>, body: Option<Bytes>, room: Rc<Room>) {
    let outcome = async {
        let _place = tab.enter().await?;
        match start.kind {
            RequestKind::EventSource => event_source(&shared, &start, &room).await,
            RequestKind::Fetch | RequestKind::Page => fetch_one(&shared, &start, body, &room).await,
            RequestKind::WebSocket => Err(Reason::Unsupported),
        }
    }
    .await;
    // No longer live before its last event is said, so an abort that
    // crosses it is answered by nothing more.
    shared.ended(start.id);
    shared
        .send(match outcome {
            Ok(()) => FromWebClient::End(start.id),
            Err(reason) => FromWebClient::Failed { id: start.id, error: ErrorKind::Network, reason },
        })
        .await;
}

/// Starts the task of a request whose body, if it has one, is whole.
fn spawn(shared: &Rc<Shared>, start: Box<Start>, body: Option<Vec<u8>>) {
    let id = start.id;
    let tab = Rc::clone(shared.tabs.borrow_mut().entry(start.tab).or_insert_with(|| Rc::new(Tab::new())));
    let mut live = shared.live.borrow_mut();
    let Some(entry) = live.get_mut(&id) else { return };
    entry.task = Some(tokio::task::spawn_local(serve(Rc::clone(shared), tab, start, body.map(Bytes::from), Rc::clone(&entry.room))));
}

/// Fails a request that never became a task.
async fn refuse(shared: &Shared, id: u32, error: ErrorKind, reason: Reason) {
    shared.ended(id);
    shared.send(FromWebClient::Failed { id, error, reason }).await;
}

/// Does what one message from the trusted core asks.
///
/// # Errors
/// What ends the process: a request for another partition, a request
/// number already in use, or a kept jar past its bound.
async fn handle(shared: &Rc<Shared>, kept: &mut [Vec<u8>; 2], message: ToWebClient) -> Result<(), String> {
    match message {
        ToWebClient::Start(start) => {
            if !shared.engine.policy.holds(start.jar, &start.channel) {
                return Err("a request for another partition".to_owned());
            }
            let id = start.id;
            if shared.live.borrow().contains_key(&id) {
                return Err(format!("request {id} started twice"));
            }
            let admitted = Rc::clone(shared.tabs.borrow_mut().entry(start.tab).or_insert_with(|| Rc::new(Tab::new()))).admits(Instant::now());
            shared
                .live
                .borrow_mut()
                .insert(id, Live { tab: start.tab, room: Rc::new(Room { bytes: Cell::new(INITIAL_ROOM), more: Notify::new() }), task: None, upload: None });
            if start.kind == RequestKind::WebSocket {
                refuse(shared, id, ErrorKind::Network, Reason::Unsupported).await;
            } else if !admitted {
                refuse(shared, id, ErrorKind::Network, Reason::Busy).await;
            } else if start.body == BodySource::None {
                spawn(shared, start, None);
            } else if let Some(live) = shared.live.borrow_mut().get_mut(&id) {
                live.upload = Some((start, Vec::new()));
            }
        }
        ToWebClient::RequestBody { id, bytes } => {
            let too_large = shared.live.borrow_mut().get_mut(&id).and_then(|live| live.upload.as_mut()).is_some_and(|(_, body)| {
                body.extend_from_slice(&bytes);
                body.len() > MAX_REQUEST_BODY
            });
            if too_large {
                refuse(shared, id, ErrorKind::Network, Reason::TooLarge).await;
            }
        }
        ToWebClient::RequestBodyEnd(id) => {
            let upload = shared.live.borrow_mut().get_mut(&id).and_then(|live| live.upload.take());
            if let Some((start, body)) = upload {
                spawn(shared, start, Some(body));
            }
        }
        ToWebClient::Abort(id) => {
            let task = shared.live.borrow_mut().get_mut(&id).map(|live| live.task.take());
            if let Some(task) = task {
                if let Some(task) = task {
                    task.abort();
                }
                refuse(shared, id, ErrorKind::Abort, Reason::Aborted).await;
            }
        }
        ToWebClient::More { id, bytes } => {
            if let Some(live) = shared.live.borrow().get(&id) {
                live.room.add(bytes);
            }
        }
        ToWebClient::TabCloses(tab) => {
            shared.live.borrow_mut().retain(|_, live| {
                if live.tab != tab {
                    return true;
                }
                if let Some(task) = live.task.take() {
                    task.abort();
                }
                false
            });
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
        // Reserved: no WebSocket is ever live, so there is none to act on.
        ToWebClient::SocketSend { .. } | ToWebClient::SocketClose { .. } => {}
    }
    Ok(())
}

/// The loop: each message from the trusted core, until its pipe ends.
async fn serve_core(shared: Rc<Shared>, mut from_core: mpsc::Receiver<FromCore>) -> Result<(), String> {
    shared.send(FromWebClient::Confined).await;
    let mut kept = [Vec::new(), Vec::new()];
    while let Some(read) = from_core.recv().await {
        match read {
            FromCore::Message(message) => handle(&shared, &mut kept, message).await?,
            FromCore::End => break,
            FromCore::Broken(error) => return Err(error),
        }
    }
    Ok(())
}

/// Runs the web client for the partition of the jar of windows `windows`
/// and the channel identity `channel`, on the process's pipes, with the
/// addresses on this computer or the local network in `allowed` open to
/// any page (the viewer's standing permission, DEC-7315), and `proxy`.
///
/// # Errors
/// When `channel` is no channel identity, the trusted core sends what is
/// no message or a request for another partition, or the runtime cannot
/// start.
pub fn run(allowed: &[IpAddr], proxy: Option<Proxy>, windows: u32, channel: &str) -> Result<(), String> {
    let settings = Settings::new(allowed, proxy, neoteletext_transport::mozilla_roots());
    run_with(settings, windows, channel, None, Box::new(std::io::stdin()), Box::new(std::io::stdout()))
}

/// [`run`] with the connection settings, the public suffix list and the
/// pipes, which a test replaces.
pub(crate) fn run_with(
    settings: Settings,
    windows: u32,
    channel: &str,
    suffixes: Suffixes,
    input: Box<dyn Read + Send>,
    output: Box<dyn Write + Send>,
) -> Result<(), String> {
    let policy = ChannelPolicy::new(windows, channel).ok_or_else(|| format!("{channel}: not a channel identity"))?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().map_err(|error| error.to_string())?;
    let (to_loop, from_core) = mpsc::channel(QUEUED_MESSAGES);
    let (events, mut to_core) = mpsc::channel::<FromWebClient>(QUEUED_MESSAGES);
    // The pipes block, so each has a thread; the queues between hold a
    // bounded number of messages, and a full one holds its sender back.
    std::thread::spawn(move || {
        let mut input = BufReader::new(input);
        loop {
            let read = match read_to_client(&mut input) {
                Ok(Some(message)) => FromCore::Message(message),
                Ok(None) => FromCore::End,
                Err(error) => FromCore::Broken(error.to_string()),
            };
            let last = !matches!(read, FromCore::Message(_));
            if to_loop.blocking_send(read).is_err() || last {
                break;
            }
        }
    });
    let writer = std::thread::spawn(move || {
        let mut output = BufWriter::new(output);
        while let Some(event) = to_core.blocking_recv() {
            // The events of one burst go in one write.
            if write_to_core(&mut output, &event).is_err() || (to_core.is_empty() && output.flush().is_err()) {
                break;
            }
        }
        let _ = output.flush();
    });
    let shared = Rc::new(Shared {
        engine: Engine { policy, pool: Pool::new(settings), jar: RefCell::new(Jar::new(suffixes)), preflights: RefCell::new(PreflightCache::default()) },
        live: RefCell::default(),
        tabs: RefCell::default(),
        events,
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

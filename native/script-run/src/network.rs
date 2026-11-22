//! The network a run's scripts reach (`--network`, script-api.md section
//! 16), behind one interface, [`Network`]: what a script host writes to a
//! web client goes in, tick by tick, and what a web client would answer
//! comes out. Both are the web client's own messages
//! (`protocol::web`, section 11.3), so that the script library is given
//! exactly what the browser's relay would give it, and a mode differs
//! from the real web client only in where its answers come from.
//!
//! - [`Refusing`], `--network none`: every request fails at once;
//! - [`Table`], `--network mock FILE` and `--network replay FILE`: answers
//!   from a file, each placed on a tick; no socket is opened and no clock
//!   is read, so a run is the same every time;
//! - [`Recorder`], `--network record FILE`: another network's answers
//!   passed through, and written as a file a replay reads.
//!
//! # The file
//!
//! One form serves a mock table and a recording. A line that is empty or
//! begins with `#` is skipped; every other line is a word and its fields.
//!
//! ```text
//! request fetch GET https://channel.test/tides.json
//! +1 head 200 type=basic text=OK
//! header content-type: application/json
//! +1 body {"height":3}\n
//! +2 end
//!
//! request events GET https://channel.test/feed
//! +1 opened
//! +30 event message - first
//! +90 event message 7 second
//!
//! request socket GET wss://channel.test/chat
//! +1 opened chat.v1
//! +5 message text hello
//! +60 closed 1000 clean bye
//! ```
//!
//! `request KIND METHOD URL` begins an entry: the kind is `fetch`,
//! `events` or `socket`. Its steps follow, each `+N` ticks after the tick
//! the script asked on (an answer is given in the first step after its
//! tick comes), in an order that never goes back:
//!
//! | Step | The web client's message |
//! |---|---|
//! | `head STATUS [type=basic\|cors\|opaqueredirect] [url=URL] [redirected] [text=STATUS-TEXT]` | the answer's head; `header NAME: VALUE` lines after it are its headers; the address defaults to the request's |
//! | `body BYTES` | a piece of the body |
//! | `end` | the body's end |
//! | `fail network\|abort\|timeout [REASON]` | the request failed; the reason is its number in `protocol::web::Reason`, for the log |
//! | `opened [PROTOCOL]` | an event stream or a socket opened |
//! | `event TYPE ID DATA` | an event of a stream; `-` for no type or no id |
//! | `retrying` | an event stream reconnecting |
//! | `message text\|binary\|text-more\|binary-more BYTES` | a socket's message, or a piece of one with more to come |
//! | `closed CODE clean\|unclean [REASON]` | a socket closed |
//!
//! `BYTES`, `DATA` and a status text are the rest of the line, with
//! `\n`, `\t`, `\\` and `\xHH` for any byte.
//!
//! A request is answered by the first entry of its kind, method and
//! address not yet used. When all are used a mock answers with the last
//! of them again, so one entry serves any number of the same request and
//! several serve in turn; a replay does not, and the request fails: a
//! recording answers exactly the requests that were made.

use protocol::web::{ErrorKind, FromWebClient, Head, Reason, RequestKind, ResponseType, Start, ToWebClient, write_answer};

use crate::limits::{MAX_NETWORK_ENTRIES, MAX_NETWORK_FILE_BYTES, MAX_NETWORK_LINE_BYTES, MAX_NETWORK_STEPS, MAX_TICKS};

/// What a network has done in a run, for the report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// The requests scripts made: fetches, event streams and sockets.
    pub requests: u64,
    /// Those no entry answered, which failed.
    pub unanswered: u64,
    /// The answers given to scripts.
    pub answers: u64,
}

/// The network of a run. A mode that needs sockets or a clock is not
/// deterministic and says so with [`Network::deterministic`].
pub trait Network {
    /// A message a script host wrote during the step of `tick`.
    fn request(&mut self, tick: u64, message: ToWebClient);
    /// The answers that have arrived for the step of `tick`, in order.
    fn answers(&mut self, tick: u64) -> Vec<FromWebClient>;
    /// What it has done.
    fn counts(&self) -> Counts;
    /// Whether the same requests always get the same answers on the same
    /// ticks.
    fn deterministic(&self) -> bool {
        true
    }
    /// The file a recording mode has made.
    fn recording(&self) -> Option<Vec<u8>> {
        None
    }
}

/// `--network none`: every request fails as a network error, in the next
/// step.
#[derive(Debug, Default)]
pub struct Refusing {
    failed: Vec<u32>,
    counts: Counts,
}

impl Network for Refusing {
    fn request(&mut self, _tick: u64, message: ToWebClient) {
        if let ToWebClient::Start(start) = message {
            self.counts.requests = self.counts.requests.saturating_add(1);
            self.counts.unanswered = self.counts.unanswered.saturating_add(1);
            self.failed.push(start.id);
        }
    }

    fn answers(&mut self, _tick: u64) -> Vec<FromWebClient> {
        let answers: Vec<FromWebClient> =
            self.failed.drain(..).map(|id| FromWebClient::Failed { id, error: ErrorKind::Network, reason: Reason::Unsupported }).collect();
        self.counts.answers = self.counts.answers.saturating_add(u64::try_from(answers.len()).unwrap_or(u64::MAX));
        answers
    }

    fn counts(&self) -> Counts {
        self.counts
    }
}

/// One step of an entry: an answer without its request's number, and the
/// ticks after the request it is given on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// The ticks after the request.
    pub after: u64,
    /// The answer, its `id` to be set to the request's.
    pub answer: FromWebClient,
}

/// One entry of a file: a request and its answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The request's kind.
    pub kind: RequestKind,
    /// Its method.
    pub method: String,
    /// Its address.
    pub url: String,
    /// The answers, in order.
    pub replies: Vec<Reply>,
}

const fn kind_word(kind: RequestKind) -> &'static str {
    match kind {
        RequestKind::Fetch => "fetch",
        RequestKind::EventSource => "events",
        RequestKind::WebSocket => "socket",
    }
}

/// Bytes as a file writes them: printable ASCII as it is, everything else
/// escaped.
#[must_use]
pub fn escaped(bytes: &[u8]) -> String {
    escaped_but(bytes, |_| false)
}

/// Bytes as a file writes them where the reader splits or trims at
/// spaces: every space escaped, so that the same bytes are read back.
fn escaped_word(bytes: &[u8]) -> String {
    escaped_but(bytes, |_| true)
}

/// [`escaped`], with the space at each index `spaced` names escaped too.
fn escaped_but(bytes: &[u8], spaced: impl Fn(usize) -> bool) -> String {
    use core::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len());
    for (index, &byte) in bytes.iter().enumerate() {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b' ' if spaced(index) => out.push_str("\\x20"),
            0x20..=0x7E => out.push(char::from(byte)),
            other => {
                let _ = write!(out, "\\x{other:02x}");
            }
        }
    }
    out
}

/// The bytes a file's text stands for.
///
/// # Errors
/// An escape that is none of `\n`, `\t`, `\\` and `\xHH`.
pub fn unescaped(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        match bytes.next() {
            Some(b'n') => out.push(b'\n'),
            Some(b't') => out.push(b'\t'),
            Some(b'\\') => out.push(b'\\'),
            Some(b'x') => {
                let digit = |byte: Option<u8>| byte.and_then(|byte| char::from(byte).to_digit(16)).and_then(|digit| u8::try_from(digit).ok());
                let (Some(high), Some(low)) = (digit(bytes.next()), digit(bytes.next())) else {
                    return Err("`\\x` is followed by two hexadecimal digits".into());
                };
                out.push((high << 4) | low);
            }
            _ => return Err("a `\\` is followed by `n`, `t`, `\\` or `xHH`".into()),
        }
    }
    Ok(out)
}

fn utf8(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|_| "text that is not UTF-8".into())
}

/// The answer with its request's number set.
fn numbered(answer: &FromWebClient, id: u32) -> FromWebClient {
    let mut answer = answer.clone();
    match &mut answer {
        FromWebClient::Head(head) => head.id = id,
        FromWebClient::Body { id: held, .. }
        | FromWebClient::Failed { id: held, .. }
        | FromWebClient::Opened { id: held, .. }
        | FromWebClient::Event { id: held, .. }
        | FromWebClient::SocketMessage { id: held, .. }
        | FromWebClient::SocketClosed { id: held, .. }
        | FromWebClient::Sent { id: held, .. }
        | FromWebClient::End(held)
        | FromWebClient::Retrying(held) => *held = id,
        FromWebClient::Confined | FromWebClient::Jar(_) => {}
    }
    answer
}

/// The request number of an answer, if it has one.
fn id_of(answer: &FromWebClient) -> Option<u32> {
    match answer {
        FromWebClient::Head(head) => Some(head.id),
        FromWebClient::Body { id, .. }
        | FromWebClient::Failed { id, .. }
        | FromWebClient::Opened { id, .. }
        | FromWebClient::Event { id, .. }
        | FromWebClient::SocketMessage { id, .. }
        | FromWebClient::SocketClosed { id, .. }
        | FromWebClient::Sent { id, .. }
        | FromWebClient::End(id)
        | FromWebClient::Retrying(id) => Some(*id),
        FromWebClient::Confined | FromWebClient::Jar(_) => None,
    }
}

/// The step of a line's word and fields.
fn reply(word: &str, rest: &str) -> Result<FromWebClient, String> {
    let words: Vec<&str> = rest.split_ascii_whitespace().collect();
    match word {
        "head" => {
            let (status, options) = words.split_first().ok_or("a head line is `head STATUS` and its options")?;
            let status = status.parse::<u16>().map_err(|_| format!("the status `{status}` is not a whole number"))?;
            let mut head = Head {
                id: 0,
                response_type: ResponseType::Basic,
                status,
                status_text: String::new(),
                url: String::new(),
                redirected: false,
                headers: Vec::new(),
            };
            for (index, option) in options.iter().enumerate() {
                match option.split_once('=') {
                    None if *option == "redirected" => head.redirected = true,
                    Some(("type", "basic")) => head.response_type = ResponseType::Basic,
                    Some(("type", "cors")) => head.response_type = ResponseType::Cors,
                    Some(("type", "opaqueredirect")) => head.response_type = ResponseType::OpaqueRedirect,
                    Some(("url", url)) => url.clone_into(&mut head.url),
                    // The status text is the rest of the line: it may hold spaces.
                    Some(("text", first)) => {
                        let mut text = String::from(first);
                        for more in options.iter().skip(index.saturating_add(1)) {
                            text.push(' ');
                            text.push_str(more);
                        }
                        head.status_text = utf8(unescaped(&text)?)?;
                        break;
                    }
                    _ => return Err(format!("`{option}` is not an option of a head line")),
                }
            }
            Ok(FromWebClient::Head(Box::new(head)))
        }
        "body" => Ok(FromWebClient::Body { id: 0, bytes: unescaped(rest)? }),
        "end" => Ok(FromWebClient::End(0)),
        "fail" => {
            let (kind, reason) = match words.as_slice() {
                [kind] => (*kind, None),
                [kind, reason] => (*kind, Some(*reason)),
                _ => return Err("a fail line is `fail network|abort|timeout [REASON]`".into()),
            };
            let error = match kind {
                "network" => ErrorKind::Network,
                "abort" => ErrorKind::Abort,
                "timeout" => ErrorKind::Timeout,
                other => return Err(format!("`{other}` is not an error kind: network, abort or timeout")),
            };
            let reason = match reason {
                None => Reason::Connection,
                Some(code) => code.parse::<u8>().ok().and_then(|code| Reason::of(code).ok()).ok_or_else(|| format!("`{code}` is not a reason's number"))?,
            };
            Ok(FromWebClient::Failed { id: 0, error, reason })
        }
        "opened" => Ok(FromWebClient::Opened { id: 0, protocol: rest.trim().to_owned(), url: String::new() }),
        "event" => {
            let (kind, after) = rest.split_once(' ').unwrap_or((rest, ""));
            let (last_id, data) = after.split_once(' ').unwrap_or((after, ""));
            let named = |word: &str| if word == "-" { String::new() } else { word.to_owned() };
            Ok(FromWebClient::Event { id: 0, event: named(kind), data: utf8(unescaped(data)?)?, last_id: named(last_id) })
        }
        "retrying" => Ok(FromWebClient::Retrying(0)),
        "message" => {
            let (form, data) = rest.split_once(' ').unwrap_or((rest, ""));
            let (binary, last) = match form {
                "text" => (false, true),
                "binary" => (true, true),
                "text-more" => (false, false),
                "binary-more" => (true, false),
                other => return Err(format!("`{other}` is not a message's form: text, binary, text-more or binary-more")),
            };
            Ok(FromWebClient::SocketMessage { id: 0, binary, last, bytes: unescaped(data)? })
        }
        "closed" => {
            let (code, after) = rest.split_once(' ').unwrap_or((rest, ""));
            let (clean, reason) = after.split_once(' ').unwrap_or((after, ""));
            let code = code.parse::<u16>().map_err(|_| format!("the close code `{code}` is not a whole number"))?;
            let clean = match clean {
                "clean" => true,
                "unclean" => false,
                other => return Err(format!("`{other}` is neither `clean` nor `unclean`")),
            };
            Ok(FromWebClient::SocketClosed { id: 0, code, reason: utf8(unescaped(reason)?)?, clean })
        }
        other => Err(format!("`{other}` is not a step of a network file")),
    }
}

/// Reads a network file.
///
/// # Errors
/// The first line that is wrong, by its number, and why. Every answer is
/// checked as the web client's own writer checks it, so that a file can
/// hold nothing the real web client could not send.
pub fn parse(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    if bytes.len() > MAX_NETWORK_FILE_BYTES {
        return Err(format!("the network file is longer than {MAX_NETWORK_FILE_BYTES} bytes"));
    }
    let text = core::str::from_utf8(bytes).map_err(|_| "the network file is not UTF-8")?;
    let mut entries: Vec<Entry> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index.saturating_add(1);
        let fail = |why: String| format!("line {number} of the network file: {why}");
        if line.len() > MAX_NETWORK_LINE_BYTES {
            return Err(fail(format!("longer than {MAX_NETWORK_LINE_BYTES} bytes")));
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        if word == "request" {
            let words: Vec<&str> = rest.split_ascii_whitespace().collect();
            let [kind, method, url] = words.as_slice() else { return Err(fail("a request line is `request KIND METHOD URL`".into())) };
            let (kind, method, url) = (*kind, *method, *url);
            let kind = match kind {
                "fetch" => RequestKind::Fetch,
                "events" => RequestKind::EventSource,
                "socket" => RequestKind::WebSocket,
                other => return Err(fail(format!("`{other}` is not a request's kind: fetch, events or socket"))),
            };
            if entries.len() >= MAX_NETWORK_ENTRIES {
                return Err(fail(format!("more than {MAX_NETWORK_ENTRIES} requests")));
            }
            entries.push(Entry { kind, method: method.to_owned(), url: url.to_owned(), replies: Vec::new() });
            continue;
        }
        let entry = entries.last_mut().ok_or_else(|| fail("a step comes before any `request` line".into()))?;
        if word == "header" {
            let (name, value) = rest.split_once(':').ok_or_else(|| fail("a header line is `header NAME: VALUE`".into()))?;
            let Some(Reply { answer: FromWebClient::Head(head), .. }) = entry.replies.last_mut() else {
                return Err(fail("a header line follows a head line".into()));
            };
            head.headers.push((name.trim().to_owned(), unescaped(value.trim()).map_err(fail)?));
            continue;
        }
        let after = word.strip_prefix('+').and_then(|ticks| ticks.parse::<u64>().ok()).filter(|ticks| *ticks <= MAX_TICKS);
        let after = after.ok_or_else(|| fail(format!("`{word}` is not `request`, `header` or `+N`, the ticks after the request")))?;
        if entry.replies.last().is_some_and(|last| last.after > after) {
            return Err(fail("the steps of a request never go back in time".into()));
        }
        if entry.replies.len() >= MAX_NETWORK_STEPS {
            return Err(fail(format!("more than {MAX_NETWORK_STEPS} steps of one request")));
        }
        let (step, fields) = rest.split_once(' ').unwrap_or((rest, ""));
        let mut answer = reply(step, fields).map_err(fail)?;
        if let FromWebClient::Head(head) = &mut answer
            && head.url.is_empty()
        {
            head.url.clone_from(&entry.url);
        }
        entry.replies.push(Reply { after, answer });
    }
    // Checked whole, once its headers are on: by the web client's writer.
    for entry in &entries {
        for step in &entry.replies {
            write_answer(&mut std::io::sink(), &step.answer)
                .map_err(|error| format!("the network file's answers to {} {}: {error:?}", entry.method, entry.url))?;
        }
    }
    Ok(entries)
}

/// The file of `entries`: what [`parse`] reads back as the same entries.
#[must_use]
pub fn write(entries: &[Entry]) -> Vec<u8> {
    use core::fmt::Write as _;
    let mut out = String::new();
    for entry in entries {
        let _ = writeln!(out, "request {} {} {}", kind_word(entry.kind), entry.method, entry.url);
        for Reply { after, answer } in &entry.replies {
            let _ = match answer {
                FromWebClient::Head(head) => {
                    let kind = match head.response_type {
                        ResponseType::Basic => "basic",
                        ResponseType::Cors => "cors",
                        ResponseType::OpaqueRedirect => "opaqueredirect",
                    };
                    let _ = write!(out, "+{after} head {} type={kind}", head.status);
                    if !head.url.is_empty() {
                        let _ = write!(out, " url={}", head.url);
                    }
                    if head.redirected {
                        out.push_str(" redirected");
                    }
                    if !head.status_text.is_empty() {
                        let _ = write!(out, " text={}", escaped_word(head.status_text.as_bytes()));
                    }
                    out.push('\n');
                    for (name, value) in &head.headers {
                        let _ = writeln!(out, "header {name}: {}", escaped_word(value));
                    }
                    Ok(())
                }
                FromWebClient::Body { bytes, .. } => writeln!(out, "+{after} body {}", escaped(bytes)),
                FromWebClient::End(_) => writeln!(out, "+{after} end"),
                FromWebClient::Failed { error, reason, .. } => {
                    let kind = match error {
                        ErrorKind::Network => "network",
                        ErrorKind::Abort => "abort",
                        ErrorKind::Timeout => "timeout",
                    };
                    writeln!(out, "+{after} fail {kind} {}", reason.code())
                }
                FromWebClient::Opened { protocol, .. } => writeln!(out, "+{after} opened {protocol}"),
                FromWebClient::Event { event, data, last_id, .. } => {
                    let named = |word: &str| if word.is_empty() { "-".to_owned() } else { word.to_owned() };
                    writeln!(out, "+{after} event {} {} {}", named(event), named(last_id), escaped(data.as_bytes()))
                }
                FromWebClient::Retrying(_) => writeln!(out, "+{after} retrying"),
                FromWebClient::SocketMessage { binary, last, bytes, .. } => {
                    let form = match (binary, last) {
                        (false, true) => "text",
                        (true, true) => "binary",
                        (false, false) => "text-more",
                        (true, false) => "binary-more",
                    };
                    writeln!(out, "+{after} message {form} {}", escaped(bytes))
                }
                FromWebClient::SocketClosed { code, reason, clean, .. } => {
                    writeln!(out, "+{after} closed {code} {} {}", if *clean { "clean" } else { "unclean" }, escaped(reason.as_bytes()))
                }
                // Flow control and the cookie jar are the web client's own
                // business with its relay, and no part of a file.
                FromWebClient::Sent { .. } | FromWebClient::Confined | FromWebClient::Jar(_) => Ok(()),
            };
        }
        out.push('\n');
    }
    out.into_bytes()
}

/// An answer waiting for its tick.
#[derive(Debug)]
struct Due {
    tick: u64,
    answer: FromWebClient,
}

/// `--network mock FILE` and `--network replay FILE`: answers from a
/// file's entries.
#[derive(Debug)]
pub struct Table {
    entries: Vec<Entry>,
    used: Vec<bool>,
    /// A replay: an entry answers one request and no more.
    replay: bool,
    due: Vec<Due>,
    counts: Counts,
}

impl Table {
    /// A table of `entries`; a `replay` uses each once.
    #[must_use]
    pub fn new(entries: Vec<Entry>, replay: bool) -> Self {
        let used = vec![false; entries.len()];
        Self { entries, used, replay, due: Vec::new(), counts: Counts::default() }
    }

    /// The entry that answers `start`: the first of its kind, method and
    /// address not yet used, or for a mock the last of them.
    fn entry_for(&mut self, start: &Start) -> Option<&Entry> {
        let matches = |entry: &Entry| entry.kind == start.kind && entry.method == start.method && entry.url == start.url;
        let unused = self.entries.iter().zip(&self.used).position(|(entry, used)| !used && matches(entry));
        let index = match unused {
            Some(index) => Some(index),
            None if self.replay => None,
            None => self.entries.iter().rposition(matches),
        }?;
        if let Some(used) = self.used.get_mut(index) {
            *used = true;
        }
        self.entries.get(index)
    }

    fn push(&mut self, tick: u64, answer: FromWebClient) {
        // Kept in the order of their ticks, and within a tick in the
        // order they were made.
        let at = self.due.partition_point(|due| due.tick <= tick);
        self.due.insert(at, Due { tick, answer });
    }
}

impl Network for Table {
    fn request(&mut self, tick: u64, message: ToWebClient) {
        match message {
            ToWebClient::Start(start) => {
                self.counts.requests = self.counts.requests.saturating_add(1);
                let id = start.id;
                if let Some(replies) = self.entry_for(&start).map(|entry| entry.replies.clone()) {
                    for step in replies {
                        self.push(tick.saturating_add(step.after), numbered(&step.answer, id));
                    }
                } else {
                    self.counts.unanswered = self.counts.unanswered.saturating_add(1);
                    self.push(tick, FromWebClient::Failed { id, error: ErrorKind::Network, reason: Reason::Connection });
                }
            }
            // A request the script gave up gets nothing more.
            ToWebClient::Abort(id) => self.due.retain(|due| id_of(&due.answer) != Some(id)),
            // What a socket sent is taken at once, as a web client with
            // room says it is, so that the script's count of bytes waiting
            // falls.
            ToWebClient::SocketSend { id, bytes, .. } => {
                self.push(tick, FromWebClient::Sent { id, bytes: u32::try_from(bytes.len()).unwrap_or(u32::MAX) });
            }
            ToWebClient::SocketClose { id, code, reason } => {
                self.due.retain(|due| id_of(&due.answer) != Some(id));
                self.push(tick, FromWebClient::SocketClosed { id, code, reason, clean: true });
            }
            ToWebClient::RequestBody { .. }
            | ToWebClient::RequestBodyEnd(_)
            | ToWebClient::More { .. }
            | ToWebClient::ContextCloses(_)
            | ToWebClient::TabCloses
            | ToWebClient::JarKept(_)
            | ToWebClient::ForgetJar => {}
        }
    }

    fn answers(&mut self, tick: u64) -> Vec<FromWebClient> {
        // An answer is given in the first step after its tick comes: a
        // request made in one step is never answered in the same one.
        let ready = self.due.partition_point(|due| due.tick < tick);
        let answers: Vec<FromWebClient> = self.due.drain(..ready).map(|due| due.answer).collect();
        self.counts.answers = self.counts.answers.saturating_add(u64::try_from(answers.len()).unwrap_or(u64::MAX));
        answers
    }

    fn counts(&self) -> Counts {
        self.counts
    }
}

/// `--network record FILE`: another network, with every request and its
/// answers written down as a file a replay reads.
pub struct Recorder {
    inner: Box<dyn Network>,
    /// Each request's number, the tick it was made on, and its entry.
    made: Vec<(u32, u64, Entry)>,
}

impl Recorder {
    /// Records `inner`.
    #[must_use]
    pub fn new(inner: Box<dyn Network>) -> Self {
        Self { inner, made: Vec::new() }
    }
}

impl Network for Recorder {
    fn request(&mut self, tick: u64, message: ToWebClient) {
        if let ToWebClient::Start(start) = &message
            && self.made.len() < MAX_NETWORK_ENTRIES
        {
            self.made.push((start.id, tick, Entry { kind: start.kind, method: start.method.clone(), url: start.url.clone(), replies: Vec::new() }));
        }
        self.inner.request(tick, message);
    }

    fn answers(&mut self, tick: u64) -> Vec<FromWebClient> {
        let answers = self.inner.answers(tick);
        for answer in &answers {
            let Some(id) = id_of(answer) else { continue };
            let Some((_, asked, entry)) = self.made.iter_mut().rev().find(|(made, ..)| *made == id) else { continue };
            // Given in the step of `tick`, so it had arrived by the tick
            // before: a replay gives it in the same step.
            let after = tick.saturating_sub(1).saturating_sub(*asked);
            if !matches!(answer, FromWebClient::Sent { .. }) && entry.replies.len() < MAX_NETWORK_STEPS {
                entry.replies.push(Reply { after, answer: numbered(answer, 0) });
            }
        }
        answers
    }

    fn counts(&self) -> Counts {
        self.inner.counts()
    }

    fn deterministic(&self) -> bool {
        self.inner.deterministic()
    }

    fn recording(&self) -> Option<Vec<u8>> {
        let entries: Vec<Entry> = self.made.iter().map(|(_, _, entry)| entry.clone()).collect();
        Some(write(&entries))
    }
}

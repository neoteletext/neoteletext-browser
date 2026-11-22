//! Messages between a channel's scripts and its web client (DEC-7260 item
//! 6, DEC-7262, DEC-7287 item 13; the script API's specification section
//! 11.3): a script's request crosses as flat, bounded values, and its
//! answer returns as events. This is the one definition both sides use.
//!
//! - **Three processes, two forms.** A script host writes a request in the
//!   *host form* ([`write_request`]): the message alone. The trusted core
//!   relays it by its kind and length, never reading its payload
//!   ([`relay_to_client`]), into the *client form* the web client reads
//!   ([`read_to_client`]): the tab's number before each message, as on a
//!   fetcher's pipe (REQ-6980), and the partition's token at the front of
//!   a [`Start`]. Answers go the other way: the web client writes the
//!   client form ([`write_from_client`]), the core relays
//!   ([`relay_from_client`]) and the script host reads the host form
//!   ([`read_answer`]).
//! - **One process, one partition** (DEC-7268, DEC-7264): a web client
//!   serves the one channel identity in the one jar of windows it was
//!   started for. The trusted core gives it a [`PartitionToken`] at its
//!   start and adds that token to every `Start` it relays; a `Start` with
//!   another token is refused by [`read_to_client`], and the web client
//!   ends. A script host never sees a token and cannot name a partition.
//! - **What a script host may say** is held by kind: it cannot write a
//!   tab's closing, a kept jar or a jar's forgetting, and it is never
//!   given the web client's `Confined` or `Jar`.
//!
//! A request's body and an answer's body cross in pieces of at most
//! [`MAX_CHUNK_BYTES`]. The answer's body is sent only as far as the
//! receiver has made room for it ([`FETCH_WINDOW_BYTES`], then
//! [`ToWebClient::More`]), so that a server that sends without end holds
//! no more memory than that.

use std::io::{Read, Write};

use crate::coded::coded;
use crate::fetcher::{PROCESS_TAB, cut_short, read_tab};
use crate::framing::HEADER_BYTES;
use crate::script::ContextId;
use crate::{Fields, MAX_PAYLOAD, ProtocolError, push_bytes, read_frame, write_frame};

// The web client's kinds: numbers no other pipe used on 22 November 2026,
// and none the script link uses (`crate::script`), since a script host
// writes both on its one pipe and the trusted core tells them apart by
// kind alone.
const KIND_FORGET_JAR: u8 = 229;
const KIND_START: u8 = 234;
const KIND_REQUEST_BODY: u8 = 235;
const KIND_REQUEST_BODY_END: u8 = 236;
const KIND_ABORT: u8 = 237;
const KIND_MORE: u8 = 238;
const KIND_TAB_CLOSES: u8 = 239;
const KIND_JAR_KEPT: u8 = 240;
const KIND_SOCKET_SEND: u8 = 241;
const KIND_SOCKET_CLOSE: u8 = 242;
const KIND_CONFINED: u8 = 243;
const KIND_HEAD: u8 = 244;
const KIND_BODY: u8 = 245;
const KIND_END: u8 = 246;
const KIND_FAILED: u8 = 247;
const KIND_OPENED: u8 = 248;
const KIND_EVENT: u8 = 249;
const KIND_RETRYING: u8 = 250;
const KIND_SOCKET_MESSAGE: u8 = 251;
const KIND_SOCKET_CLOSED: u8 = 252;
const KIND_JAR: u8 = 253;
const KIND_CONTEXT_CLOSES: u8 = 254;
const KIND_SENT: u8 = 255;

/// Every kind of this module, for the test that no two pipes that share a
/// process confuse a kind.
#[cfg(test)]
pub(crate) const KINDS: &[u8] = &[
    KIND_FORGET_JAR,
    KIND_START,
    KIND_REQUEST_BODY,
    KIND_REQUEST_BODY_END,
    KIND_ABORT,
    KIND_MORE,
    KIND_TAB_CLOSES,
    KIND_JAR_KEPT,
    KIND_SOCKET_SEND,
    KIND_SOCKET_CLOSE,
    KIND_CONFINED,
    KIND_HEAD,
    KIND_BODY,
    KIND_END,
    KIND_FAILED,
    KIND_OPENED,
    KIND_EVENT,
    KIND_RETRYING,
    KIND_SOCKET_MESSAGE,
    KIND_SOCKET_CLOSED,
    KIND_JAR,
    KIND_CONTEXT_CLOSES,
    KIND_SENT,
];

/// The longest address, in bytes: the engine's `MAX_URL_LENGTH`, which the
/// fetcher asserts this equals.
pub const MAX_URL_BYTES: usize = 8000;
/// The longest method: the longest the HTTP method registry holds is
/// `UPDATEREDIRECTREF`, 17 bytes; twice that, rounded up.
pub const MAX_METHOD_BYTES: usize = 40;
/// The most headers a request carries.
pub const MAX_REQUEST_HEADERS: usize = 64;
/// The most headers an answer's head carries.
pub const MAX_RESPONSE_HEADERS: usize = 128;
/// The longest header name, in bytes.
pub const MAX_HEADER_NAME_BYTES: usize = 128;
/// The longest header value, in bytes: what common servers accept for one
/// field line.
pub const MAX_HEADER_VALUE_BYTES: usize = 8 * 1024;
/// The most bytes of names and values together in one head, a request's or
/// an answer's.
pub const MAX_HEADER_BYTES: usize = 64 * 1024;
/// The longest status text of an answer, in bytes (RFC 9112 section 4: a
/// reason phrase, which HTTP/2 does not have).
pub const MAX_STATUS_TEXT_BYTES: usize = 128;
/// The longest integrity metadata, in bytes: three hashes of the longest
/// kind, SHA-512's 88 bytes of base64 and its `sha512-` prefix, with
/// options.
pub const MAX_INTEGRITY_BYTES: usize = 512;
/// The most bytes of a body, a WebSocket message or a jar in one message.
pub const MAX_CHUNK_BYTES: usize = 64 * 1024;
/// The largest request body, in bytes. The web client holds a request's
/// body whole before it sends it, so that a redirect can send it again.
pub const MAX_REQUEST_BODY_BYTES: usize = 16 * 1024 * 1024;
/// The bytes of an answer's body a request starts with room for; the
/// receiver makes more with [`ToWebClient::More`].
pub const FETCH_WINDOW_BYTES: u32 = 256 * 1024;
/// The most room a request may have at once, in bytes.
pub const MAX_NETWORK_BUFFER_BYTES: u32 = 4 * 1024 * 1024;
/// The longest event type of an event stream, in bytes.
pub const MAX_EVENT_TYPE_BYTES: usize = 256;
/// The longest last event ID of an event stream, in bytes.
pub const MAX_EVENT_ID_BYTES: usize = 1024;
/// The longest data of one event of an event stream, in bytes; a longer
/// event ends the stream with [`Reason::TooLarge`].
pub const MAX_EVENT_DATA_BYTES: usize = 128 * 1024;
/// The longest reason of a WebSocket's close, in bytes (RFC 6455 section
/// 5.5: a control frame's payload of 125 bytes less the two of the code).
pub const MAX_CLOSE_REASON_BYTES: usize = 123;
/// The most bytes of one part of a cookie jar written out for the trusted
/// core to keep (DEC-7268 item 3): room for a channel's whole jar at the
/// cookie standard's limits, which the fetcher asserts.
pub const MAX_JAR_BYTES: usize = 4 * 1024 * 1024;
/// The most web client processes alive at once, browser-wide (DEC-7268
/// item 4): one for each partition with a page open, the least recently
/// used ending first. The trusted core holds to it; a process for each
/// of the most tabs a window shows.
pub const MAX_WEB_CLIENTS: usize = crate::MAX_TABS;
/// The longest subprotocol of a WebSocket, in bytes.
pub const MAX_SUBPROTOCOL_BYTES: usize = 128;
/// The most subprotocols a WebSocket's opening offers.
pub const MAX_WS_PROTOCOLS: usize = 8;
/// The largest whole WebSocket message, in bytes, which crosses in pieces
/// of at most [`MAX_CHUNK_BYTES`]. Each side counts the pieces of a
/// message against it; no one message here can.
pub const MAX_WS_MESSAGE_BYTES: usize = 1024 * 1024;
/// The bytes of a partition's token.
pub const PARTITION_TOKEN_BYTES: usize = 16;

// The rates and counts one (tab, channel)'s scripts are held to (the
// script API's specification, sections 10.5 and 10.6; its departures D8
// and D22). They are defined once, here, for both sides: the script host
// holds them first, at the call, so that a script is told at once and a
// request waits its turn; the web client holds them again and its
// refusal is the authority, since a script host may be subverted. The
// script host counts by presentation ticks and the web client by the
// clock, so what the script host lets through the web client lets
// through as well, but for the two clocks' drift.

/// The fetches of a (tab, channel) under way at once.
pub const MAX_REQUESTS_IN_FLIGHT: usize = 8;
/// The fetches that wait at the script host for a place or for the rate;
/// one more is refused.
pub const MAX_REQUESTS_QUEUED: usize = 32;
/// The requests (fetches, and event streams' openings) started in one
/// step, aborted or not: the script host's count, which has steps.
pub const MAX_REQUESTS_PER_STEP: u32 = 8;
/// The requests started in any second, aborted or not. The web client
/// holds it as a second's worth saved up and a second's worth each
/// second, which admits every run of starts that keeps to this in each
/// second.
pub const MAX_REQUESTS_PER_SECOND: u32 = 32;
/// The event streams of a (tab, channel) open at once.
pub const MAX_EVENT_STREAMS: usize = 4;
/// The event streams opened in one step, closed or not.
pub const MAX_EVENT_STREAM_OPENS_PER_STEP: u32 = 2;
/// The sockets of a (tab, channel) open at once.
pub const MAX_SOCKETS: usize = 4;
/// The sockets opened in one step, closed or not.
pub const MAX_SOCKET_OPENS_PER_STEP: u32 = 2;
/// The sockets opened in any minute.
pub const MAX_SOCKET_OPENS_PER_MINUTE: u32 = 16;
const _: () = assert!(MAX_REQUESTS_PER_STEP <= MAX_REQUESTS_PER_SECOND && MAX_REQUESTS_IN_FLIGHT <= MAX_REQUESTS_QUEUED);

/// The longest payload of a `Start` as a script host writes it: the
/// trusted core puts the token before it, and the whole is one payload.
const MAX_HOST_START_BYTES: usize = MAX_PAYLOAD - PARTITION_TOKEN_BYTES;
/// The bytes of a length before a text or byte field.
const LENGTH_BYTES: usize = size_of::<u32>();

// Every message fits one payload.
const _: () = assert!(MAX_CHUNK_BYTES + 8 <= MAX_PAYLOAD);
const _: () = assert!(
    PARTITION_TOKEN_BYTES
        + 3 * MAX_URL_BYTES
        + MAX_HEADER_BYTES
        + 2 * LENGTH_BYTES * MAX_REQUEST_HEADERS
        + MAX_INTEGRITY_BYTES
        + MAX_METHOD_BYTES
        + MAX_WS_PROTOCOLS * (LENGTH_BYTES + MAX_SUBPROTOCOL_BYTES)
        + MAX_EVENT_ID_BYTES
        + 64
        <= MAX_PAYLOAD
);
const _: () = assert!(MAX_URL_BYTES + MAX_HEADER_BYTES + 2 * LENGTH_BYTES * MAX_RESPONSE_HEADERS + MAX_STATUS_TEXT_BYTES + 64 <= MAX_PAYLOAD);
const _: () = assert!(MAX_EVENT_TYPE_BYTES + MAX_EVENT_ID_BYTES + MAX_EVENT_DATA_BYTES + 16 <= MAX_PAYLOAD);

/// The token of a web client's partition: the trusted core's own value,
/// made for one process and told to it once at its start. It is no
/// secret kept from the web client; it is how a `Start` relayed to the
/// wrong process is known and refused.
#[derive(Clone, Copy)]
pub struct PartitionToken([u8; PARTITION_TOKEN_BYTES]);

impl PartitionToken {
    /// The token of `bytes`.
    #[must_use]
    pub const fn new(bytes: [u8; PARTITION_TOKEN_BYTES]) -> Self {
        Self(bytes)
    }

    /// The token's bytes, for the trusted core to hand to the process it
    /// starts.
    #[must_use]
    pub const fn bytes(&self) -> &[u8; PARTITION_TOKEN_BYTES] {
        &self.0
    }

    /// Whether `other` is this token, compared in the same time whatever
    /// the bytes are.
    fn is(&self, other: &[u8; PARTITION_TOKEN_BYTES]) -> bool {
        self.0.iter().zip(other).fold(0_u8, |difference, (own, given)| difference | (own ^ given)) == 0
    }
}

impl std::fmt::Debug for PartitionToken {
    /// A token is never written to a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PartitionToken(..)")
    }
}

coded! {
    /// What a request is for.
    RequestKind, "request kind" {
        /// A script's `fetch` (the Fetch standard).
        Fetch = 0,
        /// A script's `EventSource` (the HTML standard, server-sent events).
        EventSource = 1,
        /// A script's `WebSocket`.
        WebSocket = 2,
    }
}

coded! {
    /// The Fetch standard's request mode. `no-cors` is refused by the
    /// script API (its departure D5) and has no code here.
    Mode, "request mode" {
        /// Only the client's own origin.
        SameOrigin = 0,
        /// Another origin if it allows it (CORS).
        Cors = 1,
    }
}

coded! {
    /// The Fetch standard's credentials mode.
    Credentials, "credentials mode" {
        /// No cookies sent or kept.
        Omit = 0,
        /// Cookies for the client's own origin.
        SameOrigin = 1,
        /// Cookies for any origin.
        Include = 2,
    }
}

coded! {
    /// The Fetch standard's cache mode.
    CacheMode, "cache mode" {
        /// The HTTP cache's ordinary rules.
        Default = 0,
        /// Neither read nor written.
        NoStore = 1,
        /// Not read; written.
        Reload = 2,
        /// Revalidated before use.
        NoCache = 3,
        /// Used even when stale.
        ForceCache = 4,
        /// Used even when stale, and the network never asked.
        OnlyIfCached = 5,
    }
}

coded! {
    /// The Fetch standard's redirect mode.
    RedirectMode, "redirect mode" {
        /// Followed.
        Follow = 0,
        /// A redirect is a network error.
        Error = 1,
        /// The redirect itself is the answer, unreadable.
        Manual = 2,
    }
}

coded! {
    /// A referrer policy (the Referrer Policy standard, section 3).
    ReferrerPolicy, "referrer policy" {
        /// None stated: the default, `strict-origin-when-cross-origin`.
        Unset = 0,
        /// `no-referrer`.
        NoReferrer = 1,
        /// `no-referrer-when-downgrade`.
        NoReferrerWhenDowngrade = 2,
        /// `same-origin`.
        SameOrigin = 3,
        /// `origin`.
        Origin = 4,
        /// `strict-origin`.
        StrictOrigin = 5,
        /// `origin-when-cross-origin`.
        OriginWhenCrossOrigin = 6,
        /// `strict-origin-when-cross-origin`.
        StrictOriginWhenCrossOrigin = 7,
        /// `unsafe-url`.
        UnsafeUrl = 8,
    }
}

coded! {
    /// The Fetch standard's response type. An opaque answer, `no-cors`'s,
    /// has no code here (departure D5).
    ResponseType, "response type" {
        /// Same origin: every header but the cookie ones.
        Basic = 0,
        /// Another origin that allowed it: the headers it exposes.
        Cors = 1,
        /// A redirect not followed, unreadable.
        OpaqueRedirect = 3,
    }
}

coded! {
    /// The error a script is given, as the standards name it.
    ErrorKind, "error kind" {
        /// The Fetch standard's network error: a `TypeError`.
        Network = 0,
        /// The request was aborted: an `AbortError`.
        Abort = 1,
        /// A time limit passed: a `TimeoutError`.
        Timeout = 2,
    }
}

coded! {
    /// Why a request failed, by kind alone, for the developer log: never
    /// text from outside, and never given to the script.
    Reason, "failure reason" {
        /// The script aborted it, or its tab or context closed.
        Aborted = 0,
        /// The address is no address, or its scheme is not the request's.
        Address = 1,
        /// The port is one the Fetch standard blocks.
        BadPort = 2,
        /// The method is one the Fetch standard forbids, or no method.
        Method = 3,
        /// A channel from the internet that is not a secure context asked
        /// this computer or the local network, or a local name was asked
        /// through the proxy: refused, and never asked about
        /// (neoteletext-common's DEC-7315, the browser's TODO-7316 item 7).
        LocalRefused = 4,
        /// A page loaded securely asked for an address that is not.
        MixedContent = 5,
        /// A redirect led from `https` to `http`.
        Downgrade = 6,
        /// Another origin was asked in the mode `same-origin`.
        SameOriginMode = 7,
        /// The other origin's answer did not allow the page (CORS).
        Cors = 8,
        /// The other origin's answer to the preflight did not allow the
        /// request.
        Preflight = 9,
        /// A redirect in the redirect mode `error`, or one the mode or the
        /// standard does not allow.
        Redirect = 10,
        /// More than the standard's 20 redirects.
        RedirectLimit = 11,
        /// The body did not match the integrity metadata.
        Integrity = 12,
        /// The cache mode `only-if-cached` and nothing cached.
        NotCached = 13,
        /// The name did not resolve or no connection could be made.
        Connection = 14,
        /// The server's certificate was refused, or TLS failed.
        Certificate = 15,
        /// The proxy refused or failed.
        Proxy = 16,
        /// A time limit passed.
        Timeout = 17,
        /// The answer broke a rule of HTTP or of WebSocket framing.
        Protocol = 18,
        /// A limit on size or count was passed.
        TooLarge = 19,
        /// The body's content coding was unknown or its bytes malformed.
        Coding = 20,
        /// Too many requests under way, or started too fast.
        Busy = 21,
        /// An event stream's answer was not an event stream.
        NotEventStream = 22,
        /// The kind of request is not built.
        Unsupported = 23,
        /// The request itself was malformed.
        Request = 24,
        /// A channel from the internet asked an address on the local
        /// network: it needs the viewer's permission to reach devices
        /// there (DEC-7315). Until the browser can ask for it, the request
        /// is refused; this reason is what the trusted core turns into the
        /// asking.
        LocalPermission = 25,
        /// A channel from the internet asked this computer itself: it
        /// needs the viewer's permission to reach this device's own
        /// services, a permission apart from the local network's
        /// (DEC-7315). Refused until the browser can ask, as
        /// [`Reason::LocalPermission`] is.
        DevicePermission = 26,
    }
}

/// Where an address is (the Local Network Access draft's address spaces,
/// which neoteletext-common's DEC-7315 follows). A web client is told at
/// its start the space its channel was loaded from: only a channel of
/// [`AddressSpace::Public`] needs the viewer's permission for this
/// computer or the local network. It is the trusted core's word and no
/// message carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddressSpace {
    /// The internet.
    Public,
    /// The local network: private, link-local and shared addresses.
    Local,
    /// This computer itself.
    Loopback,
}

impl AddressSpace {
    /// Every space.
    pub const ALL: &'static [Self] = &[Self::Public, Self::Local, Self::Loopback];

    /// The space's name, as a web client's argument says it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Local => "local",
            Self::Loopback => "loopback",
        }
    }

    /// The space `name` names, if it names one.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|space| space.name() == name)
    }
}

/// The address space of a page opened from disk. The behaviour DEC-7315
/// follows makes it this computer's own, so its script reaches this
/// computer and the local network unasked; whether NeoTeletext does the
/// same is with the owner and not yet decided, and this one value is
/// where either answer is set ([`AddressSpace::Public`] makes such a
/// page ask).
pub const FILE_PAGE_SPACE: AddressSpace = AddressSpace::Loopback;

/// The referrer a request starts with (the Fetch standard's request
/// referrer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Referrer {
    /// `no-referrer`.
    None,
    /// The client: its address.
    Client,
    /// An address the script gave, of the client's own origin.
    Url(String),
}

const REFERRER_NONE: u8 = 0;
const REFERRER_CLIENT: u8 = 1;
const REFERRER_URL: u8 = 2;

/// Where a request's body comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodySource {
    /// No body.
    None,
    /// Bytes the script holds, this many, at most
    /// [`MAX_REQUEST_BODY_BYTES`], which a redirect may send again;
    /// [`ToWebClient::RequestBody`] messages of that many bytes together
    /// and a [`ToWebClient::RequestBodyEnd`] follow.
    Bytes(u32),
    /// A stream, which cannot be sent twice (the Fetch standard's body
    /// with a null source); the same messages follow. Version 1's script
    /// host never sends it (departure D7).
    Stream,
}

const BODY_NONE: u8 = 0;
const BODY_BYTES: u8 = 1;
const BODY_STREAM: u8 = 2;

/// A header: its name, and its value's bytes.
pub type Header = (String, Vec<u8>);

/// A request's start, as a script host writes it. Every field is the
/// script host's word and hostile input to the web client, which holds
/// each to the channel's policy before it acts. What the web client must
/// know and a script must not say (its tab, its partition) is not here:
/// the trusted core adds the tab before the message and the token before
/// these fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start {
    /// The request's number, the script host's choice, unique for its
    /// container's life, which every later message of the request
    /// carries. A web client knows a request by its tab and this.
    pub id: u32,
    /// The context asking, so that a page's requests end with it.
    pub context: ContextId,
    /// What it is for.
    pub kind: RequestKind,
    /// The method: an RFC 9110 token as the script side normalized it,
    /// never `CONNECT`, `TRACE` or `TRACK`.
    pub method: String,
    /// The address, absolute, as the host's parser serialised it: `http:`
    /// or `https:`, or for a WebSocket `ws:` or `wss:`.
    pub url: String,
    /// The address of the page asking, from which its referrer comes;
    /// empty for none. The web client trusts it only as far as its
    /// partition's channel holds it.
    pub client: String,
    /// The headers the script set.
    pub headers: Vec<Header>,
    /// The request mode.
    pub mode: Mode,
    /// The credentials mode.
    pub credentials: Credentials,
    /// The cache mode.
    pub cache: CacheMode,
    /// The redirect mode.
    pub redirect: RedirectMode,
    /// The referrer.
    pub referrer: Referrer,
    /// The referrer policy.
    pub referrer_policy: ReferrerPolicy,
    /// The integrity metadata, empty for none.
    pub integrity: String,
    /// The Fetch standard's keepalive flag.
    pub keepalive: bool,
    /// Where the body comes from.
    pub body: BodySource,
    /// A WebSocket's subprotocols, at most [`MAX_WS_PROTOCOLS`] distinct
    /// tokens; none for any other kind.
    pub protocols: Vec<String>,
    /// An event stream's last event ID, for its `Last-Event-ID` header;
    /// empty for none, and for any other kind.
    pub last_event_id: String,
}

/// An answer's head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    /// The request's number.
    pub id: u32,
    /// The response type; what follows is already filtered for it.
    pub response_type: ResponseType,
    /// The status, 0 for an unreadable answer.
    pub status: u16,
    /// The status text.
    pub status_text: String,
    /// The address that answered, after redirects, without its fragment;
    /// empty for an unreadable answer.
    pub url: String,
    /// Whether a redirect was followed.
    pub redirected: bool,
    /// The headers the script may read: for a `cors` answer only the
    /// CORS-safelisted names and those the server exposed, and never
    /// `Set-Cookie`.
    pub headers: Vec<Header>,
}

/// A piece of a cookie jar written out (DEC-7268 item 3). The web client
/// writes no file: after a change it sends its jar whole, in pieces, for
/// the trusted core to seal and keep for its partition, and is given it
/// back when a process for the partition next starts. The bytes are the
/// web client's own writing and the trusted core reads none of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JarPiece {
    /// Whether these are the cookies that end with the session, which the
    /// trusted core keeps in memory and never writes; else those that
    /// outlive it.
    pub session: bool,
    /// Whether this is the part's last piece. The pieces of one part are
    /// at most [`MAX_JAR_BYTES`] together.
    pub last: bool,
    /// At most [`MAX_CHUNK_BYTES`] bytes; none only in a last piece.
    pub bytes: Vec<u8>,
}

impl JarPiece {
    fn check(&self) -> Result<(), ProtocolError> {
        if self.bytes.len() > MAX_CHUNK_BYTES || (self.bytes.is_empty() && !self.last) { Err(ProtocolError::BadValue("jar piece")) } else { Ok(()) }
    }

    fn write(&self) -> Result<Vec<u8>, ProtocolError> {
        self.check()?;
        let mut payload = vec![u8::from(self.session), u8::from(self.last)];
        payload.extend_from_slice(&self.bytes);
        Ok(payload)
    }

    fn read(fields: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let piece = Self { session: fields.flag("session")?, last: fields.flag("last")?, bytes: fields.rest().to_vec() };
        piece.check()?;
        Ok(piece)
    }
}

/// To the web client. A script host writes the requests' messages; the
/// trusted core alone writes [`ToWebClient::TabCloses`],
/// [`ToWebClient::JarKept`] and [`ToWebClient::ForgetJar`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToWebClient {
    /// A request starts.
    Start(Box<Start>),
    /// The next bytes of a request's body.
    RequestBody {
        /// The request's number.
        id: u32,
        /// At most [`MAX_CHUNK_BYTES`] bytes, and not none.
        bytes: Vec<u8>,
    },
    /// A request's body is complete.
    RequestBodyEnd(u32),
    /// A request is aborted; a [`FromWebClient::Failed`] answers, unless
    /// the request had already ended.
    Abort(u32),
    /// The script consumed this many bytes of a request's answer: room
    /// for as many more.
    More {
        /// The request's number.
        id: u32,
        /// The bytes, not none; a request never has more than
        /// [`MAX_NETWORK_BUFFER_BYTES`] of room at once.
        bytes: u32,
    },
    /// A piece of a message to send on a WebSocket.
    SocketSend {
        /// The request's number.
        id: u32,
        /// Whether binary, else text.
        binary: bool,
        /// Whether this is the message's last piece.
        last: bool,
        /// At most [`MAX_CHUNK_BYTES`] bytes; none only in a last piece.
        bytes: Vec<u8>,
    },
    /// A WebSocket's close, from the script.
    SocketClose {
        /// The request's number.
        id: u32,
        /// The close code.
        code: u16,
        /// The reason, at most [`MAX_CLOSE_REASON_BYTES`].
        reason: String,
    },
    /// A context of the tab ends while the tab stays: each of its
    /// requests is aborted, unanswered, but those with keepalive.
    ContextCloses(ContextId),
    /// The tab closes: each of its requests is aborted, unanswered, but
    /// those with keepalive. The trusted core's alone.
    TabCloses,
    /// A piece of the partition's cookie jar as the trusted core kept it,
    /// given before the first request. The trusted core's alone.
    JarKept(JarPiece),
    /// The process forgets its one jar, and every kept state with it. The
    /// trusted core's alone.
    ForgetJar,
}

/// From the web client, which is untrusted. A script host is given the
/// answers' messages; [`FromWebClient::Confined`] and
/// [`FromWebClient::Jar`] are for the trusted core alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromWebClient {
    /// It is confined and running, said first as every helper says it.
    Confined,
    /// An answer's head.
    Head(Box<Head>),
    /// The next bytes of an answer's body, within the room given.
    Body {
        /// The request's number.
        id: u32,
        /// At most [`MAX_CHUNK_BYTES`] bytes, and not none.
        bytes: Vec<u8>,
    },
    /// An answer's body is complete; nothing follows for the request.
    End(u32),
    /// A request failed; nothing follows for it.
    Failed {
        /// The request's number.
        id: u32,
        /// The error the script is given.
        error: ErrorKind,
        /// Why, for the developer log; the script host never gives it to
        /// the script.
        reason: Reason,
    },
    /// An event stream is open, or a WebSocket.
    Opened {
        /// The request's number.
        id: u32,
        /// For a WebSocket, the subprotocol the server chose; empty for
        /// none, and for an event stream.
        protocol: String,
    },
    /// An event of an event stream.
    Event {
        /// The request's number.
        id: u32,
        /// The event's type, `message` when the stream names none.
        event: String,
        /// The data.
        data: String,
        /// The last event ID.
        last_id: String,
    },
    /// An event stream's connection was lost and will be made again: the
    /// script's `error` event with the stream still connecting.
    Retrying(u32),
    /// A piece of a message received on a WebSocket.
    SocketMessage {
        /// The request's number.
        id: u32,
        /// Whether binary, else text.
        binary: bool,
        /// Whether this is the message's last piece.
        last: bool,
        /// At most [`MAX_CHUNK_BYTES`] bytes; none only in a last piece.
        bytes: Vec<u8>,
    },
    /// A WebSocket closed.
    SocketClosed {
        /// The request's number.
        id: u32,
        /// The close code.
        code: u16,
        /// The reason.
        reason: String,
        /// Whether the closing handshake completed.
        clean: bool,
    },
    /// Bytes of a request's body or of a WebSocket's messages were
    /// written to the network: for `bufferedAmount` and the flow of an
    /// upload.
    Sent {
        /// The request's number.
        id: u32,
        /// How many, not none and at most [`MAX_REQUEST_BODY_BYTES`].
        bytes: u32,
    },
    /// A piece of the partition's cookie jar, written out after a change
    /// for the trusted core to keep. Never relayed to a script host.
    Jar(JarPiece),
}

/// Whether `byte` is a token character (RFC 9110 section 5.6.2), of which
/// a method, a header name and a subprotocol are made.
#[must_use]
pub const fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~')
}

/// Whether `value` is a header value as the Fetch standard defines one: no
/// NUL, carriage return or line feed, and no space or tab at either end.
#[must_use]
pub fn is_header_value(value: &[u8]) -> bool {
    let edge = |byte: Option<&u8>| matches!(byte, Some(b' ' | b'\t'));
    !value.iter().any(|&byte| matches!(byte, 0 | b'\r' | b'\n')) && !edge(value.first()) && !edge(value.last())
}

/// The methods the Fetch standard forbids, compared without case.
const FORBIDDEN_METHODS: [&str; 3] = ["CONNECT", "TRACE", "TRACK"];

fn check_token(text: &str, max: usize, what: &'static str) -> Result<(), ProtocolError> {
    if text.is_empty() || text.len() > max || !text.bytes().all(is_token_byte) { Err(ProtocolError::BadValue(what)) } else { Ok(()) }
}

/// An address on the pipe: visible ASCII, as the URL standard writes one.
fn check_url(text: &str, what: &'static str) -> Result<(), ProtocolError> {
    if text.len() > MAX_URL_BYTES || !text.bytes().all(|byte| byte.is_ascii_graphic()) { Err(ProtocolError::BadValue(what)) } else { Ok(()) }
}

fn check_headers(headers: &[Header], most: usize) -> Result<(), ProtocolError> {
    if headers.len() > most {
        return Err(ProtocolError::BadValue("too many headers"));
    }
    let mut total = 0_usize;
    for (name, value) in headers {
        check_token(name, MAX_HEADER_NAME_BYTES, "header name")?;
        if value.len() > MAX_HEADER_VALUE_BYTES || !is_header_value(value) {
            return Err(ProtocolError::BadValue("header value"));
        }
        total = total.saturating_add(name.len()).saturating_add(value.len());
    }
    if total > MAX_HEADER_BYTES { Err(ProtocolError::BadValue("headers too long")) } else { Ok(()) }
}

/// A piece of a body: not none.
fn check_piece(bytes: &[u8]) -> Result<(), ProtocolError> {
    if bytes.is_empty() || bytes.len() > MAX_CHUNK_BYTES { Err(ProtocolError::BadValue("body piece")) } else { Ok(()) }
}

/// A piece of a WebSocket message: none only when it is the last, since a
/// message may be empty.
fn check_socket_piece(bytes: &[u8], last: bool) -> Result<(), ProtocolError> {
    if bytes.len() > MAX_CHUNK_BYTES || (bytes.is_empty() && !last) { Err(ProtocolError::BadValue("socket piece")) } else { Ok(()) }
}

fn check_text(text: &str, max: usize, what: &'static str) -> Result<(), ProtocolError> {
    if text.len() > max { Err(ProtocolError::BadValue(what)) } else { Ok(()) }
}

/// A status text: visible ASCII, spaces and tabs (RFC 9112 section 4,
/// without its obsolete bytes).
fn check_status_text(text: &str) -> Result<(), ProtocolError> {
    if text.len() > MAX_STATUS_TEXT_BYTES || !text.bytes().all(|byte| byte.is_ascii_graphic() || matches!(byte, b' ' | b'\t')) {
        Err(ProtocolError::BadValue("status text"))
    } else {
        Ok(())
    }
}

fn check_room(bytes: u32) -> Result<(), ProtocolError> {
    if bytes == 0 || bytes > MAX_NETWORK_BUFFER_BYTES { Err(ProtocolError::BadValue("room")) } else { Ok(()) }
}

fn check_sent(bytes: u32) -> Result<(), ProtocolError> {
    if bytes == 0 || usize::try_from(bytes).is_ok_and(|bytes| bytes > MAX_REQUEST_BODY_BYTES) { Err(ProtocolError::BadValue("sent")) } else { Ok(()) }
}

/// A subprotocol the server chose: a token, or empty for none.
fn check_protocol(text: &str) -> Result<(), ProtocolError> {
    if text.is_empty() { Ok(()) } else { check_token(text, MAX_SUBPROTOCOL_BYTES, "subprotocol") }
}

/// A status a head may carry: 0 for an unreadable answer, else a final
/// HTTP status (RFC 9110 section 15).
fn check_status(status: u16, response_type: ResponseType) -> Result<(), ProtocolError> {
    let unreadable = matches!(response_type, ResponseType::OpaqueRedirect);
    if (unreadable && status == 0) || (!unreadable && (200..=599).contains(&status)) { Ok(()) } else { Err(ProtocolError::BadValue("status")) }
}

fn push_headers(payload: &mut Vec<u8>, headers: &[Header], most: usize) -> Result<(), ProtocolError> {
    check_headers(headers, most)?;
    crate::push_count(payload, headers.len(), most, "too many headers")?;
    for (name, value) in headers {
        push_bytes(payload, name.as_bytes())?;
        push_bytes(payload, value)?;
    }
    Ok(())
}

fn take_headers(fields: &mut Fields<'_>, most: usize) -> Result<Vec<Header>, ProtocolError> {
    let count = fields.count_u16(most, "too many headers")?;
    let mut headers = Vec::with_capacity(count);
    for _ in 0..count {
        let name = fields.string(MAX_HEADER_NAME_BYTES, "header name")?;
        let value = fields.bytes(MAX_HEADER_VALUE_BYTES, "header value")?.to_vec();
        headers.push((name, value));
    }
    check_headers(&headers, most)?;
    Ok(headers)
}

impl Start {
    /// Whether every field keeps to its bounds.
    ///
    /// # Errors
    /// [`ProtocolError::BadValue`] naming the first that does not.
    pub fn check(&self) -> Result<(), ProtocolError> {
        check_token(&self.method, MAX_METHOD_BYTES, "method")?;
        if FORBIDDEN_METHODS.iter().any(|forbidden| self.method.eq_ignore_ascii_case(forbidden)) {
            return Err(ProtocolError::BadValue("method"));
        }
        check_url(&self.url, "address")?;
        let socket = self.kind == RequestKind::WebSocket;
        let schemes: [&str; 2] = if socket { ["ws://", "wss://"] } else { ["http://", "https://"] };
        if !schemes.iter().any(|scheme| self.url.starts_with(scheme)) {
            return Err(ProtocolError::BadValue("address"));
        }
        check_url(&self.client, "client address")?;
        check_headers(&self.headers, MAX_REQUEST_HEADERS)?;
        if let Referrer::Url(url) = &self.referrer {
            check_url(url, "referrer")?;
        }
        if self.integrity.len() > MAX_INTEGRITY_BYTES || !self.integrity.bytes().all(|byte| byte.is_ascii_graphic() || byte == b' ') {
            return Err(ProtocolError::BadValue("integrity"));
        }
        if let BodySource::Bytes(length) = self.body
            && usize::try_from(length).is_ok_and(|length| length > MAX_REQUEST_BODY_BYTES)
        {
            return Err(ProtocolError::BadValue("body length"));
        }
        if self.protocols.len() > MAX_WS_PROTOCOLS || (!socket && !self.protocols.is_empty()) {
            return Err(ProtocolError::BadValue("subprotocols"));
        }
        for (place, protocol) in self.protocols.iter().enumerate() {
            check_token(protocol, MAX_SUBPROTOCOL_BYTES, "subprotocol")?;
            if self.protocols.iter().take(place).any(|earlier| earlier == protocol) {
                return Err(ProtocolError::BadValue("subprotocols"));
            }
        }
        // It becomes a header's value, so it is held to one.
        if self.last_event_id.len() > MAX_EVENT_ID_BYTES
            || (self.kind != RequestKind::EventSource && !self.last_event_id.is_empty())
            || self.last_event_id.bytes().any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
        {
            return Err(ProtocolError::BadValue("last event ID"));
        }
        Ok(())
    }

    fn write(&self, payload: &mut Vec<u8>) -> Result<(), ProtocolError> {
        self.check()?;
        payload.extend_from_slice(&self.id.to_be_bytes());
        payload.extend_from_slice(&[
            self.context.code(),
            self.kind.code(),
            self.mode.code(),
            self.credentials.code(),
            self.cache.code(),
            self.redirect.code(),
            self.referrer_policy.code(),
            u8::from(self.keepalive),
        ]);
        match self.body {
            BodySource::None => payload.push(BODY_NONE),
            BodySource::Bytes(length) => {
                payload.push(BODY_BYTES);
                payload.extend_from_slice(&length.to_be_bytes());
            }
            BodySource::Stream => payload.push(BODY_STREAM),
        }
        push_bytes(payload, self.method.as_bytes())?;
        push_bytes(payload, self.url.as_bytes())?;
        push_bytes(payload, self.client.as_bytes())?;
        match &self.referrer {
            Referrer::None => payload.push(REFERRER_NONE),
            Referrer::Client => payload.push(REFERRER_CLIENT),
            Referrer::Url(url) => {
                payload.push(REFERRER_URL);
                push_bytes(payload, url.as_bytes())?;
            }
        }
        push_bytes(payload, self.integrity.as_bytes())?;
        push_headers(payload, &self.headers, MAX_REQUEST_HEADERS)?;
        crate::push_count(payload, self.protocols.len(), MAX_WS_PROTOCOLS, "subprotocols")?;
        for protocol in &self.protocols {
            push_bytes(payload, protocol.as_bytes())?;
        }
        push_bytes(payload, self.last_event_id.as_bytes())
    }

    fn read(fields: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let id = fields.u32()?;
        let context = ContextId::of(fields.u8()?)?;
        let kind = RequestKind::of(fields.u8()?)?;
        let mode = Mode::of(fields.u8()?)?;
        let credentials = Credentials::of(fields.u8()?)?;
        let cache = CacheMode::of(fields.u8()?)?;
        let redirect = RedirectMode::of(fields.u8()?)?;
        let referrer_policy = ReferrerPolicy::of(fields.u8()?)?;
        let keepalive = fields.flag("keepalive")?;
        let body = match fields.u8()? {
            BODY_NONE => BodySource::None,
            BODY_BYTES => BodySource::Bytes(fields.u32()?),
            BODY_STREAM => BodySource::Stream,
            _ => return Err(ProtocolError::BadValue("body source")),
        };
        let method = fields.string(MAX_METHOD_BYTES, "method")?;
        let url = fields.string(MAX_URL_BYTES, "address")?;
        let client = fields.string(MAX_URL_BYTES, "client address")?;
        let referrer = match fields.u8()? {
            REFERRER_NONE => Referrer::None,
            REFERRER_CLIENT => Referrer::Client,
            REFERRER_URL => Referrer::Url(fields.string(MAX_URL_BYTES, "referrer")?),
            _ => return Err(ProtocolError::BadValue("referrer")),
        };
        let integrity = fields.string(MAX_INTEGRITY_BYTES, "integrity")?;
        let headers = take_headers(fields, MAX_REQUEST_HEADERS)?;
        let count = fields.count_u16(MAX_WS_PROTOCOLS, "subprotocols")?;
        let mut protocols = Vec::with_capacity(count);
        for _ in 0..count {
            protocols.push(fields.string(MAX_SUBPROTOCOL_BYTES, "subprotocol")?);
        }
        let last_event_id = fields.string(MAX_EVENT_ID_BYTES, "last event ID")?;
        let start = Self {
            id,
            context,
            kind,
            method,
            url,
            client,
            headers,
            mode,
            credentials,
            cache,
            redirect,
            referrer,
            referrer_policy,
            integrity,
            keepalive,
            body,
            protocols,
            last_event_id,
        };
        start.check()?;
        Ok(start)
    }
}

impl Head {
    /// Whether every field keeps to its bounds.
    ///
    /// # Errors
    /// [`ProtocolError::BadValue`] naming the first that does not.
    pub fn check(&self) -> Result<(), ProtocolError> {
        check_status(self.status, self.response_type)?;
        check_status_text(&self.status_text)?;
        check_url(&self.url, "address")?;
        check_headers(&self.headers, MAX_RESPONSE_HEADERS)
    }

    fn write(&self, payload: &mut Vec<u8>) -> Result<(), ProtocolError> {
        self.check()?;
        payload.extend_from_slice(&self.id.to_be_bytes());
        payload.push(self.response_type.code());
        payload.extend_from_slice(&self.status.to_be_bytes());
        payload.push(u8::from(self.redirected));
        push_bytes(payload, self.status_text.as_bytes())?;
        push_bytes(payload, self.url.as_bytes())?;
        push_headers(payload, &self.headers, MAX_RESPONSE_HEADERS)
    }

    fn read(fields: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let id = fields.u32()?;
        let response_type = ResponseType::of(fields.u8()?)?;
        let status = fields.u16()?;
        let redirected = fields.flag("redirected")?;
        let status_text = fields.string(MAX_STATUS_TEXT_BYTES, "status text")?;
        let url = fields.string(MAX_URL_BYTES, "address")?;
        let headers = take_headers(fields, MAX_RESPONSE_HEADERS)?;
        let head = Self { id, response_type, status, status_text, url, redirected, headers };
        head.check()?;
        Ok(head)
    }
}

fn with_id(id: u32) -> Vec<u8> {
    id.to_be_bytes().to_vec()
}

/// The rest of a payload as text of at most `max` bytes.
fn rest_text(fields: &mut Fields<'_>, max: usize, what: &'static str) -> Result<String, ProtocolError> {
    let bytes = fields.rest();
    if bytes.len() > max {
        return Err(ProtocolError::BadValue(what));
    }
    std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| ProtocolError::BadValue(what))
}

/// Whether a script host may write a message of `kind`, and the trusted
/// core relay it to the web client: the requests' kinds, and none of the
/// trusted core's own.
#[must_use]
pub const fn is_request_kind(kind: u8) -> bool {
    matches!(kind, KIND_START | KIND_REQUEST_BODY | KIND_REQUEST_BODY_END | KIND_ABORT | KIND_MORE | KIND_SOCKET_SEND | KIND_SOCKET_CLOSE | KIND_CONTEXT_CLOSES)
}

/// Whether the trusted core relays a web client's message of `kind` to a
/// script host: the answers' kinds, and neither `Confined` nor `Jar`.
#[must_use]
pub const fn is_answer_kind(kind: u8) -> bool {
    matches!(
        kind,
        KIND_HEAD | KIND_BODY | KIND_END | KIND_FAILED | KIND_OPENED | KIND_EVENT | KIND_RETRYING | KIND_SOCKET_MESSAGE | KIND_SOCKET_CLOSED | KIND_SENT
    )
}

impl ToWebClient {
    /// The message's kind and its payload, in the host form.
    fn payload(&self) -> Result<(u8, Vec<u8>), ProtocolError> {
        Ok(match self {
            Self::Start(start) => {
                let mut payload = Vec::new();
                start.write(&mut payload)?;
                if payload.len() > MAX_HOST_START_BYTES {
                    return Err(ProtocolError::TooLong(payload.len()));
                }
                (KIND_START, payload)
            }
            Self::RequestBody { id, bytes } => {
                check_piece(bytes)?;
                let mut payload = with_id(*id);
                payload.extend_from_slice(bytes);
                (KIND_REQUEST_BODY, payload)
            }
            Self::RequestBodyEnd(id) => (KIND_REQUEST_BODY_END, with_id(*id)),
            Self::Abort(id) => (KIND_ABORT, with_id(*id)),
            Self::More { id, bytes } => {
                check_room(*bytes)?;
                let mut payload = with_id(*id);
                payload.extend_from_slice(&bytes.to_be_bytes());
                (KIND_MORE, payload)
            }
            Self::SocketSend { id, binary, last, bytes } => {
                check_socket_piece(bytes, *last)?;
                let mut payload = with_id(*id);
                payload.extend_from_slice(&[u8::from(*binary), u8::from(*last)]);
                payload.extend_from_slice(bytes);
                (KIND_SOCKET_SEND, payload)
            }
            Self::SocketClose { id, code, reason } => {
                check_text(reason, MAX_CLOSE_REASON_BYTES, "close reason")?;
                let mut payload = with_id(*id);
                payload.extend_from_slice(&code.to_be_bytes());
                payload.extend_from_slice(reason.as_bytes());
                (KIND_SOCKET_CLOSE, payload)
            }
            Self::ContextCloses(context) => (KIND_CONTEXT_CLOSES, vec![context.code()]),
            Self::TabCloses => (KIND_TAB_CLOSES, Vec::new()),
            Self::JarKept(piece) => (KIND_JAR_KEPT, piece.write()?),
            Self::ForgetJar => (KIND_FORGET_JAR, Vec::new()),
        })
    }

    /// The message of `kind` whose host-form payload is `payload`.
    fn parse(kind: u8, payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut fields = Fields::new(payload);
        let message = match kind {
            KIND_START => Self::Start(Box::new(Start::read(&mut fields)?)),
            KIND_REQUEST_BODY => {
                let id = fields.u32()?;
                let bytes = fields.rest();
                check_piece(bytes)?;
                Self::RequestBody { id, bytes: bytes.to_vec() }
            }
            KIND_REQUEST_BODY_END => Self::RequestBodyEnd(fields.u32()?),
            KIND_ABORT => Self::Abort(fields.u32()?),
            KIND_MORE => {
                let id = fields.u32()?;
                let bytes = fields.u32()?;
                check_room(bytes)?;
                Self::More { id, bytes }
            }
            KIND_SOCKET_SEND => {
                let (id, binary, last) = (fields.u32()?, fields.flag("binary")?, fields.flag("last")?);
                let bytes = fields.rest();
                check_socket_piece(bytes, last)?;
                Self::SocketSend { id, binary, last, bytes: bytes.to_vec() }
            }
            KIND_SOCKET_CLOSE => {
                Self::SocketClose { id: fields.u32()?, code: fields.u16()?, reason: rest_text(&mut fields, MAX_CLOSE_REASON_BYTES, "close reason")? }
            }
            KIND_CONTEXT_CLOSES => Self::ContextCloses(ContextId::of(fields.u8()?)?),
            KIND_TAB_CLOSES => Self::TabCloses,
            KIND_JAR_KEPT => Self::JarKept(JarPiece::read(&mut fields)?),
            KIND_FORGET_JAR => Self::ForgetJar,
            other => return Err(ProtocolError::UnknownKind(other)),
        };
        fields.end()?;
        Ok(message)
    }

    /// Whether the message is for the process and no tab: the jar's.
    const fn is_process(&self) -> bool {
        matches!(self, Self::JarKept(_) | Self::ForgetJar)
    }
}

impl FromWebClient {
    /// The message's kind and its payload.
    fn payload(&self) -> Result<(u8, Vec<u8>), ProtocolError> {
        Ok(match self {
            Self::Confined => (KIND_CONFINED, Vec::new()),
            Self::Head(head) => {
                let mut payload = Vec::new();
                head.write(&mut payload)?;
                (KIND_HEAD, payload)
            }
            Self::Body { id, bytes } => {
                check_piece(bytes)?;
                let mut payload = with_id(*id);
                payload.extend_from_slice(bytes);
                (KIND_BODY, payload)
            }
            Self::End(id) => (KIND_END, with_id(*id)),
            Self::Failed { id, error, reason } => {
                let mut payload = with_id(*id);
                payload.extend_from_slice(&[error.code(), reason.code()]);
                (KIND_FAILED, payload)
            }
            Self::Opened { id, protocol } => {
                check_protocol(protocol)?;
                let mut payload = with_id(*id);
                payload.extend_from_slice(protocol.as_bytes());
                (KIND_OPENED, payload)
            }
            Self::Event { id, event, data, last_id } => {
                check_text(event, MAX_EVENT_TYPE_BYTES, "event type")?;
                check_text(data, MAX_EVENT_DATA_BYTES, "event data")?;
                check_text(last_id, MAX_EVENT_ID_BYTES, "event ID")?;
                let mut payload = with_id(*id);
                push_bytes(&mut payload, event.as_bytes())?;
                push_bytes(&mut payload, last_id.as_bytes())?;
                push_bytes(&mut payload, data.as_bytes())?;
                (KIND_EVENT, payload)
            }
            Self::Retrying(id) => (KIND_RETRYING, with_id(*id)),
            Self::SocketMessage { id, binary, last, bytes } => {
                check_socket_piece(bytes, *last)?;
                let mut payload = with_id(*id);
                payload.extend_from_slice(&[u8::from(*binary), u8::from(*last)]);
                payload.extend_from_slice(bytes);
                (KIND_SOCKET_MESSAGE, payload)
            }
            Self::SocketClosed { id, code, reason, clean } => {
                check_text(reason, MAX_CLOSE_REASON_BYTES, "close reason")?;
                let mut payload = with_id(*id);
                payload.extend_from_slice(&code.to_be_bytes());
                payload.push(u8::from(*clean));
                payload.extend_from_slice(reason.as_bytes());
                (KIND_SOCKET_CLOSED, payload)
            }
            Self::Sent { id, bytes } => {
                check_sent(*bytes)?;
                let mut payload = with_id(*id);
                payload.extend_from_slice(&bytes.to_be_bytes());
                (KIND_SENT, payload)
            }
            Self::Jar(piece) => (KIND_JAR, piece.write()?),
        })
    }

    /// The message of `kind` whose payload is `payload`.
    fn parse(kind: u8, payload: &[u8]) -> Result<Self, ProtocolError> {
        let mut fields = Fields::new(payload);
        let message = match kind {
            KIND_CONFINED => Self::Confined,
            KIND_HEAD => Self::Head(Box::new(Head::read(&mut fields)?)),
            KIND_BODY => {
                let id = fields.u32()?;
                let bytes = fields.rest();
                check_piece(bytes)?;
                Self::Body { id, bytes: bytes.to_vec() }
            }
            KIND_END => Self::End(fields.u32()?),
            KIND_FAILED => Self::Failed { id: fields.u32()?, error: ErrorKind::of(fields.u8()?)?, reason: Reason::of(fields.u8()?)? },
            KIND_OPENED => {
                let id = fields.u32()?;
                let protocol = rest_text(&mut fields, MAX_SUBPROTOCOL_BYTES, "subprotocol")?;
                check_protocol(&protocol)?;
                Self::Opened { id, protocol }
            }
            KIND_EVENT => {
                let id = fields.u32()?;
                let event = fields.string(MAX_EVENT_TYPE_BYTES, "event type")?;
                let last_id = fields.string(MAX_EVENT_ID_BYTES, "event ID")?;
                let data = fields.string(MAX_EVENT_DATA_BYTES, "event data")?;
                Self::Event { id, event, data, last_id }
            }
            KIND_RETRYING => Self::Retrying(fields.u32()?),
            KIND_SOCKET_MESSAGE => {
                let (id, binary, last) = (fields.u32()?, fields.flag("binary")?, fields.flag("last")?);
                let bytes = fields.rest();
                check_socket_piece(bytes, last)?;
                Self::SocketMessage { id, binary, last, bytes: bytes.to_vec() }
            }
            KIND_SOCKET_CLOSED => {
                let (id, code, clean) = (fields.u32()?, fields.u16()?, fields.flag("clean")?);
                Self::SocketClosed { id, code, reason: rest_text(&mut fields, MAX_CLOSE_REASON_BYTES, "close reason")?, clean }
            }
            KIND_SENT => {
                let (id, bytes) = (fields.u32()?, fields.u32()?);
                check_sent(bytes)?;
                Self::Sent { id, bytes }
            }
            KIND_JAR => Self::Jar(JarPiece::read(&mut fields)?),
            other => return Err(ProtocolError::UnknownKind(other)),
        };
        fields.end()?;
        Ok(message)
    }

    /// Whether the message is the process's and no tab's: for the trusted
    /// core alone.
    const fn is_process(&self) -> bool {
        matches!(self, Self::Confined | Self::Jar(_))
    }
}

/// Holds a message's tab to its kind: the process's own messages name
/// [`PROCESS_TAB`], and a tab's never do.
fn check_tab(tab: u32, process: bool) -> Result<(), ProtocolError> {
    if (tab == PROCESS_TAB) == process { Ok(()) } else { Err(ProtocolError::BadValue("tab")) }
}

/// Writes a request's message as a script host writes it: the host form.
///
/// # Errors
/// When writing fails, a field is out of its bounds, or the message is
/// one only the trusted core may write.
pub fn write_request(out: &mut dyn Write, message: &ToWebClient) -> Result<(), ProtocolError> {
    let (kind, payload) = message.payload()?;
    if !is_request_kind(kind) {
        return Err(ProtocolError::BadValue("not a script host's message"));
    }
    write_frame(out, kind, &payload)
}

/// Reads one message a script host wrote, or `None` at a clean end: the
/// host form, for whatever stands where the web client would without a
/// trusted core between (the standalone runner, a test).
///
/// # Errors
/// On input errors, a kind a script host may not write, and any payload
/// that is malformed or out of its bounds.
pub fn read_request(input: &mut dyn Read) -> Result<Option<ToWebClient>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    if !is_request_kind(kind) {
        return Err(ProtocolError::UnknownKind(kind));
    }
    ToWebClient::parse(kind, &payload).map(Some)
}

/// The trusted core's relay to a web client: moves every whole message at
/// the front of `written`, which a script host of the tab `tab` wrote, to
/// `out` in the client form, each after the tab's number and a `Start`
/// with `token` before its fields. Only a kind and a length are read. A
/// message not yet whole stays in `written` for the bytes that complete
/// it.
///
/// # Errors
/// [`ProtocolError::UnknownKind`] at a kind a script host may not write,
/// and [`ProtocolError::TooLong`] at a length no message has. The
/// messages before it are moved; it and what follows stay, and the caller
/// ends the script host.
pub fn relay_to_client(tab: u32, token: &PartitionToken, written: &mut Vec<u8>, out: &mut Vec<u8>) -> Result<(), ProtocolError> {
    let mut moved = 0_usize;
    let mut outcome = Ok(());
    while let Some(header) = written.get(moved..).and_then(|rest| rest.first_chunk::<HEADER_BYTES>()) {
        let [kind, length @ ..] = *header;
        let length = u32::from_be_bytes(length) as usize;
        if !is_request_kind(kind) {
            outcome = Err(ProtocolError::UnknownKind(kind));
            break;
        }
        let start = kind == KIND_START;
        if length > if start { MAX_HOST_START_BYTES } else { MAX_PAYLOAD } {
            outcome = Err(ProtocolError::TooLong(length));
            break;
        }
        let from = moved.saturating_add(HEADER_BYTES);
        let whole = from.saturating_add(length);
        let Some(payload) = written.get(from..whole) else { break };
        let relayed = if start { length.saturating_add(PARTITION_TOKEN_BYTES) } else { length };
        let Ok(relayed) = u32::try_from(relayed) else {
            outcome = Err(ProtocolError::TooLong(length));
            break;
        };
        out.extend_from_slice(&tab.to_be_bytes());
        out.push(kind);
        out.extend_from_slice(&relayed.to_be_bytes());
        if start {
            out.extend_from_slice(token.bytes());
        }
        out.extend_from_slice(payload);
        moved = whole;
    }
    written.drain(..moved);
    outcome
}

/// Writes `message` for the tab `tab` in the client form, as
/// [`relay_to_client`] relays a script host's: what the trusted core
/// writes of its own, with [`PROCESS_TAB`] for the jar's messages.
///
/// # Errors
/// When writing fails, a field is out of its bounds, or the tab is not
/// one the message may name.
pub fn write_to_client(out: &mut dyn Write, tab: u32, token: &PartitionToken, message: &ToWebClient) -> Result<(), ProtocolError> {
    check_tab(tab, message.is_process())?;
    let (kind, mut payload) = message.payload()?;
    if kind == KIND_START {
        payload.splice(0..0, token.bytes().iter().copied());
    }
    out.write_all(&tab.to_be_bytes())?;
    write_frame(out, kind, &payload)
}

/// Reads one message as the web client whose token is `own` reads it, with
/// its tab, or `None` at a clean end.
///
/// # Errors
/// On input errors, an unknown kind, any payload that is malformed or out
/// of its bounds, a tab the message may not name, and a `Start` whose
/// token is not `own`. Each ends the web client: it serves one partition,
/// and a start for any other is never served (DEC-7268).
pub fn read_to_client(input: &mut dyn Read, own: &PartitionToken) -> Result<Option<(u32, ToWebClient)>, ProtocolError> {
    let Some(tab) = read_tab(input)? else { return Ok(None) };
    let (kind, payload) = read_frame(input)?.ok_or_else(cut_short)?;
    let message = if kind == KIND_START {
        let mut fields = Fields::new(&payload);
        if !own.is(&fields.array()?) {
            return Err(ProtocolError::BadValue("partition token"));
        }
        ToWebClient::parse(kind, fields.rest())?
    } else {
        ToWebClient::parse(kind, &payload)?
    };
    check_tab(tab, message.is_process())?;
    Ok(Some((tab, message)))
}

/// Writes `message` as the web client writes it, for the tab `tab`: the
/// client form, with [`PROCESS_TAB`] for `Confined` and `Jar`.
///
/// # Errors
/// When writing fails, a field is out of its bounds, or the tab is not
/// one the message may name.
pub fn write_from_client(out: &mut dyn Write, tab: u32, message: &FromWebClient) -> Result<(), ProtocolError> {
    check_tab(tab, message.is_process())?;
    let (kind, payload) = message.payload()?;
    out.write_all(&tab.to_be_bytes())?;
    write_frame(out, kind, &payload)
}

/// Reads one message a web client wrote, whole, with its tab, or `None` at
/// a clean end.
///
/// # Errors
/// On input errors, a kind a web client may not send (every other pipe's
/// kind among them), any payload that is malformed or out of its bounds,
/// and a tab the message may not name.
pub fn read_from_client(input: &mut dyn Read) -> Result<Option<(u32, FromWebClient)>, ProtocolError> {
    let Some(tab) = read_tab(input)? else { return Ok(None) };
    let (kind, payload) = read_frame(input)?.ok_or_else(cut_short)?;
    let message = FromWebClient::parse(kind, &payload)?;
    check_tab(tab, message.is_process())?;
    Ok(Some((tab, message)))
}

/// What the trusted core takes from a web client's pipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromClientFrame {
    /// A message for the trusted core itself, read whole:
    /// [`FromWebClient::Confined`] or [`FromWebClient::Jar`].
    Core(FromWebClient),
    /// An answer for the script host of a tab, its payload unread.
    Answer {
        /// The tab.
        tab: u32,
        /// The message in the host form, to be written to that tab's
        /// script host as it stands.
        frame: Vec<u8>,
    },
}

/// The trusted core's relay from a web client: reads one message in the
/// client form, or `None` at a clean end. An answer is given back as the
/// bytes a script host reads, known by its kind and length alone; only
/// the process's own messages are parsed.
///
/// # Errors
/// On input errors, a kind a web client may not send, a tab the kind may
/// not name, and a malformed `Confined` or `Jar`.
pub fn relay_from_client(input: &mut dyn Read) -> Result<Option<FromClientFrame>, ProtocolError> {
    let Some(tab) = read_tab(input)? else { return Ok(None) };
    let (kind, payload) = read_frame(input)?.ok_or_else(cut_short)?;
    if is_answer_kind(kind) {
        check_tab(tab, false)?;
        let mut frame = Vec::with_capacity(HEADER_BYTES.saturating_add(payload.len()));
        write_frame(&mut frame, kind, &payload)?;
        return Ok(Some(FromClientFrame::Answer { tab, frame }));
    }
    if !matches!(kind, KIND_CONFINED | KIND_JAR) {
        return Err(ProtocolError::UnknownKind(kind));
    }
    check_tab(tab, true)?;
    FromWebClient::parse(kind, &payload).map(|message| Some(FromClientFrame::Core(message)))
}

/// Writes an answer's message in the host form, as a script host reads
/// it: for whatever stands where the web client would without a trusted
/// core between.
///
/// # Errors
/// When writing fails, a field is out of its bounds, or the message is
/// one a script host is never given.
pub fn write_answer(out: &mut dyn Write, message: &FromWebClient) -> Result<(), ProtocolError> {
    let (kind, payload) = message.payload()?;
    if !is_answer_kind(kind) {
        return Err(ProtocolError::BadValue("not a script host's message"));
    }
    write_frame(out, kind, &payload)
}

/// Reads one answer's message as a script host reads it, or `None` at a
/// clean end. Every field is hostile input to the script host too.
///
/// # Errors
/// On input errors, a kind a script host is never given, and any payload
/// that is malformed or out of its bounds.
pub fn read_answer(input: &mut dyn Read) -> Result<Option<FromWebClient>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    if !is_answer_kind(kind) {
        return Err(ProtocolError::UnknownKind(kind));
    }
    FromWebClient::parse(kind, &payload).map(Some)
}

#[cfg(test)]
mod tests;

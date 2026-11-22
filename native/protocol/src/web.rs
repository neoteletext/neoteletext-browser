//! Messages between the trusted core and the fetcher's general web client
//! (neoteletext-common's DEC-7260 item 6, adopted by DEC-7262): a script's
//! request crosses as flat, bounded values, and its answer returns as
//! events. A set of their own, on a pipe of their own: no message of
//! `protocol::fetcher` is read or written here, and none of these on a
//! channel's fetcher's.
//!
//! - **To the web client:** [`ToWebClient::Start`] (a request's number,
//!   kind, method, address, headers and the Fetch standard's options),
//!   its body in pieces and their end, an abort, more room for the answer's
//!   body, a tab's closing and a private window's jars forgotten.
//! - **From it:** that it is confined, an answer's head, its body in
//!   pieces, its end, a failure by kind, for an event stream (the HTML
//!   standard's `EventSource`) its opening, each event and each
//!   reconnection, and the cookie jar written out after a change.
//! - **One process, one partition** (the browser's DEC-7268): a web client
//!   serves the one channel identity in the one jar of windows it was
//!   started for, and holds no other's cookies or connections.
//! - **Reserved** for a WebSocket, in the same shape: a message each way
//!   and a close each way. The web client refuses a WebSocket's start with
//!   [`Reason::Unsupported`] until it is built.
//!
//! A request's body and an answer's body cross in pieces of at most
//! [`MAX_BODY_PIECE`] bytes. The answer's body is sent only as far as the
//! receiver has made room for it ([`INITIAL_ROOM`], then
//! [`ToWebClient::More`]), so that a server that sends without end holds
//! no more memory than that.

use std::io::{Read, Write};

use crate::{Fields, MAX_PAYLOAD, ProtocolError, read_frame, write_frame};

// The web client pipe's kinds, in the next numbers free across every pipe
// on 22 November 2026, after the 233 then in use.
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

/// The longest address, in bytes: the engine's `MAX_URL_LENGTH`, which the
/// fetcher asserts this equals.
pub const MAX_URL: usize = 8000;
/// The longest method: the longest the HTTP method registry holds is
/// `UPDATEREDIRECTREF`, 17 bytes; twice that, rounded up.
pub const MAX_METHOD: usize = 40;
/// The most headers a request carries.
pub const MAX_REQUEST_HEADERS: usize = 64;
/// The most headers an answer's head carries to the trusted core.
pub const MAX_RESPONSE_HEADERS: usize = 128;
/// The longest header name, in bytes.
pub const MAX_HEADER_NAME: usize = 128;
/// The longest header value, in bytes: what common servers accept for one
/// field line.
pub const MAX_HEADER_VALUE: usize = 8 * 1024;
/// The most bytes of names and values together in one head, a request's or
/// an answer's.
pub const MAX_HEADER_BYTES: usize = 64 * 1024;
/// The longest status text of an answer, in bytes (RFC 9112 section 4: a
/// reason phrase, which HTTP/2 does not have).
pub const MAX_STATUS_TEXT: usize = 128;
/// The longest integrity metadata, in bytes: three hashes of the longest
/// kind, SHA-512's 88 bytes of base64 and its `sha512-` prefix, with
/// options.
pub const MAX_INTEGRITY: usize = 512;
/// The most bytes of a body in one message.
pub const MAX_BODY_PIECE: usize = 64 * 1024;
/// The largest request body, in bytes. The web client holds a request's
/// body whole before it sends it, so that a redirect can send it again.
pub const MAX_REQUEST_BODY: usize = 16 * 1024 * 1024;
/// The bytes of an answer's body a request starts with room for; the
/// receiver makes more with [`ToWebClient::More`].
pub const INITIAL_ROOM: u32 = 256 * 1024;
/// The most room a request may have at once, in bytes.
pub const MAX_ROOM: u32 = 4 * 1024 * 1024;
/// The longest event type of an event stream, in bytes.
pub const MAX_EVENT_TYPE: usize = 256;
/// The longest last event ID of an event stream, in bytes.
pub const MAX_EVENT_ID: usize = 1024;
/// The longest data of one event of an event stream, in bytes; a longer
/// event ends the stream with [`Reason::TooLarge`].
pub const MAX_EVENT_DATA: usize = 128 * 1024;
/// The longest reason of a WebSocket's close, in bytes (RFC 6455 section
/// 5.5: a control frame's payload of 125 bytes less the two of the code).
pub const MAX_CLOSE_REASON: usize = 123;
/// The most bytes of one part of a cookie jar written out for the trusted
/// core to keep (the browser's DEC-7268 item 3): room for a channel's
/// whole jar at the cookie standard's limits, which the fetcher asserts.
pub const MAX_JAR_BYTES: usize = 4 * 1024 * 1024;
/// The most web client processes alive at once, browser-wide (DEC-7268
/// item 4): one for each partition with a page open, the least recently
/// used ending first. The trusted core holds to it; a process for each
/// of the most tabs a window shows.
pub const MAX_WEB_CLIENTS: usize = crate::MAX_TABS;
/// The longest subprotocol a WebSocket's server chose, in bytes.
pub const MAX_SUBPROTOCOL: usize = 128;

// Every message fits one payload.
const _: () = assert!(MAX_BODY_PIECE + 8 <= MAX_PAYLOAD);
const _: () = assert!(5 * MAX_URL + MAX_HEADER_BYTES + 4 * MAX_RESPONSE_HEADERS + MAX_INTEGRITY + MAX_METHOD + MAX_STATUS_TEXT + 64 <= MAX_PAYLOAD);
const _: () = assert!(MAX_EVENT_TYPE + MAX_EVENT_ID + MAX_EVENT_DATA + 16 <= MAX_PAYLOAD);

/// An enumeration carried as one byte.
macro_rules! coded {
    ($(#[$meta:meta])* $name:ident, $what:literal { $($(#[$variant_meta:meta])* $variant:ident = $code:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)+
        }

        impl $name {
            /// Every value.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            const fn code(self) -> u8 {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            const fn of(code: u8) -> Result<Self, ProtocolError> {
                match code {
                    $($code => Ok(Self::$variant),)+
                    _ => Err(ProtocolError::BadValue($what)),
                }
            }
        }
    };
}

coded! {
    /// What a request is for.
    RequestKind, "request kind" {
        /// A script's `fetch` (the Fetch standard).
        Fetch = 0,
        /// A script's `EventSource` (the HTML standard, server-sent events).
        EventSource = 1,
        /// A script's `WebSocket`: reserved, and refused until built.
        WebSocket = 2,
        /// The browser's own load of a page, a navigation: it carries no
        /// header of the caller's, and no `Referer` or `Origin`.
        Page = 3,
    }
}

coded! {
    /// The Fetch standard's request mode.
    Mode, "request mode" {
        /// Only the client's own origin.
        SameOrigin = 0,
        /// Another origin if it allows it (CORS).
        Cors = 1,
        /// Another origin, with an answer the script cannot read.
        NoCors = 2,
        /// A navigation, the browser's own.
        Navigate = 3,
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
    /// Where an address is, from most public to most private (the Local
    /// Network Access draft's address spaces, which neoteletext-common's
    /// DEC-7315 follows).
    AddressSpace, "address space" {
        /// The internet.
        Public = 0,
        /// The local network: private, link-local and shared addresses.
        Local = 1,
        /// This computer itself.
        Loopback = 2,
    }
}

coded! {
    /// The Fetch standard's response type.
    ResponseType, "response type" {
        /// Same origin: every header but the cookie ones.
        Basic = 0,
        /// Another origin that allowed it: the headers it exposes.
        Cors = 1,
        /// Another origin, unreadable: no status, header or body.
        Opaque = 2,
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
    }
}

coded! {
    /// Why a request failed, by kind alone, for the console: never text
    /// from outside.
    Reason, "failure reason" {
        /// The script aborted it, or its tab closed.
        Aborted = 0,
        /// The address is no address, or not `http` or `https`.
        Address = 1,
        /// The port is one the Fetch standard blocks.
        BadPort = 2,
        /// The method is one the Fetch standard forbids, or no method.
        Method = 3,
        /// The address is on this computer or the local network, in an
        /// address space more private than the page's: it needs the
        /// viewer's permission (neoteletext-common's DEC-7315). Until the
        /// browser can ask for it, the request is refused; this reason is
        /// what the trusted core turns into the asking.
        LocalPermission = 4,
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
        /// The answer broke a rule of HTTP.
        Protocol = 18,
        /// A limit on size or count was passed.
        TooLarge = 19,
        /// The body's content coding was unknown or its bytes malformed.
        Coding = 20,
        /// Too many requests under way for the tab.
        Busy = 21,
        /// An event stream's answer was not an event stream.
        NotEventStream = 22,
        /// The kind of request is not built.
        Unsupported = 23,
        /// The request itself was malformed.
        Request = 24,
    }
}

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
    /// Bytes the script holds, which a redirect may send again;
    /// [`ToWebClient::RequestBody`] messages and a
    /// [`ToWebClient::RequestBodyEnd`] follow.
    Bytes,
    /// A stream, which cannot be sent twice (the Fetch standard's body
    /// with a null source); the same messages follow.
    Stream,
}

const BODY_NONE: u8 = 0;
const BODY_BYTES: u8 = 1;
const BODY_STREAM: u8 = 2;

/// A header: its name, and its value's bytes.
pub type Header = (String, Vec<u8>);

/// A request's start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start {
    /// The request's number, the trusted core's choice, which every later
    /// message of the request carries.
    pub id: u32,
    /// The tab it belongs to: its requests are counted and closed
    /// together.
    pub tab: u32,
    /// The jar of windows: 0 the ordinary windows', another a private
    /// window's. With [`Start::channel`] it names the partition the
    /// request belongs to (neoteletext-common's DEC-7264). A web client
    /// process serves one partition, the one it was started for (the
    /// browser's DEC-7268): a start that names another ends the process.
    pub jar: u32,
    /// The identity of the channel asking, as the trusted core knows it:
    /// the address of its folder, ending in `/` (the browser's REQ-712).
    /// Never the script's word.
    pub channel: String,
    /// What it is for.
    pub kind: RequestKind,
    /// The method, as the script side normalized it.
    pub method: String,
    /// The address, absolute.
    pub url: String,
    /// The address of the page asking, from which its origin and its
    /// referrer come; empty for a page with no address, whose origin is
    /// opaque.
    pub client: String,
    /// The address space the page asking was itself loaded from, as the
    /// trusted core knows it; for a page the viewer asked for by hand,
    /// [`AddressSpace::Loopback`], the viewer being at this computer. A
    /// request to a more private space than this needs the viewer's
    /// permission (neoteletext-common's DEC-7315).
    pub client_space: AddressSpace,
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
    /// The headers the script may read.
    pub headers: Vec<Header>,
}

/// A piece of a cookie jar written out (the browser's DEC-7268 item 3).
/// The web client writes no file: after a change it sends its jar whole,
/// in pieces, for the trusted core to seal and keep for its partition, and
/// is given it back when a process for the partition next starts. The
/// bytes are the web client's own writing and the trusted core reads none
/// of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JarPiece {
    /// Whether these are the cookies that end with the session, which the
    /// trusted core keeps in memory and never writes; else those that
    /// outlive it.
    pub session: bool,
    /// Whether this is the part's last piece. The pieces of one part are
    /// at most [`MAX_JAR_BYTES`] together.
    pub last: bool,
    /// At most [`MAX_BODY_PIECE`] bytes; none only in a last piece.
    pub bytes: Vec<u8>,
}

impl JarPiece {
    fn check(&self) -> Result<(), ProtocolError> {
        if self.bytes.len() > MAX_BODY_PIECE || (self.bytes.is_empty() && !self.last) { Err(ProtocolError::BadValue("jar piece")) } else { Ok(()) }
    }

    fn write(&self) -> Result<Vec<u8>, ProtocolError> {
        self.check()?;
        let mut payload = vec![u8::from(self.session), u8::from(self.last)];
        payload.extend_from_slice(&self.bytes);
        Ok(payload)
    }

    fn read(fields: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let piece = Self { session: fields.flag("session")?, last: fields.flag("last")?, bytes: fields.take(fields.remaining())?.to_vec() };
        piece.check()?;
        Ok(piece)
    }
}

/// From the trusted core to the web client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToWebClient {
    /// A request starts.
    Start(Box<Start>),
    /// The next bytes of a request's body.
    RequestBody {
        /// The request's number.
        id: u32,
        /// At most [`MAX_BODY_PIECE`] bytes, and not none.
        bytes: Vec<u8>,
    },
    /// A request's body is complete.
    RequestBodyEnd(u32),
    /// A request is aborted; a [`FromWebClient::Failed`] answers, unless
    /// the request had already ended.
    Abort(u32),
    /// Room for this many more bytes of a request's answer.
    More {
        /// The request's number.
        id: u32,
        /// The bytes, not none; a request never has more than
        /// [`MAX_ROOM`] at once.
        bytes: u32,
    },
    /// A tab closes: each of its requests is aborted, unanswered.
    TabCloses(u32),
    /// A piece of the partition's cookie jar as the trusted core kept it,
    /// given before the first request.
    JarKept(JarPiece),
    /// Reserved: a message to send on a WebSocket.
    SocketSend {
        /// The request's number.
        id: u32,
        /// Whether binary, else text.
        binary: bool,
        /// At most [`MAX_BODY_PIECE`] bytes.
        bytes: Vec<u8>,
    },
    /// Reserved: a WebSocket's close, from the script.
    SocketClose {
        /// The request's number.
        id: u32,
        /// The close code.
        code: u16,
        /// The reason.
        reason: String,
    },
}

/// From the web client to the trusted core. A fetcher is untrusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromWebClient {
    /// It is confined and running, said first as every helper says it.
    Confined,
    /// An answer's head.
    Head(Box<Head>),
    /// The next bytes of an answer's body.
    Body {
        /// The request's number.
        id: u32,
        /// At most [`MAX_BODY_PIECE`] bytes, and not none.
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
        /// Why, for the console.
        reason: Reason,
    },
    /// An event stream is open, or a WebSocket (reserved).
    Opened {
        /// The request's number.
        id: u32,
        /// The address that answered, whose origin is each event's; for a
        /// WebSocket, the subprotocol the server chose.
        text: String,
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
    /// Reserved: a message received on a WebSocket.
    SocketMessage {
        /// The request's number.
        id: u32,
        /// Whether binary, else text.
        binary: bool,
        /// At most [`MAX_BODY_PIECE`] bytes.
        bytes: Vec<u8>,
    },
    /// A piece of the partition's cookie jar, written out after a change
    /// for the trusted core to keep.
    Jar(JarPiece),
    /// Reserved: a WebSocket closed.
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
}

/// Whether `byte` is a token character (RFC 9110 section 5.6.2), of which
/// a method and a header name are made.
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

fn check_token(text: &str, max: usize, what: &'static str) -> Result<(), ProtocolError> {
    if text.is_empty() || text.len() > max || !text.bytes().all(is_token_byte) { Err(ProtocolError::BadValue(what)) } else { Ok(()) }
}

/// An address on the pipe: visible ASCII, as the URL standard writes one.
fn check_url(text: &str, what: &'static str) -> Result<(), ProtocolError> {
    if text.len() > MAX_URL || !text.bytes().all(|byte| byte.is_ascii_graphic()) { Err(ProtocolError::BadValue(what)) } else { Ok(()) }
}

fn check_headers(headers: &[Header], most: usize) -> Result<(), ProtocolError> {
    if headers.len() > most {
        return Err(ProtocolError::BadValue("too many headers"));
    }
    let mut total = 0_usize;
    for (name, value) in headers {
        check_token(name, MAX_HEADER_NAME, "header name")?;
        if value.len() > MAX_HEADER_VALUE || !is_header_value(value) {
            return Err(ProtocolError::BadValue("header value"));
        }
        total = total.saturating_add(name.len()).saturating_add(value.len());
    }
    if total > MAX_HEADER_BYTES { Err(ProtocolError::BadValue("headers too long")) } else { Ok(()) }
}

fn check_piece(bytes: &[u8]) -> Result<(), ProtocolError> {
    if bytes.is_empty() || bytes.len() > MAX_BODY_PIECE { Err(ProtocolError::BadValue("body piece")) } else { Ok(()) }
}

fn check_text(text: &str, max: usize, what: &'static str) -> Result<(), ProtocolError> {
    if text.len() > max { Err(ProtocolError::BadValue(what)) } else { Ok(()) }
}

/// A status text: visible ASCII, spaces and tabs (RFC 9112 section 4,
/// without its obsolete bytes).
fn check_status_text(text: &str) -> Result<(), ProtocolError> {
    if text.len() > MAX_STATUS_TEXT || !text.bytes().all(|byte| byte.is_ascii_graphic() || matches!(byte, b' ' | b'\t')) {
        Err(ProtocolError::BadValue("status text"))
    } else {
        Ok(())
    }
}

fn check_room(bytes: u32) -> Result<(), ProtocolError> {
    if bytes == 0 || bytes > MAX_ROOM { Err(ProtocolError::BadValue("room")) } else { Ok(()) }
}

/// A status a head may carry: 0 for an unreadable answer, else a final
/// HTTP status (RFC 9110 section 15).
fn check_status(status: u16, response_type: ResponseType) -> Result<(), ProtocolError> {
    let unreadable = matches!(response_type, ResponseType::Opaque | ResponseType::OpaqueRedirect);
    if (unreadable && status == 0) || (!unreadable && (200..=599).contains(&status)) { Ok(()) } else { Err(ProtocolError::BadValue("status")) }
}

fn push_bytes(payload: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ProtocolError> {
    payload.extend_from_slice(&u32::try_from(bytes.len()).map_err(|_| ProtocolError::BadValue("length"))?.to_be_bytes());
    payload.extend_from_slice(bytes);
    Ok(())
}

fn take_bytes<'a>(fields: &mut Fields<'a>, max: usize, what: &'static str) -> Result<&'a [u8], ProtocolError> {
    let length = usize::try_from(fields.u32()?).map_err(|_| ProtocolError::BadValue(what))?;
    if length > max {
        return Err(ProtocolError::BadValue(what));
    }
    fields.take(length)
}

fn take_string(fields: &mut Fields<'_>, max: usize, what: &'static str) -> Result<String, ProtocolError> {
    std::str::from_utf8(take_bytes(fields, max, what)?).map(str::to_owned).map_err(|_| ProtocolError::BadValue(what))
}

fn push_headers(payload: &mut Vec<u8>, headers: &[Header], most: usize) -> Result<(), ProtocolError> {
    check_headers(headers, most)?;
    payload.extend_from_slice(&u16::try_from(headers.len()).map_err(|_| ProtocolError::BadValue("too many headers"))?.to_be_bytes());
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
        let name = take_string(fields, MAX_HEADER_NAME, "header name")?;
        let value = take_bytes(fields, MAX_HEADER_VALUE, "header value")?.to_vec();
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
        check_token(&self.method, MAX_METHOD, "method")?;
        check_url(&self.url, "address")?;
        if self.url.is_empty() {
            return Err(ProtocolError::BadValue("address"));
        }
        check_url(&self.client, "client address")?;
        check_url(&self.channel, "channel identity")?;
        if !self.channel.ends_with('/') {
            return Err(ProtocolError::BadValue("channel identity"));
        }
        check_headers(&self.headers, MAX_REQUEST_HEADERS)?;
        if let Referrer::Url(url) = &self.referrer {
            check_url(url, "referrer")?;
        }
        if self.integrity.len() > MAX_INTEGRITY || !self.integrity.bytes().all(|byte| byte.is_ascii_graphic() || byte == b' ') {
            return Err(ProtocolError::BadValue("integrity"));
        }
        Ok(())
    }

    fn write(&self, payload: &mut Vec<u8>) -> Result<(), ProtocolError> {
        self.check()?;
        payload.extend_from_slice(&self.id.to_be_bytes());
        payload.extend_from_slice(&self.tab.to_be_bytes());
        payload.extend_from_slice(&self.jar.to_be_bytes());
        payload.extend_from_slice(&[
            self.kind.code(),
            self.mode.code(),
            self.credentials.code(),
            self.cache.code(),
            self.redirect.code(),
            self.referrer_policy.code(),
            self.client_space.code(),
            u8::from(self.keepalive),
            match self.body {
                BodySource::None => BODY_NONE,
                BodySource::Bytes => BODY_BYTES,
                BodySource::Stream => BODY_STREAM,
            },
        ]);
        push_bytes(payload, self.method.as_bytes())?;
        push_bytes(payload, self.url.as_bytes())?;
        push_bytes(payload, self.client.as_bytes())?;
        push_bytes(payload, self.channel.as_bytes())?;
        match &self.referrer {
            Referrer::None => payload.push(REFERRER_NONE),
            Referrer::Client => payload.push(REFERRER_CLIENT),
            Referrer::Url(url) => {
                payload.push(REFERRER_URL);
                push_bytes(payload, url.as_bytes())?;
            }
        }
        push_bytes(payload, self.integrity.as_bytes())?;
        push_headers(payload, &self.headers, MAX_REQUEST_HEADERS)
    }

    fn read(fields: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let id = fields.u32()?;
        let tab = fields.u32()?;
        let jar = fields.u32()?;
        let kind = RequestKind::of(fields.u8()?)?;
        let mode = Mode::of(fields.u8()?)?;
        let credentials = Credentials::of(fields.u8()?)?;
        let cache = CacheMode::of(fields.u8()?)?;
        let redirect = RedirectMode::of(fields.u8()?)?;
        let referrer_policy = ReferrerPolicy::of(fields.u8()?)?;
        let client_space = AddressSpace::of(fields.u8()?)?;
        let keepalive = fields.flag("keepalive")?;
        let body = match fields.u8()? {
            BODY_NONE => BodySource::None,
            BODY_BYTES => BodySource::Bytes,
            BODY_STREAM => BodySource::Stream,
            _ => return Err(ProtocolError::BadValue("body source")),
        };
        let method = take_string(fields, MAX_METHOD, "method")?;
        let url = take_string(fields, MAX_URL, "address")?;
        let client = take_string(fields, MAX_URL, "client address")?;
        let channel = take_string(fields, MAX_URL, "channel identity")?;
        let referrer = match fields.u8()? {
            REFERRER_NONE => Referrer::None,
            REFERRER_CLIENT => Referrer::Client,
            REFERRER_URL => Referrer::Url(take_string(fields, MAX_URL, "referrer")?),
            _ => return Err(ProtocolError::BadValue("referrer")),
        };
        let integrity = take_string(fields, MAX_INTEGRITY, "integrity")?;
        let headers = take_headers(fields, MAX_REQUEST_HEADERS)?;
        let start = Self {
            id,
            tab,
            jar,
            channel,
            kind,
            method,
            url,
            client,
            client_space,
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
        let status_text = take_string(fields, MAX_STATUS_TEXT, "status text")?;
        let url = take_string(fields, MAX_URL, "address")?;
        let headers = take_headers(fields, MAX_RESPONSE_HEADERS)?;
        let head = Self { id, response_type, status, status_text, url, redirected, headers };
        head.check()?;
        Ok(head)
    }
}

fn with_id(id: u32) -> Vec<u8> {
    id.to_be_bytes().to_vec()
}

fn piece(id: u32, flag: Option<bool>, bytes: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    check_piece(bytes)?;
    let mut payload = with_id(id);
    if let Some(flag) = flag {
        payload.push(u8::from(flag));
    }
    payload.extend_from_slice(bytes);
    Ok(payload)
}

/// # Errors
/// When writing fails or a field is out of its bounds.
pub fn write_to_client(out: &mut dyn Write, message: &ToWebClient) -> Result<(), ProtocolError> {
    match message {
        ToWebClient::Start(start) => {
            let mut payload = Vec::new();
            start.write(&mut payload)?;
            write_frame(out, KIND_START, &payload)
        }
        ToWebClient::RequestBody { id, bytes } => write_frame(out, KIND_REQUEST_BODY, &piece(*id, None, bytes)?),
        ToWebClient::RequestBodyEnd(id) => write_frame(out, KIND_REQUEST_BODY_END, &with_id(*id)),
        ToWebClient::Abort(id) => write_frame(out, KIND_ABORT, &with_id(*id)),
        ToWebClient::More { id, bytes } => {
            check_room(*bytes)?;
            let mut payload = with_id(*id);
            payload.extend_from_slice(&bytes.to_be_bytes());
            write_frame(out, KIND_MORE, &payload)
        }
        ToWebClient::TabCloses(tab) => write_frame(out, KIND_TAB_CLOSES, &with_id(*tab)),
        ToWebClient::JarKept(piece) => write_frame(out, KIND_JAR_KEPT, &piece.write()?),
        ToWebClient::SocketSend { id, binary, bytes } => write_frame(out, KIND_SOCKET_SEND, &piece(*id, Some(*binary), bytes)?),
        ToWebClient::SocketClose { id, code, reason } => {
            check_text(reason, MAX_CLOSE_REASON, "close reason")?;
            let mut payload = with_id(*id);
            payload.extend_from_slice(&code.to_be_bytes());
            payload.extend_from_slice(reason.as_bytes());
            write_frame(out, KIND_SOCKET_CLOSE, &payload)
        }
    }
}

/// The rest of a payload as a body's piece.
fn rest_piece(fields: &mut Fields<'_>) -> Result<Vec<u8>, ProtocolError> {
    let bytes = fields.take(fields.remaining())?;
    check_piece(bytes)?;
    Ok(bytes.to_vec())
}

/// The rest of a payload as text of at most `max` bytes.
fn rest_text(fields: &mut Fields<'_>, max: usize, what: &'static str) -> Result<String, ProtocolError> {
    let bytes = fields.take(fields.remaining())?;
    if bytes.len() > max {
        return Err(ProtocolError::BadValue(what));
    }
    std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| ProtocolError::BadValue(what))
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind the trusted core does not send here, and any
/// payload that is malformed or out of its bounds.
pub fn read_to_client(input: &mut dyn Read) -> Result<Option<ToWebClient>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_START => ToWebClient::Start(Box::new(Start::read(&mut fields)?)),
        KIND_REQUEST_BODY => ToWebClient::RequestBody { id: fields.u32()?, bytes: rest_piece(&mut fields)? },
        KIND_REQUEST_BODY_END => ToWebClient::RequestBodyEnd(fields.u32()?),
        KIND_ABORT => ToWebClient::Abort(fields.u32()?),
        KIND_MORE => {
            let id = fields.u32()?;
            let bytes = fields.u32()?;
            check_room(bytes)?;
            ToWebClient::More { id, bytes }
        }
        KIND_TAB_CLOSES => ToWebClient::TabCloses(fields.u32()?),
        KIND_JAR_KEPT => ToWebClient::JarKept(JarPiece::read(&mut fields)?),
        KIND_SOCKET_SEND => ToWebClient::SocketSend { id: fields.u32()?, binary: fields.flag("binary")?, bytes: rest_piece(&mut fields)? },
        KIND_SOCKET_CLOSE => {
            ToWebClient::SocketClose { id: fields.u32()?, code: fields.u16()?, reason: rest_text(&mut fields, MAX_CLOSE_REASON, "close reason")? }
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// # Errors
/// When writing fails or a field is out of its bounds.
pub fn write_to_core(out: &mut dyn Write, message: &FromWebClient) -> Result<(), ProtocolError> {
    match message {
        FromWebClient::Confined => write_frame(out, KIND_CONFINED, &[]),
        FromWebClient::Head(head) => {
            let mut payload = Vec::new();
            head.write(&mut payload)?;
            write_frame(out, KIND_HEAD, &payload)
        }
        FromWebClient::Body { id, bytes } => write_frame(out, KIND_BODY, &piece(*id, None, bytes)?),
        FromWebClient::End(id) => write_frame(out, KIND_END, &with_id(*id)),
        FromWebClient::Failed { id, error, reason } => {
            let mut payload = with_id(*id);
            payload.extend_from_slice(&[error.code(), reason.code()]);
            write_frame(out, KIND_FAILED, &payload)
        }
        FromWebClient::Opened { id, text } => {
            check_url(text, "opened")?;
            let mut payload = with_id(*id);
            payload.extend_from_slice(text.as_bytes());
            write_frame(out, KIND_OPENED, &payload)
        }
        FromWebClient::Event { id, event, data, last_id } => {
            check_text(event, MAX_EVENT_TYPE, "event type")?;
            check_text(data, MAX_EVENT_DATA, "event data")?;
            check_text(last_id, MAX_EVENT_ID, "event ID")?;
            let mut payload = with_id(*id);
            push_bytes(&mut payload, event.as_bytes())?;
            push_bytes(&mut payload, last_id.as_bytes())?;
            push_bytes(&mut payload, data.as_bytes())?;
            write_frame(out, KIND_EVENT, &payload)
        }
        FromWebClient::Retrying(id) => write_frame(out, KIND_RETRYING, &with_id(*id)),
        FromWebClient::Jar(piece) => write_frame(out, KIND_JAR, &piece.write()?),
        FromWebClient::SocketMessage { id, binary, bytes } => write_frame(out, KIND_SOCKET_MESSAGE, &piece(*id, Some(*binary), bytes)?),
        FromWebClient::SocketClosed { id, code, reason, clean } => {
            check_text(reason, MAX_CLOSE_REASON, "close reason")?;
            let mut payload = with_id(*id);
            payload.extend_from_slice(&code.to_be_bytes());
            payload.push(u8::from(*clean));
            payload.extend_from_slice(reason.as_bytes());
            write_frame(out, KIND_SOCKET_CLOSED, &payload)
        }
    }
}

/// Reads one message from the web client, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind a web client may not send (every
/// `protocol::fetcher` kind among them), and any payload that is malformed
/// or out of its bounds.
pub fn read_from_client(input: &mut dyn Read) -> Result<Option<FromWebClient>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_CONFINED => FromWebClient::Confined,
        KIND_HEAD => FromWebClient::Head(Box::new(Head::read(&mut fields)?)),
        KIND_BODY => FromWebClient::Body { id: fields.u32()?, bytes: rest_piece(&mut fields)? },
        KIND_END => FromWebClient::End(fields.u32()?),
        KIND_FAILED => FromWebClient::Failed { id: fields.u32()?, error: ErrorKind::of(fields.u8()?)?, reason: Reason::of(fields.u8()?)? },
        KIND_OPENED => {
            let id = fields.u32()?;
            let text = rest_text(&mut fields, MAX_URL, "opened")?;
            check_url(&text, "opened")?;
            FromWebClient::Opened { id, text }
        }
        KIND_EVENT => {
            let id = fields.u32()?;
            let event = take_string(&mut fields, MAX_EVENT_TYPE, "event type")?;
            let last_id = take_string(&mut fields, MAX_EVENT_ID, "event ID")?;
            let data = take_string(&mut fields, MAX_EVENT_DATA, "event data")?;
            FromWebClient::Event { id, event, data, last_id }
        }
        KIND_RETRYING => FromWebClient::Retrying(fields.u32()?),
        KIND_JAR => FromWebClient::Jar(JarPiece::read(&mut fields)?),
        KIND_SOCKET_MESSAGE => FromWebClient::SocketMessage { id: fields.u32()?, binary: fields.flag("binary")?, bytes: rest_piece(&mut fields)? },
        KIND_SOCKET_CLOSED => {
            let id = fields.u32()?;
            let code = fields.u16()?;
            let clean = fields.flag("clean")?;
            FromWebClient::SocketClosed { id, code, reason: rest_text(&mut fields, MAX_CLOSE_REASON, "close reason")?, clean }
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

#[cfg(test)]
mod tests;

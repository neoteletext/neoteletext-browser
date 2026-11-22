//! Every bound of the script library's web APIs (script-api.md section
//! 10.6 and departures D8, D21 and D22 of section 9.4), each defined
//! once, here. What crosses to the web client is held to the web client's
//! own bounds, which are `protocol::web`'s and are used by those names.

use crate::limits::{MAX_HOST_BYTES, MAX_HOST_STATE_BYTES, MAX_RESULT_BYTES, TICKS_PER_SECOND};

/// The requests of a container under way at once (D8).
pub const MAX_REQUESTS_IN_FLIGHT: usize = 8;
/// The requests of a container that wait for a place or for the rate;
/// one more is refused (D8).
pub const MAX_REQUESTS_QUEUED: usize = 32;
/// The requests a container starts in one step, aborted or not (section
/// 10.5; the security review's finding of 1,240 requests in one step).
pub const MAX_REQUESTS_PER_STEP: u32 = 8;
/// The requests a container starts in any second of ticks, aborted or not.
pub const MAX_REQUESTS_PER_SECOND: u32 = 32;
/// The event streams of a container open at once (D22).
pub const MAX_EVENT_STREAMS: usize = 4;
/// The event streams a container opens in one step, closed or not (D22).
pub const MAX_EVENT_STREAM_OPENS_PER_STEP: u32 = 2;
/// The sockets of a container open at once (D22).
pub const MAX_SOCKETS: usize = 4;
/// The sockets a container opens in one step, closed or not (D22).
pub const MAX_SOCKET_OPENS_PER_STEP: u32 = 2;
/// The sockets a container opens in any minute of ticks (D22).
pub const MAX_SOCKET_OPENS_PER_MINUTE: u32 = 16;
/// The ticks of that minute.
pub const TICKS_PER_MINUTE: u64 = 60 * TICKS_PER_SECOND;
/// The bytes a socket may have accepted and not yet seen written to the
/// network; a send past it closes the socket (WebSocket standard,
/// section 3.1, `send`: "flag the WebSocket as full").
pub const MAX_WS_BUFFERED_BYTES: u32 = 1024 * 1024;
/// The largest whole socket message, each way (D21): the web client's.
pub const MAX_WS_MESSAGE_BYTES: usize = protocol::web::MAX_WS_MESSAGE_BYTES;
/// The largest request body: the web client's.
pub const MAX_REQUEST_BODY_BYTES: usize = protocol::web::MAX_REQUEST_BODY_BYTES;
/// The most bytes `text()`, `json()`, `arrayBuffer()`, `bytes()`, `blob()`
/// and `formData()` read before they reject (D8). The prelude holds it;
/// the body itself is in the script's own memory, under the container's cap.
pub const MAX_RESPONSE_BUFFER_BYTES: usize = 8 * 1024 * 1024;
/// The bytes of an answer's body a request starts with room for: the web
/// client's.
pub const FETCH_WINDOW_BYTES: u32 = protocol::web::FETCH_WINDOW_BYTES;
/// The most bytes the host holds for a container's network at once:
/// requests waiting to start, messages not yet taken for the web client
/// and answers not yet given to a context (section 10.6, "network
/// buffers"). It is also the most room one request may have.
pub const MAX_NETWORK_BUFFER_BYTES: usize = protocol::web::MAX_NETWORK_BUFFER_BYTES as usize;
/// The largest piece of a body or of a socket message: the web client's.
pub const MAX_CHUNK_BYTES: usize = protocol::web::MAX_CHUNK_BYTES;
/// The most answers of the web client waiting for a step.
pub const MAX_ANSWERS_WAITING: usize = 256;
/// The body bytes of all of a container's keepalive requests under way
/// together (Fetch standard, section 4.5 "HTTP-network-or-cache fetch"
/// step 8.5: 64 kibibytes; departure D9).
pub const MAX_KEEPALIVE_BODY_BYTES: usize = 64 * 1024;
/// The streaming decoders of the encodings other than UTF-8 a context
/// may hold at once (this build's bound; a decoder is freed when its
/// stream is flushed, and all of them with the context).
pub const MAX_DECODERS: usize = 32;
/// The longest label of an encoding asked about: the longest the Encoding
/// standard lists is `unicode-1-1-utf-8`, with room for the white space
/// it lets stand around one.
pub const MAX_ENCODING_LABEL_BYTES: usize = 64;
/// The close code that says a script gave none (RFC 6455 section 7.4.1:
/// 1005 is "no status code was actually present" and is never sent).
pub const WS_NO_CODE: u16 = 1005;
/// The close code of a message too big to process (RFC 6455 section
/// 7.4.1; departure D21).
pub const WS_TOO_BIG: u16 = 1009;
/// The close code of a connection lost without a close frame (RFC 6455
/// section 7.4.1).
pub const WS_ABNORMAL: u16 = 1006;

// The network's buffers fit the host's state with room for every other table.
const _: () = assert!(MAX_NETWORK_BUFFER_BYTES * 2 <= MAX_HOST_STATE_BYTES);
// A decoded piece always fits a result: no encoding of the standard
// decodes one byte to more than three of UTF-8.
const _: () = assert!(MAX_HOST_BYTES * 3 + 16 <= MAX_RESULT_BYTES);
// A whole event of an event stream fits the scratch space an event's
// payload is written to, which is as long as a result.
const _: () = assert!(protocol::web::MAX_EVENT_TYPE_BYTES + protocol::web::MAX_EVENT_ID_BYTES + protocol::web::MAX_EVENT_DATA_BYTES <= MAX_RESULT_BYTES);
const _: () = assert!(MAX_REQUESTS_PER_STEP <= MAX_REQUESTS_PER_SECOND && MAX_REQUESTS_IN_FLIGHT <= MAX_REQUESTS_QUEUED);

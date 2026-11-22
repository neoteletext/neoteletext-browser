//! Every bound and time limit of the web client, each named once. The
//! bounds of what crosses the pipe are `protocol::web`'s.

use std::time::Duration;

use neoteletext_web::consts::{DEFAULT_CONNECT_TIMEOUT_US, DEFAULT_RESPONSE_TIMEOUT_US};

/// The most redirects followed: the Fetch standard's HTTP-redirect fetch
/// fails a request whose redirect count is 20.
pub(crate) const MAX_REDIRECTS: usize = 20;
/// One deadline over a connection's whole opening, the name's resolution,
/// the connection, the proxy's tunnel and the TLS handshake together
/// (finding F-63): the engine's connect limit, as the channel fetcher's.
pub(crate) const OPEN_DEADLINE: Duration = Duration::from_micros(DEFAULT_CONNECT_TIMEOUT_US);
/// The longest wait for an answer's head once the request is sent: the
/// engine's response limit.
pub(crate) const HEAD_TIMEOUT: Duration = Duration::from_micros(DEFAULT_RESPONSE_TIMEOUT_US);
/// The longest a fetch's body may send nothing, twice the longest interval
/// common servers leave between the keep-alive comments of a slow answer.
/// An event stream has no such limit: silence is its ordinary state.
pub(crate) const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a keepalive request goes on after its context or tab has
/// closed: the script API's `KEEPALIVE_SECONDS` (`docs/script-api.md`
/// section 10.6, departure D9).
pub(crate) const KEEPALIVE_DEADLINE: Duration = Duration::from_secs(30);
/// The bytes of the bodies of the keepalive requests under way together:
/// the Fetch standard's 64 kibibytes (its section 4.5, the "inflight
/// keep-alive bytes"), which the script host holds too
/// (`MAX_KEEPALIVE_BODY_BYTES` in its limits) and the web client holds
/// again, the host's word not being trusted.
pub(crate) const MAX_KEEPALIVE_BODY_BYTES: usize = 64 * 1024;
/// The requests of one tab under way at once: every fetch, event stream
/// and WebSocket its script host may have under way (the script API's
/// departures D8 and D22, the protocol crate's numbers by name). More
/// than the six connections a host is commonly given, since HTTP/2
/// shares one.
pub(crate) const MAX_REQUESTS_PER_TAB: usize = protocol::web::MAX_REQUESTS_IN_FLIGHT + protocol::web::MAX_EVENT_STREAMS + protocol::web::MAX_SOCKETS;
/// The requests of one tab that may wait for one of
/// [`MAX_REQUESTS_PER_TAB`]; one more fails as busy.
pub(crate) const MAX_WAITING_PER_TAB: usize = 64;
/// How long an idle HTTP/1.1 connection is kept for its next request:
/// under the 100 seconds after which common servers close one.
pub(crate) const IDLE_CONNECTION: Duration = Duration::from_secs(90);
/// The idle HTTP/1.1 connections kept for one place.
pub(crate) const MAX_IDLE_PER_PLACE: usize = 6;
/// The places connections are kept for at once; past it the idle ones of
/// the place longest unused are closed.
pub(crate) const MAX_POOLED_PLACES: usize = 256;
/// The largest answer's head read, its names and values together.
pub(crate) const MAX_RESPONSE_HEAD: usize = protocol::web::MAX_HEADER_BYTES;
/// The most bytes a coded body may decode to: 64 MiB, four times the
/// largest request body, past which the answer fails rather than a small
/// coded body fill the script's memory.
pub(crate) const MAX_DECODED_BODY: u64 = 64 * 1024 * 1024;
/// The largest body held whole to check its integrity metadata before any
/// of it is given to the script: the largest request body's size.
pub(crate) const MAX_INTEGRITY_BODY: usize = protocol::web::MAX_REQUEST_BODY_BYTES;
/// The longest gzip member header read (RFC 1952 section 2.3): its extra
/// field alone may be 65,535 bytes, and no web server writes one.
pub(crate) const MAX_GZIP_HEADER: usize = 8 * 1024;
/// The preflight answers remembered.
pub(crate) const PREFLIGHT_CACHE_ENTRIES: usize = 256;
/// How long a preflight answer without `Access-Control-Max-Age` is
/// remembered: the Fetch standard's 5 seconds.
pub(crate) const DEFAULT_PREFLIGHT_AGE: Duration = Duration::from_secs(5);
/// The longest a preflight answer is remembered whatever it asks: two
/// hours, the cap the Fetch standard notes user agents commonly impose.
pub(crate) const MAX_PREFLIGHT_AGE: Duration = Duration::from_hours(2);
/// A cookie's name and value together, in bytes (RFC 6265bis section
/// 5.6).
pub(crate) const MAX_COOKIE_BYTES: usize = 4096;
/// A cookie attribute's value, in bytes (RFC 6265bis section 5.6): a
/// longer one is ignored, the cookie kept.
pub(crate) const MAX_COOKIE_ATTRIBUTE: usize = 1024;
/// The longest a cookie lives, in seconds: 400 days (RFC 6265bis sections
/// 5.6.1 and 5.6.2).
pub(crate) const MAX_COOKIE_LIFETIME_SECONDS: i64 = 400 * 24 * 60 * 60;
/// The cookies kept for one domain in one jar: the 50 RFC 6265 section
/// 6.1 asks a user agent to allow at least.
pub(crate) const MAX_COOKIES_PER_DOMAIN: usize = 50;
/// The cookies kept in one jar. A jar is one channel's (neoteletext-common's
/// DEC-7264), not a whole profile's, so a tenth of the 3,000 RFC 6265
/// section 6.1 asks for a profile.
pub(crate) const MAX_COOKIES_PER_JAR: usize = 300;
// A whole jar, written for the trusted core to keep, fits the pipe's bound
// on one: each cookie's text, the address that set it and its numbers.
const _: () = assert!(MAX_COOKIES_PER_JAR * (MAX_COOKIE_BYTES + 3 * MAX_COOKIE_ATTRIBUTE + 512) <= protocol::web::MAX_JAR_BYTES);
/// The requests a tab may start at once after a pause: a second's worth
/// saved up.
pub(crate) const START_BURST: u32 = protocol::web::MAX_REQUESTS_PER_SECOND;
/// The requests a tab may start each second once its burst is spent: the
/// protocol crate's number, which the script host holds first, counting
/// by its ticks. Held here by the clock, with a second's worth saved up,
/// it lets through every run of starts the script host lets through, and
/// its refusal is the authority: a script that starts and aborts
/// requests in a loop is held to this, which a cap on those under way
/// alone would not do (the security review's measurement of 1,240
/// requests in one step of a script).
pub(crate) const STARTS_PER_SECOND: u32 = protocol::web::MAX_REQUESTS_PER_SECOND;
// A run of starts that keeps to the rate in each second never needs more
// saved up than a second's worth.
const _: () = assert!(START_BURST >= STARTS_PER_SECOND);
/// The wait before an event stream is opened again when it names none:
/// "a few seconds" in the HTML standard (section 9.2.2).
pub(crate) const DEFAULT_RETRY: Duration = Duration::from_secs(3);
/// The shortest wait before an event stream is opened again, whatever its
/// `retry` field asks: a server cannot have the browser reconnect in a
/// loop.
pub(crate) const MIN_RETRY: Duration = Duration::from_secs(1);
/// The longest wait an event stream's `retry` field may ask for.
pub(crate) const MAX_RETRY: Duration = Duration::from_hours(1);
/// The longest `Referer` sent whole: past it the origin alone is sent
/// (the Referrer Policy standard, section 8.3).
pub(crate) const MAX_REFERRER: usize = 4096;
/// The messages from the trusted core, and the events to it, that wait in
/// memory: the pipe is read and written no faster than they are handled.
pub(crate) const QUEUED_MESSAGES: usize = 64;
/// The bytes of a body decoded or read at a time: one piece of the pipe.
pub(crate) const PIECE: usize = protocol::web::MAX_CHUNK_BYTES;

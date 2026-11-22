//! What the fuzz targets of `native/fuzz` reach of the web client: each
//! reader of bytes from outside, behind a function that holds it to what
//! it promises.

use url::Url;

use std::cell::RefCell;
use std::future::Future;
use std::net::{IpAddr, Ipv6Addr};
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context as Polling, Poll, Waker};
use std::time::Instant;

use protocol::script::ContextId;
use protocol::web::{
    AddressSpace, BodySource, CacheMode, Credentials, FILE_PAGE_IDENTITY, FromWebClient, MAX_CHUNK_BYTES, MAX_WS_MESSAGE_BYTES, Mode, RedirectMode, Referrer,
    ReferrerPolicy, RequestKind, Start,
};

use super::coding::{Decoder, gzip_header_length};
use super::connect::Settings;
use super::cookies::{Context, Jar, Suffixes, bundled_suffixes, same_site};
use super::cors::{Allowed, PreflightCache, cors_check, exposed};
use super::fetch::{Engine, Reader, redirected};
use super::integrity;
use super::limits::{DECODED_RATIO_FLOOR, MAX_COOKIES_PER_JAR, MAX_DECODED_BODY, MAX_DECOMPRESSED_RATIO, MAX_GZIP_HEADER, MAX_SUFFIX_LIST_BYTES, PIECE};
use super::local::{Access, Asker, is_as_declared, known_local, local_access, space_of};
use super::policy::{ChannelPolicy, Relation};
use super::pool::Pool;
use super::referrer::{policy_of, referer};
use super::rules::is_forbidden_response_header;
use super::socket::{Told, accepted, handshake_request, read_frames};
use super::sse::EventStream;

/// Whether the address `url` is judged inside the channel whose identity
/// is `channel`, and the address's path as it is sent; `None` when either
/// is no address.
#[must_use]
pub fn channel_holds(channel: &str, url: &str) -> Option<(bool, String)> {
    let url = Url::parse(url).ok()?;
    let policy = ChannelPolicy::new(channel)?;
    Some((policy.is_same_origin(&url), url.path().to_owned()))
}

/// Decodes `coded` as the content coding `coding`, `at_a_time` bytes a
/// step into `room` bytes of room, to at most `most` bytes; what it
/// decoded to, or `None` for bytes that are not the coding's or a coding
/// not decoded.
///
/// # Panics
/// When a step takes more than it was given or writes more than its room.
#[must_use]
pub fn decode(coding: &[u8], coded: &[u8], at_a_time: usize, room: usize, most: usize) -> Option<Vec<u8>> {
    let mut decoder = Decoder::of(coding)?;
    let mut buffer = vec![0_u8; room.max(1)];
    let mut out = Vec::new();
    for piece in coded.chunks(at_a_time.max(1)) {
        let mut rest = piece;
        loop {
            let step = decoder.step(rest, &mut buffer).ok()?;
            assert!(step.taken <= rest.len() && step.written <= buffer.len(), "a step kept to its input and its room");
            out.extend_from_slice(buffer.get(..step.written).unwrap_or_default());
            rest = rest.get(step.taken..).unwrap_or_default();
            if out.len() > most || (step.taken == 0 && step.written == 0) {
                break;
            }
        }
        if out.len() > most {
            break;
        }
    }
    Some(out)
}

/// Reads `bytes` as the start of a gzip member.
///
/// # Panics
/// When a header longer than the bound is taken as one.
pub fn gzip_header(bytes: &[u8]) {
    if let Ok(Some(length)) = gzip_header_length(bytes) {
        assert!(length <= MAX_GZIP_HEADER && length <= bytes.len(), "a header is within its bound and its bytes");
    }
}

/// The events of the event stream `bytes`, given `at_a_time` bytes a
/// call, as type, data and last event ID; `None` when a bound is passed.
#[must_use]
pub fn events(bytes: &[u8], at_a_time: usize) -> Option<Vec<(String, String, String)>> {
    let mut stream = EventStream::default();
    let mut found = Vec::new();
    for piece in bytes.chunks(at_a_time.max(1)) {
        stream.push(piece, &mut |event| found.push((event.kind, event.data, event.last_id))).ok()?;
    }
    Some(found)
}

/// Keeps in one jar the cookies each of `headers` sets as a `Set-Cookie`
/// value of an answer from `url`, then reads back `kept` as a jar the
/// trusted core kept, and returns the `Cookie` header for `url` with the
/// jar written out.
///
/// # Panics
/// When the jar holds more than its bound, or a jar written out does not
/// read back to the same cookies.
#[must_use]
pub fn jar(url: &str, headers: &[&[u8]], kept: &[u8]) -> Option<(Option<String>, Vec<u8>)> {
    const SAME: Context = Context { same_site: true, navigation: true, safe_method: true };
    let url = Url::parse(url).ok().filter(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())?;
    let mut jar = Jar::new(None);
    for header in headers {
        jar.store(&url, header, SAME);
    }
    jar.read_written(kept);
    let (lasting, session) = (jar.write(false), jar.write(true));
    assert!(jar.count() <= MAX_COOKIES_PER_JAR, "a jar keeps to its bound");
    let mut again = Jar::new(None);
    again.read_written(&lasting);
    again.read_written(&session);
    assert_eq!(again.header(&url, SAME), jar.header(&url, SAME), "a jar written out reads back");
    Some((jar.header(&url, SAME), [lasting, session].concat()))
}

/// What the CORS rules make of an answer's `headers` (names in lower
/// case) to a request of `origin` with `method` and the unsafe header
/// names `names`, with credentials or without: whether the CORS check
/// passes, whether as a preflight's answer it allows the request (`None`
/// when its lists are malformed), and the names of the headers a script
/// may read.
///
/// # Panics
/// When a header a script may never read is among those it may, or a
/// header is exposed that the answer did not carry.
#[must_use]
pub fn cors(headers: &[(String, Vec<u8>)], origin: &str, with_credentials: bool, method: &str, names: &[String]) -> (bool, Option<bool>, Vec<String>) {
    let passes = cors_check(headers, origin, with_credentials);
    let allows = Allowed::read(headers, with_credentials, Instant::now()).map(|allowed| allowed.allows(method, names));
    let shown = exposed(headers, with_credentials);
    for (name, value) in &shown {
        assert!(!is_forbidden_response_header(name), "{name} is never a script's to read");
        assert!(headers.iter().any(|(carried, bytes)| carried == name && bytes == value), "{name} was not in the answer");
    }
    (passes, allows, shown.into_iter().map(|(name, _)| name).collect())
}

/// Whether `metadata` names a hash the standard knows, and whether
/// `body` matches it.
///
/// # Panics
/// When metadata that names no known hash does not match, or a body does
/// not match its own SHA-256 or another body's does.
#[must_use]
pub fn integrity_of(metadata: &str, body: &[u8]) -> (bool, bool) {
    let (asks, matches) = (integrity::asks(metadata), integrity::matches(metadata, body));
    assert!(asks || matches, "metadata that names no known hash matches anything");
    let own = integrity::metadata_of(body);
    assert!(integrity::matches(&own, body), "a body matches its own hash");
    let mut other = body.to_vec();
    other.push(0);
    assert!(!integrity::matches(&own, &other), "and no other body does");
    (asks, matches)
}

/// Runs `future` to its end where it waits on nothing outside itself: an
/// opening's futures read bytes they were given and write to a sink.
/// `None` when the future waits, which none of the code behind an opening
/// does without a connection: each caller holds that it did not.
fn ready<T>(future: impl Future<Output = T>) -> Option<T> {
    let mut future = pin!(future);
    match future.as_mut().poll(&mut Polling::from_waker(Waker::noop())) {
        Poll::Ready(value) => Some(value),
        Poll::Pending => None,
    }
}

/// The partition `channel` names (a channel identity, or the identity of
/// a page opened from disk), with nothing connected and an empty jar that
/// keeps to `suffixes`; `None` when the text is no identity.
fn engine(channel: &str, suffixes: Suffixes, space: AddressSpace) -> Option<Engine> {
    let policy = ChannelPolicy::new(channel)?;
    let jar = if policy.is_file_page() { Jar::unkept(suffixes) } else { Jar::new(suffixes) };
    let settings = Settings::new(&[], None, rustls::RootCertStore::empty());
    Some(Engine { policy, pool: Pool::new(settings), jar: RefCell::new(jar), preflights: RefCell::new(PreflightCache::default()), space })
}

/// A script's request of `kind` for `url` from a page of `channel`.
fn start(channel: &str, kind: RequestKind, url: &str, with_credentials: bool) -> Start {
    Start {
        id: 1,
        context: ContextId::Page,
        kind,
        method: "GET".to_owned(),
        url: url.to_owned(),
        client: format!("{channel}page"),
        headers: vec![("authorization".to_owned(), b"x".to_vec())],
        mode: Mode::Cors,
        credentials: if with_credentials { Credentials::Include } else { Credentials::SameOrigin },
        cache: CacheMode::Default,
        redirect: RedirectMode::Follow,
        referrer: Referrer::Client,
        referrer_policy: ReferrerPolicy::Unset,
        integrity: String::new(),
        keepalive: false,
        body: BodySource::None,
        protocols: Vec::new(),
        last_event_id: String::new(),
        target_space: None,
    }
}

/// The Public Suffix List built in, read once for the thread.
fn bundled() -> Suffixes {
    thread_local! {
        static LIST: Suffixes = bundled_suffixes().unwrap_or_default();
    }
    LIST.with(Clone::clone)
}

/// What a request of `base`, made by a page of `channel` (a channel
/// identity, or `null` for a page from disk), is after an answer of
/// `status` whose one `Location` is `location` and whose
/// `Referrer-Policy` is `policy`: the address asked next, or `None` when
/// the answer is not followed. `kept` is a jar the trusted core kept,
/// read first, so that the next address is asked with its cookies; the
/// preflight cache is keyed with what results.
///
/// It reaches `fetch`'s reading of a redirect whole: the join, the
/// fragment carried over, the address rules, the limit, the credentials
/// test, the method's change, the tainted origin, the channel policy,
/// the referrer under the answer's policy, the jar's `header` for the new
/// address and the preflight cache's key.
///
/// # Panics
/// When a followed redirect leads to what is no web address, from `https`
/// to anything else, or to an address with a user name outside the
/// channel; when `Authorization` crosses to another origin or out of the
/// channel; when the referrer told has a fragment or a user name (what
/// each policy tells is [`referrer`]'s to hold); or when a remembered
/// preflight answers for another address.
#[must_use]
pub fn location(channel: &str, base: &str, status: u16, location: &[u8], policy: &[u8], kept: &[u8]) -> Option<String> {
    let engine = engine(channel, bundled(), AddressSpace::Public)?;
    engine.jar.borrow_mut().read_written(kept);
    let asked = Url::parse(base).ok()?;
    let start = start(channel, RequestKind::Fetch, base, true);
    let headers = vec![("location".to_owned(), location.to_vec()), ("referrer-policy".to_owned(), policy.to_vec())];
    let next = redirected(&engine, &start, status, &headers).ok()??;
    let url = &next.url;
    assert!(matches!(url.scheme(), "http" | "https") && url.host().is_some(), "{url} is no web address");
    assert!(asked.scheme() != "https" || url.scheme() == "https", "{asked} led down to {url}");
    assert_eq!(next.own, engine.policy.is_same_origin(url) && next.origin != "null", "{url}: the channel's own or not");
    assert!((url.username().is_empty() && url.password().is_none()) || next.own, "{url} has a user name and is not the channel's");
    let stays = asked.origin() == url.origin() && engine.policy.is_same_origin(&asked) == engine.policy.is_same_origin(url);
    assert!(stays || !next.headers.iter().any(|name| name == "authorization"), "Authorization went from {asked} to {url}");
    assert_eq!(next.method, "GET", "a GET stays one");
    let told = next.referrer.as_deref().map(Url::parse);
    assert!(told.as_ref().is_none_or(Result::is_ok), "a referrer told is an address");
    if let Some(Ok(told)) = &told {
        assert!(told.fragment().is_none() && told.username().is_empty() && told.password().is_none(), "{told} tells too much");
    }
    // The next address is asked with the jar's cookies for it, and its
    // preflight is remembered for it alone.
    let context = Context { same_site: next.same_site, navigation: false, safe_method: true };
    let _ = engine.jar.borrow().header(url, context);
    let allows = [("access-control-allow-methods".to_owned(), b"PUT".to_vec()), ("access-control-max-age".to_owned(), b"60".to_vec())];
    if let Some(allowed) = Allowed::read(&allows, false, Instant::now()) {
        let mut cache = engine.preflights.borrow_mut();
        cache.remember(&next.origin, url, false, allowed, Instant::now());
        assert!(cache.allows(&next.origin, url, false, "PUT", &[], Instant::now()), "a remembered preflight answers for its address");
        let mut other = url.clone();
        other.set_path(&format!("{}x", url.path()));
        assert!(!cache.allows(&next.origin, &other, false, "PUT", &[], Instant::now()), "and for no other");
        assert!(!cache.allows("https://another.example", url, false, "PUT", &[], Instant::now()), "nor for another origin");
    }
    Some(url.as_str().to_owned())
}

/// What `socket::accepted` makes of `answer`, the whole head of a
/// server's answer to a WebSocket handshake made with `key` that offered
/// `protocols`: the subprotocol chosen, empty for none, or `None` when
/// the answer does not open the socket.
///
/// # Panics
/// When an answer is taken that chose a subprotocol not offered.
#[must_use]
pub fn handshake(answer: &[u8], key: &[u8], protocols: &[String]) -> Option<String> {
    let chosen = accepted(answer, key, protocols).ok()?;
    assert!(chosen.is_empty() || protocols.contains(&chosen), "{chosen} was not offered");
    Some(chosen)
}

/// A socket's messages as a script is told them: whether binary, and the
/// bytes.
pub type SocketMessages = Vec<(bool, Vec<u8>)>;
/// How a socket ended: its close code, and whether cleanly.
pub type SocketEnd = (u16, bool);

/// A `Told` that keeps what a script would be told.
struct Kept(RefCell<Vec<FromWebClient>>);

impl Told for Kept {
    async fn tell(&self, event: FromWebClient) {
        self.0.borrow_mut().push(event);
    }

    async fn room(&self, _count: usize) {}

    async fn jar(&self) {}
}

/// What `socket::Reading::take` tells a script of `bytes` read as a
/// server's frames, `at_a_time` bytes a call, with what it would write
/// back (pongs, a close) discarded: each message as (binary, bytes), and
/// the close that ended the socket as (code, clean), if one did.
///
/// # Panics
/// When a piece of a message is longer than the pipe's piece, a message
/// is longer than `MAX_WS_MESSAGE_BYTES`, a message's last piece is not
/// its last, or anything is told after the socket ended.
#[must_use]
pub fn frames(bytes: &[u8], at_a_time: usize) -> (SocketMessages, Option<SocketEnd>) {
    let kept = Kept(RefCell::new(Vec::new()));
    let read = ready(read_frames(bytes, at_a_time, &kept));
    assert!(read.is_some(), "reading frames waited on something");
    let ended = read.flatten();
    let mut messages = SocketMessages::new();
    let mut open = false;
    for event in kept.0.into_inner() {
        let piece = if let FromWebClient::SocketMessage { binary, last, bytes, .. } = event { Some((binary, last, bytes)) } else { None };
        assert!(piece.is_some(), "a socket's reading told what is no message");
        let Some((binary, last, bytes)) = piece else { continue };
        assert!(bytes.len() <= MAX_CHUNK_BYTES, "a piece of {} bytes", bytes.len());
        match messages.last_mut().filter(|_| open) {
            Some(message) => {
                assert_eq!(message.0, binary, "a message's pieces are of one kind");
                message.1.extend_from_slice(&bytes);
            }
            None => messages.push((binary, bytes)),
        }
        open = !last;
        assert!(messages.last().is_some_and(|message| message.1.len() <= MAX_WS_MESSAGE_BYTES), "a message past its bound");
    }
    assert!(!open, "a message was left without its last piece");
    assert!(ended.as_ref().is_none_or(|event| matches!(event, FromWebClient::SocketClosed { .. })), "a socket ends with its closing");
    let ended = ended.and_then(|event| if let FromWebClient::SocketClosed { code, clean, .. } = event { Some((code, clean)) } else { None });
    (messages, ended)
}

/// The request a WebSocket's handshake makes for the address `url`, asked
/// by a page of `channel` that offers `protocols`: the address it is sent
/// to and the request's head; `None` when the socket may not be opened.
/// Nothing is connected to.
///
/// # Panics
/// When the handshake goes to what is not an `http` or `https` address
/// with a host and no fragment, or its head is not one request: a line
/// break inside a line, or more or fewer than one `Host`, `Origin`,
/// `Upgrade` and key.
#[must_use]
pub fn socket_address(channel: &str, url: &str, protocols: &[String]) -> Option<(String, Vec<u8>)> {
    const KEY: &[u8] = b"dGhlIHNhbXBsZSBub25jZQ==";
    let engine = engine(channel, None, AddressSpace::Loopback)?;
    let mut start = start(channel, RequestKind::WebSocket, url, true);
    start.protocols = protocols.to_vec();
    let (sent_to, head) = handshake_request(&engine, &start, KEY).ok()?;
    assert!(matches!(sent_to.scheme(), "http" | "https") && sent_to.host().is_some() && sent_to.fragment().is_none(), "{sent_to}");
    let text = String::from_utf8_lossy(&head);
    assert!(std::str::from_utf8(&head).is_ok() && text.ends_with("\r\n\r\n"), "a head is text that ends with an empty line: {text:?}");
    let lines: Vec<&str> = text.trim_end_matches("\r\n").split("\r\n").collect();
    assert!(lines.iter().all(|line| !line.is_empty() && !line.contains(['\r', '\n', '\0'])), "a line of the head is broken: {text:?}");
    assert!(lines.first().is_some_and(|line| line.starts_with("GET /") && line.ends_with(" HTTP/1.1") && line.split(' ').count() == 3), "{text:?}");
    for name in ["Host: ", "Origin: ", "Upgrade: ", "Sec-WebSocket-Key: ", "Connection: ", "Sec-WebSocket-Version: "] {
        assert_eq!(lines.iter().filter(|line| line.starts_with(name)).count(), 1, "{name} in {text:?}");
    }
    assert_eq!(lines.iter().filter(|line| line.starts_with("Sec-WebSocket-Protocol: ")).count(), usize::from(!protocols.is_empty()), "{text:?}");
    Some((sent_to.into(), head))
}

/// Reads `coded` as an answer's body in the content coding `coding`
/// through `fetch::Reader::piece`, the reader a script's body comes
/// through, with no connection: the decoded length, or `None` when the
/// body fails (bytes that are not the coding's, a body cut short, or one
/// that decodes past its bounds) or the coding is not decoded.
///
/// # Panics
/// When a piece is longer than the reader's piece, or a body is given
/// that is past 64 MiB or, past the floor, more than the ratio times its
/// coded length.
#[must_use]
pub fn body_ratio(coding: &[u8], coded: &[u8]) -> Option<u64> {
    let mut reader = Reader::of_bytes(coding, coded)?;
    let mut decoded = 0_u64;
    loop {
        let read = ready(reader.piece());
        assert!(read.is_some(), "a body with no connection waited on something");
        let Some(piece) = read?.ok()? else { break };
        assert!(piece.len() <= PIECE, "a piece of {} bytes", piece.len());
        decoded = decoded.saturating_add(u64::try_from(piece.len()).unwrap_or(u64::MAX));
        let coded_bytes = u64::try_from(coded.len()).unwrap_or(u64::MAX);
        assert!(decoded <= MAX_DECODED_BODY, "{decoded} bytes decoded");
        assert!(decoded <= DECODED_RATIO_FLOOR || decoded <= coded_bytes.saturating_mul(MAX_DECOMPRESSED_RATIO), "{decoded} bytes from {coded_bytes}");
    }
    Some(decoded)
}

/// Keeps in a jar that has a public suffix list the cookies each of
/// `headers` sets as a `Set-Cookie` value of an answer from `url`, and
/// gives the `Cookie` header that jar then sends to `other`. The list is
/// the one built in when `list` is `None`, else `list` read as a list is
/// written; `None` when an address is no web address or the list does
/// not read.
///
/// # Panics
/// When a cookie set by one host is sent to a host that is not of its
/// site by the list: the rule that no cookie is set for a public suffix
/// (RFC 6265bis section 5.7), which keeps one site's cookies from
/// another's.
#[must_use]
pub fn jar_with_suffixes(list: Option<&str>, url: &str, headers: &[&[u8]], other: &str) -> Option<Option<String>> {
    const SAME: Context = Context { same_site: true, navigation: true, safe_method: true };
    let web = |text: &str| Url::parse(text).ok().filter(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some());
    let (url, other) = (web(url)?, web(other)?);
    let suffixes = match list {
        None => bundled(),
        Some(text) if text.len() <= MAX_SUFFIX_LIST_BYTES => Some(Rc::new(text.parse::<publicsuffix::List>().ok()?)),
        Some(_) => return None,
    };
    let mut jar = Jar::new(suffixes.clone());
    for header in headers {
        jar.store(&url, header, SAME);
    }
    assert!(jar.count() <= MAX_COOKIES_PER_JAR, "a jar keeps to its bound");
    let sent = jar.header(&other, SAME);
    // Sites are compared without regard to scheme here: a cookie's
    // domain has none.
    let (mut one, mut two) = (url.clone(), other.clone());
    let same = one.set_scheme("https").is_ok() && two.set_scheme("https").is_ok() && same_site(&suffixes, &one, &two);
    assert!(sent.is_none() || same, "{url} set a cookie that went to {other}: {sent:?}");
    Some(sent)
}

/// How the address `url` stands to the partition `channel` names (a
/// channel identity, or `null` for a page from disk) by
/// `ChannelPolicy::relation`: 0 inside the channel, 1 of its origin and
/// outside it, 2 of another origin; `None` when either text is not what
/// it should be.
///
/// # Panics
/// When an address judged inside the channel is not of its origin and
/// under its folder, an address of another origin is judged of the
/// channel's, or anything is judged a page from disk's own.
#[must_use]
pub fn relation(channel: &str, url: &str) -> Option<u8> {
    let policy = ChannelPolicy::new(channel)?;
    let url = Url::parse(url).ok()?;
    let relation = policy.relation(&url);
    assert_eq!(policy.is_same_origin(&url), relation == Relation::InChannel, "{url}");
    match policy.folder() {
        None => assert_eq!((channel, relation), (FILE_PAGE_IDENTITY, Relation::CrossOrigin), "{url}: a page from disk owns no address"),
        Some(folder) => {
            let same_origin = folder.origin() == url.origin() && url.origin().is_tuple();
            assert_eq!(relation != Relation::CrossOrigin, same_origin, "{url} against {folder}");
            assert!(relation != Relation::InChannel || url.path().starts_with(folder.path()), "{url} is not under {folder}");
        }
    }
    Some(match relation {
        Relation::InChannel => 0,
        Relation::SameOriginOutside => 1,
        Relation::CrossOrigin => 2,
    })
}

/// The `Referer` a request to `target` carries from the page `page`
/// under the referrer policy of the number `policy`
/// (`protocol::web::ReferrerPolicy`'s code), `same` saying whether the
/// target is the page's own channel's; and the policy the
/// `Referrer-Policy` value `header` sets, as its code. `None` when an
/// address is none or the number is no policy's.
///
/// # Panics
/// When a referrer is told under `no-referrer`, has a fragment or a user
/// name, is not the page's own origin's, or is more than the origin
/// where the policy tells an origin alone.
#[must_use]
pub fn referrer(policy: u8, page: &str, target: &str, same: bool, header: &[u8]) -> Option<(Option<String>, Option<u8>)> {
    let policy = ReferrerPolicy::of(policy).ok()?;
    let (page, target) = (Url::parse(page).ok()?, Url::parse(target).ok()?);
    let told = referer(policy, &page, &target, same);
    let parsed = told.as_deref().map(Url::parse);
    assert!(parsed.as_ref().is_none_or(Result::is_ok), "a referrer told is an address");
    if let Some(Ok(told)) = &parsed {
        assert!(policy != ReferrerPolicy::NoReferrer, "{told} told under no-referrer");
        assert!(told.fragment().is_none() && told.username().is_empty() && told.password().is_none(), "{told} tells too much");
        assert!(!page.origin().is_tuple() || told.origin() == page.origin(), "{told} is not of the page's origin");
        let origin_alone = matches!(policy, ReferrerPolicy::Origin | ReferrerPolicy::StrictOrigin)
            || (!same && matches!(policy, ReferrerPolicy::OriginWhenCrossOrigin | ReferrerPolicy::StrictOriginWhenCrossOrigin | ReferrerPolicy::Unset));
        assert!(!origin_alone || (told.path() == "/" && told.query().is_none()), "{told} is more than an origin");
        assert!(same || policy != ReferrerPolicy::SameOrigin, "{told} told to another channel under same-origin");
    }
    let set = policy_of(&[("referrer-policy".to_owned(), header.to_vec())]).map(ReferrerPolicy::code);
    Some((told, set))
}

/// The address-space decisions for a request to `url` that the script
/// declared of the space `declared` (`AddressSpace`'s code, or none) and
/// that connected to `peer` (an address in IPv6's sixteen bytes, an IPv4
/// one in its mapped form), asked by a page of the space `page_space`
/// that is a secure context or not: the peer's space as its code, whether
/// the address is known local before it is resolved, whether the peer is
/// as declared, and the rule's answer (0 allowed, 1 needs the
/// local-network permission, 2 needs this computer's, 3 refused). `None`
/// when a number is no space's or the address is none.
///
/// # Panics
/// When a declared space is held met by a peer of another, an undeclared
/// request is held not met, an address is known local that was neither
/// declared so nor written as a local address or a `.local` name, or the
/// rule lets a page from the internet reach a local or loopback address
/// unasked, or stops any other.
#[must_use]
pub fn address_space(url: &str, declared: Option<u8>, peer: [u8; 16], page_space: u8, secure: bool) -> Option<(u8, bool, bool, u8)> {
    let url = Url::parse(url).ok()?;
    let declared = match declared {
        None => None,
        Some(code) => Some(AddressSpace::of(code)?),
    };
    let page_space = AddressSpace::of(page_space)?;
    let peer = IpAddr::V6(Ipv6Addr::from(peer));
    let space = space_of(peer);
    let as_declared = is_as_declared(Some(peer), declared);
    assert_eq!(as_declared, declared.is_none_or(|declared| declared == space), "{peer} as {declared:?}");
    assert!(is_as_declared(None, declared) == declared.is_none_or(|declared| declared == AddressSpace::Public), "through a proxy");
    let known = known_local(&url, declared);
    let written = match url.host() {
        Some(url::Host::Ipv4(address)) => space_of(IpAddr::V4(address)) == AddressSpace::Local,
        Some(url::Host::Ipv6(address)) => space_of(IpAddr::V6(address)) == AddressSpace::Local,
        Some(url::Host::Domain(name)) => name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase().ends_with(".local"),
        None => false,
    };
    assert_eq!(known, declared == Some(AddressSpace::Local) || written, "{url} declared {declared:?}");
    let access = local_access(Asker { space: page_space, secure }, peer, &[]);
    let expected = match (page_space, space, secure) {
        (AddressSpace::Local | AddressSpace::Loopback, _, _) | (AddressSpace::Public, AddressSpace::Public, _) => Access::Allowed,
        (AddressSpace::Public, _, false) => Access::Refused,
        (AddressSpace::Public, AddressSpace::Local, true) => Access::NeedsLocalNetwork,
        (AddressSpace::Public, AddressSpace::Loopback, true) => Access::NeedsThisDevice,
    };
    assert_eq!(access, expected, "{page_space:?} (secure: {secure}) asking {peer}");
    let answer = match access {
        Access::Allowed => 0,
        Access::NeedsLocalNetwork => 1,
        Access::NeedsThisDevice => 2,
        Access::Refused => 3,
    };
    Some((space.code(), known, as_declared, answer))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::net::Ipv4Addr;

    use super::*;

    const CHANNEL: &str = "https://host.example/a/";

    fn gzip(body: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(body).unwrap();
        encoder.finish().unwrap()
    }

    /// Each opening of TODO-7438 reaches the code it names and gives what
    /// that code gives: a redirect's next address.
    #[test]
    fn location_follows_a_redirect_as_fetch_does() {
        let follow = |channel: &str, base: &str, location: &[u8]| location_of(channel, base, location);
        assert_eq!(follow(CHANNEL, "https://host.example/a/x#frag", b"y?q=1").as_deref(), Some("https://host.example/a/y?q=1#frag"));
        assert_eq!(follow(CHANNEL, "https://host.example/a/x#frag", b"https://other.example/z#own").as_deref(), Some("https://other.example/z#own"));
        assert_eq!(follow(CHANNEL, "https://host.example/a/x", b"/a/\xE9").as_deref(), Some("https://host.example/a/%C3%A9"), "bytes above 0x7F are followed");
        assert_eq!(follow(CHANNEL, "https://host.example/a/x", b"http://host.example/a/y"), None, "never down from https");
        assert_eq!(follow(CHANNEL, "https://host.example/a/x", b"file:///etc/passwd"), None);
        assert_eq!(follow(CHANNEL, "https://other.example/x", b"https://user:pass@third.example/"), None, "credentials in a cross-origin redirect");
        assert_eq!(follow(FILE_PAGE_IDENTITY, "http://localhost:8080/x", b"/y").as_deref(), Some("http://localhost:8080/y"));
        assert_eq!(follow("no identity", "https://host.example/a/x", b"y"), None);
        assert_eq!(
            location(CHANNEL, "https://host.example/a/x", 200, b"", b"", b""),
            Some("https://host.example/a/x".to_owned()),
            "an empty Location is the address itself"
        );
    }

    fn location_of(channel: &str, base: &str, to: &[u8]) -> Option<String> {
        location(channel, base, 302, to, b"no-referrer, strict-origin", b"")
    }

    #[test]
    fn handshake_takes_only_this_handshakes_acceptance() {
        let key = b"dGhlIHNhbXBsZSBub25jZQ==";
        let accept = String::from_utf8(neoteletext_web::ws::accept_for(key).to_vec()).unwrap();
        let answer = |more: &str| {
            format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n{more}\r\n")
        };
        let offered = vec!["chat".to_owned()];
        assert_eq!(handshake(answer("").as_bytes(), key, &offered).as_deref(), Some(""));
        assert_eq!(handshake(answer("Sec-WebSocket-Protocol: chat\r\n").as_bytes(), key, &offered).as_deref(), Some("chat"));
        assert_eq!(handshake(answer("Sec-WebSocket-Protocol: other\r\n").as_bytes(), key, &offered), None);
        assert_eq!(handshake(answer("").as_bytes(), b"another key", &offered), None);
        assert_eq!(handshake(b"HTTP/1.1 200 OK\r\n\r\n", key, &offered), None);
        assert_eq!(handshake(b"\xFF\x00", key, &offered), None);
    }

    #[test]
    fn frames_reads_a_servers_frames_in_any_pieces() {
        let bytes = [&[0x01, 2, b'h', b'i'][..], &[0x89, 0], &[0x80, 1, b'!'], &[0x82, 0], &[0x88, 2, 0x03, 0xE8], &[0x81, 1, b'x']].concat();
        for at_a_time in [1, 3, 1000] {
            let (messages, ended) = frames(&bytes, at_a_time);
            assert_eq!(messages, vec![(false, b"hi!".to_vec()), (true, Vec::new())], "{at_a_time}");
            assert_eq!(ended, Some((1000, true)), "{at_a_time}");
        }
        assert_eq!(frames(&[0x81, 0x81, 1, 2, 3, 4, b'x'], 2), (Vec::new(), Some((1006, false))), "a server's masked frame");
        assert_eq!(frames(&[0x81, 2, b'h'], 1), (Vec::new(), None), "a frame not yet whole");
    }

    #[test]
    fn socket_address_gives_the_handshakes_request() {
        let (sent_to, head) = socket_address(CHANNEL, "wss://host.example/a/live?room=1#part", &["chat".to_owned()]).unwrap();
        assert_eq!(sent_to, "https://host.example/a/live?room=1");
        let head = String::from_utf8(head).unwrap();
        assert!(head.starts_with("GET /a/live?room=1 HTTP/1.1\r\nHost: host.example\r\n") && head.contains("\r\nOrigin: https://host.example\r\n"), "{head}");
        assert_eq!(socket_address(CHANNEL, "ws://other.example/live", &[]), None, "mixed content");
        assert_eq!(socket_address(CHANNEL, "https://host.example/a/live", &[]), None, "no socket's scheme");
        assert_eq!(socket_address(CHANNEL, "wss://host.example/a/live", &["a b".to_owned()]), None);
        assert!(
            socket_address(FILE_PAGE_IDENTITY, "ws://device.example:8080/", &[])
                .is_some_and(|(_, head)| head.windows(14).any(|window| window == b"Origin: null\r\n"))
        );
    }

    #[test]
    fn body_ratio_reads_a_body_through_the_readers_bounds() {
        assert_eq!(body_ratio(b"gzip", &gzip(b"hello, hello, hello")), Some(19));
        let floor = usize::try_from(DECODED_RATIO_FLOOR).unwrap();
        assert_eq!(body_ratio(b"gzip", &gzip(&vec![0; floor / 2])), Some(DECODED_RATIO_FLOOR / 2));
        assert_eq!(body_ratio(b"gzip", &gzip(&vec![0; 4 * floor])), None, "past the ratio");
        assert_eq!(body_ratio(b"gzip", b"not gzip"), None);
        assert_eq!(body_ratio(b"gzip", gzip(b"cut short").get(..12).unwrap()), None);
        assert_eq!(body_ratio(b"zstd", b"anything"), None, "a coding not decoded");
    }

    #[test]
    fn jar_with_suffixes_keeps_a_sites_cookies_from_another() {
        let shop = "https://www.example.co.uk/";
        assert_eq!(jar_with_suffixes(None, shop, &[b"a=1; Domain=co.uk"], "https://other.co.uk/"), Some(None));
        assert_eq!(jar_with_suffixes(None, shop, &[b"a=1; Domain=example.co.uk"], "https://shop.example.co.uk/"), Some(Some("a=1".to_owned())));
        assert_eq!(jar_with_suffixes(None, shop, &[b"a=1"], "https://shop.example.co.uk/"), Some(None), "a host's own cookie");
        let list = "// ===BEGIN ICANN DOMAINS===\nzone\nco.zone\n";
        assert_eq!(jar_with_suffixes(Some(list), "https://a.b.co.zone/", &[b"a=1; Domain=co.zone"], "https://c.co.zone/"), Some(None));
        assert_eq!(jar_with_suffixes(Some(list), "https://a.b.co.zone/", &[b"a=1; Domain=b.co.zone"], "https://c.b.co.zone/"), Some(Some("a=1".to_owned())));
        assert_eq!(jar_with_suffixes(Some(""), shop, &[b"a=1"], shop), None, "what is no list");
        assert_eq!(jar_with_suffixes(None, "ftp://host.example/", &[], shop), None);
    }

    #[test]
    fn relation_says_how_an_address_stands_to_a_partition() {
        assert_eq!(relation(CHANNEL, "https://host.example/a/100.tti"), Some(0));
        assert_eq!(relation(CHANNEL, "https://host.example/b/"), Some(1));
        assert_eq!(relation(CHANNEL, "https://host.example/a/..%2fb/x"), Some(1));
        assert_eq!(relation(CHANNEL, "http://host.example/a/x"), Some(2));
        assert_eq!(relation(FILE_PAGE_IDENTITY, "file:///home/viewer/page.tti"), Some(2));
        assert_eq!(relation(FILE_PAGE_IDENTITY, "https://host.example/a/"), Some(2));
        assert_eq!(relation("file:///home/", "https://host.example/a/"), None);
        assert_eq!(relation(CHANNEL, "no address"), None);
    }

    #[test]
    fn referrer_tells_what_the_policy_lets_it() {
        let page = "https://host.example/a/page?q=1#part";
        let told = |policy: ReferrerPolicy, target: &str, same: bool| referrer(policy.code(), page, target, same, b"").unwrap().0;
        assert_eq!(told(ReferrerPolicy::Unset, "https://host.example/a/x", true).as_deref(), Some("https://host.example/a/page?q=1"));
        assert_eq!(told(ReferrerPolicy::Unset, "https://other.example/", false).as_deref(), Some("https://host.example/"));
        assert_eq!(told(ReferrerPolicy::Unset, "http://other.example/", false), None, "a downgrade");
        assert_eq!(told(ReferrerPolicy::NoReferrer, "https://host.example/a/x", true), None);
        assert_eq!(told(ReferrerPolicy::SameOrigin, "https://other.example/", false), None);
        assert_eq!(told(ReferrerPolicy::UnsafeUrl, "http://other.example/", false).as_deref(), Some("https://host.example/a/page?q=1"));
        assert_eq!(referrer(ReferrerPolicy::Unset.code(), page, page, true, b"bogus, origin").unwrap().1, Some(ReferrerPolicy::Origin.code()));
        assert_eq!(referrer(ReferrerPolicy::Unset.code(), page, page, true, b"ORIGIN").unwrap().1, None);
        assert_eq!(referrer(0xFF, page, page, true, b""), None, "no policy's number");
    }

    #[test]
    fn address_space_gives_each_decision_of_the_rule() {
        let v4 = |text: &str| text.parse::<Ipv4Addr>().unwrap().to_ipv6_mapped().octets();
        let (public, local, loopback) = (AddressSpace::Public.code(), AddressSpace::Local.code(), AddressSpace::Loopback.code());
        // A public https page asking a name that connects to the local network.
        assert_eq!(address_space("https://device.example/", None, v4("192.168.1.10"), public, true), Some((local, false, true, 1)));
        assert_eq!(address_space("https://device.example/", None, v4("127.0.0.1"), public, true), Some((loopback, false, true, 2)));
        assert_eq!(address_space("http://device.example/", None, v4("10.0.0.1"), public, false), Some((local, false, true, 3)));
        assert_eq!(address_space("https://far.example/", None, v4("93.184.216.34"), public, true), Some((public, false, true, 0)));
        // Declared, and what is known before resolving.
        assert_eq!(address_space("http://device.example/", Some(local), v4("192.168.1.10"), local, true), Some((local, true, true, 0)));
        assert_eq!(address_space("http://device.example/", Some(local), v4("127.0.0.1"), loopback, true), Some((loopback, true, false, 0)));
        assert_eq!(address_space("http://printer.local/", Some(public), v4("192.168.1.10"), loopback, true), Some((local, true, false, 0)));
        assert_eq!(address_space("http://[fe80::1]/", None, Ipv6Addr::LOCALHOST.octets(), loopback, false), Some((loopback, true, true, 0)));
        assert_eq!(address_space("http://device.example/", Some(9), v4("127.0.0.1"), public, true), None);
        assert_eq!(address_space("http://device.example/", None, v4("127.0.0.1"), 0, true), None);
    }
}

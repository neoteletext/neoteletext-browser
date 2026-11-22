//! The network half of the Fetch standard (its section 4): main fetch,
//! HTTP fetch, HTTP-redirect fetch, HTTP-network-or-cache fetch and the
//! CORS-preflight fetch, for one request, to an answer filtered for the
//! script. Where the standard asks whether a request is same-origin with
//! its client, the channel is asked instead (`policy::ChannelPolicy`,
//! neoteletext-common's DEC-7264 item 3).
//!
//! There is no HTTP cache: an empty cache keeps the standard, and the
//! cache modes are followed as far as that goes (`only-if-cached` finds
//! nothing; `no-cache`, `no-store` and `reload` send the headers the
//! standard gives them).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use hyper::body::Bytes;
use protocol::web::{BodySource, CacheMode, Credentials, Header, Mode, Reason, RedirectMode, Referrer, ReferrerPolicy, ResponseType, Start};
use url::Url;

use super::coding::{ACCEPT_ENCODING, Decoder};
use super::cookies::{self, Jar};
use super::cors::{self, Allowed, PreflightCache};
use super::integrity;
use super::limits::{DECODED_RATIO_FLOOR, MAX_DECODED_BODY, MAX_DECOMPRESSED_RATIO, MAX_INTEGRITY_BODY, MAX_REDIRECTS, PIECE};
use super::local::Asker;
use super::policy::ChannelPolicy;
use super::pool::{Answer, Body, Outgoing, Pool};
use super::referrer::{DEFAULT_POLICY, policy_of, referer};
use super::rules::{is_bad_port, is_forbidden_method, is_forbidden_request_header, is_forbidden_response_header};
use protocol::web::AddressSpace;

/// The headers of a request's body, which go when a redirect makes it a
/// `GET` (the Fetch standard, "request-body-header name").
const BODY_HEADERS: [&str; 4] = ["content-encoding", "content-language", "content-location", "content-type"];
/// The conditional headers, with which the cache mode `default` becomes
/// `no-store` (the Fetch standard's HTTP-network-or-cache fetch).
const CONDITIONAL_HEADERS: [&str; 5] = ["if-modified-since", "if-none-match", "if-unmodified-since", "if-match", "if-range"];
/// The statuses that redirect (the Fetch standard, "redirect status").
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];
/// The statuses whose answer has no body (the Fetch standard, "null body
/// status", less the interim one hyper never gives).
const NULL_BODY_STATUSES: [u16; 3] = [204, 205, 304];
/// A proxy asking for authentication, which the web client has none to
/// give (RFC 9110 section 15.5.8).
const PROXY_AUTHENTICATION_REQUIRED: u16 = 407;

/// The browser as it names itself, the channel fetcher's own words
/// (requirement 5.14): its name, its version and its system's family, and
/// nothing of the viewer.
pub(crate) fn user_agent() -> Vec<u8> {
    neoteletext_web::user_agent::UserAgentString::of(crate::agent::product().as_ref()).as_bytes().to_vec()
}

/// What every request of the process shares.
pub(crate) struct Engine {
    /// The policy of the partition the process serves.
    pub(crate) policy: ChannelPolicy,
    /// Its connections.
    pub(crate) pool: Rc<Pool>,
    /// Its cookies.
    pub(crate) jar: RefCell<Jar>,
    /// Its remembered preflights.
    pub(crate) preflights: RefCell<PreflightCache>,
    /// The address space its channel was loaded from, as the trusted core
    /// told the process at its start.
    pub(crate) space: AddressSpace,
}

impl Engine {
    /// The channel asking, for the rule on this computer and the local
    /// network: its address space as the trusted core told the process,
    /// and whether it is a secure context, loaded over `https` or from
    /// this computer itself. Nothing of it is a script host's word.
    pub(crate) fn asker(&self) -> Asker {
        Asker { space: self.space, secure: self.policy.folder().is_some_and(|folder| folder.scheme() == "https" || is_loopback(folder)) }
    }
}

/// An answer, filtered for the script.
pub(crate) struct Fetched {
    /// The response type.
    pub(crate) response_type: ResponseType,
    /// The status, 0 when unreadable.
    pub(crate) status: u16,
    /// The status text.
    pub(crate) status_text: String,
    /// The address that answered, empty when unreadable.
    pub(crate) url: String,
    /// Whether a redirect was followed.
    pub(crate) redirected: bool,
    /// The headers the script may read.
    pub(crate) headers: Vec<Header>,
    /// The body, if the script may read one.
    pub(crate) body: Option<Reader>,
    /// The bytes of the request's body written to the network.
    pub(crate) sent: usize,
}

/// An answer's body as the script is given it: decoded, in pieces of at
/// most [`PIECE`] bytes.
pub(crate) struct Reader {
    /// The network's body, until its end.
    source: Option<Body>,
    /// Bytes read and not yet given or decoded.
    pending: Bytes,
    decoder: Option<Decoder>,
    /// How many bytes the decoder has written.
    decoded: u64,
    /// How many coded bytes the decoder has taken.
    coded: u64,
}

impl Reader {
    /// The body's next piece, or none at its end.
    pub(crate) async fn piece(&mut self) -> Result<Option<Vec<u8>>, Reason> {
        loop {
            if let Some(decoder) = &mut self.decoder {
                let mut room = vec![0_u8; PIECE];
                let step = decoder.step(&self.pending, &mut room).map_err(|_| Reason::Coding)?;
                let _ = self.pending.split_to(step.taken.min(self.pending.len()));
                self.coded = self.coded.saturating_add(u64::try_from(step.taken).unwrap_or(u64::MAX));
                if step.written > 0 {
                    self.decoded = self.decoded.saturating_add(u64::try_from(step.written).unwrap_or(u64::MAX));
                    let past_ratio = self.decoded > DECODED_RATIO_FLOOR && self.decoded > self.coded.saturating_mul(MAX_DECOMPRESSED_RATIO);
                    if self.decoded > MAX_DECODED_BODY || past_ratio {
                        return Err(Reason::TooLarge);
                    }
                    room.truncate(step.written);
                    return Ok(Some(room));
                }
                if step.taken > 0 {
                    continue;
                }
                if !self.pending.is_empty() {
                    // Input waiting with nothing done: only what follows
                    // the coded body's end may be left unread.
                    if !decoder.is_complete() {
                        return Err(Reason::Coding);
                    }
                    self.pending = Bytes::new();
                }
            } else if !self.pending.is_empty() {
                let piece = self.pending.split_to(self.pending.len().min(PIECE));
                return Ok(Some(piece.to_vec()));
            }
            match &mut self.source {
                Some(body) => match body.piece().await? {
                    Some(bytes) => self.pending = bytes,
                    None => self.source = None,
                },
                None => {
                    // A coded body that stops before its end was cut.
                    return if self.decoder.as_ref().is_some_and(|decoder| !decoder.is_complete()) { Err(Reason::Coding) } else { Ok(None) };
                }
            }
        }
    }

    /// The whole body, of at most `most` bytes.
    async fn whole(&mut self, most: usize) -> Result<Vec<u8>, Reason> {
        let mut whole = Vec::new();
        while let Some(piece) = self.piece().await? {
            if whole.len().saturating_add(piece.len()) > most {
                return Err(Reason::TooLarge);
            }
            whole.extend_from_slice(&piece);
        }
        Ok(whole)
    }
}

/// A request under way: the standard's request, as far as the network
/// half needs it.
struct Request<'a> {
    start: &'a Start,
    policy: &'a ChannelPolicy,
    /// The address space the channel was loaded from.
    space: AddressSpace,
    method: String,
    url: Url,
    headers: Vec<Header>,
    body: Option<Bytes>,
    /// The page asking, when the script host named one of the channel's
    /// own; else the channel's folder.
    client: Option<Url>,
    referrer: Option<Url>,
    referrer_policy: ReferrerPolicy,
    tainting: ResponseType,
    /// The standard's tainted origin flag: the request's origin is then
    /// written `null`.
    tainted_origin: bool,
    redirects: usize,
    /// The bytes of the body written to the network, the first time it
    /// was sent.
    sent: Cell<usize>,
    /// Whether every address so far was of the asking page's site.
    same_site: bool,
}

fn is_web(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && url.host().is_some()
}

/// Whether `url` names this computer itself, to which a request from a
/// page loaded securely is not mixed content (the W3C Mixed Content
/// standard's "potentially trustworthy" addresses).
pub(crate) fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => name == "localhost" || name.ends_with(".localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

fn has(headers: &[Header], name: &str) -> bool {
    headers.iter().any(|(header, _)| header == name)
}

fn set(headers: &mut Vec<Header>, name: &str, value: impl Into<Vec<u8>>) {
    headers.retain(|(header, _)| header != name);
    headers.push((name.to_owned(), value.into()));
}

fn set_if_absent(headers: &mut Vec<Header>, name: &str, value: impl Into<Vec<u8>>) {
    if !has(headers, name) {
        headers.push((name.to_owned(), value.into()));
    }
}

impl<'a> Request<'a> {
    /// The request `start` asks for, checked and with only the headers it
    /// may carry.
    fn new(start: &'a Start, engine: &'a Engine, body: Option<Bytes>) -> Result<Self, Reason> {
        let policy = &engine.policy;
        if is_forbidden_method(&start.method) {
            return Err(Reason::Method);
        }
        let url = Url::parse(&start.url).map_err(|_| Reason::Address)?;
        // The page asking is the script host's word: it is taken only when
        // it is a page of the process's own channel, and the channel's
        // folder stands in for anything else.
        let client = Url::parse(&start.client).ok().filter(|client| policy.is_same_origin(client)).or_else(|| policy.folder().cloned());
        let mut headers: Vec<Header> = start.headers.iter().map(|(name, value)| (name.to_ascii_lowercase(), value.clone())).collect();
        // The standard's `Headers` guard, held again here: a script's
        // forbidden headers are dropped.
        headers.retain(|(name, value)| !is_forbidden_request_header(name, value));
        let referrer = match &start.referrer {
            Referrer::None => None,
            Referrer::Client => client.clone(),
            // A referrer the script names is of its own channel, or the
            // page's address stands in (the standard's `Request`
            // constructor, with the channel for the origin).
            Referrer::Url(url) => Url::parse(url).ok().filter(|url| policy.is_same_origin(url)).or_else(|| client.clone()),
        };
        let same_site = client.as_ref().is_some_and(|client| cookies::same_site(engine.jar.borrow().suffixes(), client, &url));
        let referrer_policy = if start.referrer_policy == ReferrerPolicy::Unset { DEFAULT_POLICY } else { start.referrer_policy };
        Ok(Self {
            start,
            policy,
            space: engine.space,
            method: start.method.clone(),
            url,
            headers,
            body,
            client,
            referrer,
            referrer_policy,
            tainting: ResponseType::Basic,
            tainted_origin: false,
            redirects: 0,
            sent: Cell::new(0),
            same_site,
        })
    }

    /// Whether `url` is the asking channel's own: what stands for the
    /// standard's "same origin with request's origin".
    fn is_own(&self, url: &Url) -> bool {
        !self.tainted_origin && self.policy.is_same_origin(url)
    }

    /// The request's origin as it is written (the standard's "serializing
    /// a request origin").
    fn origin(&self) -> String {
        match &self.client {
            Some(client) if !self.tainted_origin => client.origin().ascii_serialization(),
            _ => "null".to_owned(),
        }
    }

    /// The request's current address as an answer shows it: without its
    /// fragment.
    fn shown_url(&self) -> String {
        let mut url = self.url.clone();
        url.set_fragment(None);
        url.into()
    }

    /// The channel asking, for the rule on this computer and the local
    /// network: its address space as the trusted core told the process,
    /// and whether it is a secure context, loaded over `https` or from
    /// this computer itself. Nothing of it is a script host's word.
    fn asker(&self) -> Asker {
        Asker { space: self.space, secure: self.policy.folder().is_some_and(|folder| folder.scheme() == "https" || is_loopback(folder)) }
    }

    /// Whether the request carries and keeps cookies (the standard's
    /// HTTP-network-or-cache fetch, step 8.4).
    fn with_credentials(&self) -> bool {
        self.start.credentials == Credentials::Include || (self.start.credentials == Credentials::SameOrigin && self.tainting == ResponseType::Basic)
    }

    fn cookie_context(&self) -> cookies::Context {
        cookies::Context { same_site: self.same_site, navigation: false, safe_method: matches!(self.method.as_str(), "GET" | "HEAD" | "OPTIONS" | "TRACE") }
    }

    /// Main fetch's step "set request's referrer to the result of invoking
    /// determine request's referrer": the request's referrer becomes what
    /// its policy lets the current address be told, and stays so. A
    /// redirect runs it again on what an earlier address was told, so a
    /// referrer once cut to its origin, or withheld, is never given back
    /// whole under a later, looser policy.
    fn determine_referrer(&mut self) {
        let own = self.is_own(&self.url);
        self.referrer =
            self.referrer.as_ref().and_then(|referrer| referer(self.referrer_policy, referrer, &self.url, own)).and_then(|told| Url::parse(&told).ok());
    }

    /// The `Origin` header's value, if the request carries one (the
    /// standard's "append a request `Origin` header").
    fn origin_header(&self) -> Option<String> {
        if self.tainting == ResponseType::Cors {
            return Some(self.origin());
        }
        if matches!(self.method.as_str(), "GET" | "HEAD") {
            return None;
        }
        let downgraded = self.client.as_ref().is_some_and(|client| client.scheme() == "https") && self.url.scheme() != "https";
        // The referrer policy withholds the origin only of a request that
        // is not in the mode `cors` (the standard's step 3.1).
        let withheld = self.start.mode != Mode::Cors
            && match self.referrer_policy {
                ReferrerPolicy::NoReferrer => true,
                ReferrerPolicy::NoReferrerWhenDowngrade
                | ReferrerPolicy::StrictOrigin
                | ReferrerPolicy::StrictOriginWhenCrossOrigin
                | ReferrerPolicy::Unset => downgraded,
                ReferrerPolicy::SameOrigin => !self.is_own(&self.url),
                ReferrerPolicy::Origin | ReferrerPolicy::OriginWhenCrossOrigin | ReferrerPolicy::UnsafeUrl => false,
            };
        Some(if withheld { "null".to_owned() } else { self.origin() })
    }

    /// The headers every request to the network carries beside its own.
    fn common_headers(&self, headers: &mut Vec<Header>) {
        // The referrer as main fetch last determined it for this address.
        if let Some(referrer) = &self.referrer {
            set(headers, "referer", referrer.as_str());
        }
        if let Some(origin) = self.origin_header() {
            set(headers, "origin", origin);
        }
        set_if_absent(headers, "user-agent", user_agent());
        set_if_absent(headers, "accept", "*/*");
        // A range is of the coded bytes, so none is asked for with one.
        set(headers, "accept-encoding", if has(headers, "range") { "identity" } else { ACCEPT_ENCODING });
    }

    /// The standard's HTTP-network-or-cache fetch, without a cache: the
    /// request with its headers and cookies, sent, and the answer's
    /// cookies kept.
    async fn network(&self, engine: &Engine) -> Result<Answer, Reason> {
        let mut headers = self.headers.clone();
        match &self.body {
            Some(body) => set(&mut headers, "content-length", body.len().to_string()),
            None if matches!(self.method.as_str(), "POST" | "PUT") => set(&mut headers, "content-length", "0"),
            None => {}
        }
        self.common_headers(&mut headers);
        // A script's conditional request under `default` is `no-store` (the
        // standard's step 8.14).
        let conditional = CONDITIONAL_HEADERS.iter().any(|name| has(&headers, name));
        match self.start.cache {
            CacheMode::OnlyIfCached => return Err(Reason::NotCached),
            CacheMode::NoCache => set_if_absent(&mut headers, "cache-control", "max-age=0"),
            // Only `default` is turned by a conditional header; under
            // `force-cache` nothing is added.
            CacheMode::ForceCache => {}
            CacheMode::Default if !conditional => {}
            CacheMode::NoStore | CacheMode::Reload | CacheMode::Default => {
                set_if_absent(&mut headers, "pragma", "no-cache");
                set_if_absent(&mut headers, "cache-control", "no-cache");
            }
        }
        let credentials = self.with_credentials();
        let cookie = if credentials { engine.jar.borrow().header(&self.url, self.cookie_context()) } else { None };
        if let Some(cookie) = cookie {
            set(&mut headers, "cookie", cookie);
        }
        let answer = engine
            .pool
            .send(&Outgoing { method: &self.method, url: &self.url, headers: &headers, body: self.body.clone(), page: self.asker(), credentials })
            .await?;
        if self.redirects == 0 {
            self.sent.set(self.body.as_ref().map_or(0, Bytes::len));
        }
        if answer.status == PROXY_AUTHENTICATION_REQUIRED {
            return Err(Reason::Proxy);
        }
        if credentials {
            let mut jar = engine.jar.borrow_mut();
            for (_, value) in answer.headers.iter().filter(|(name, _)| name == "set-cookie") {
                jar.store(&self.url, value, self.cookie_context());
            }
        }
        Ok(answer)
    }

    /// The standard's CORS-preflight fetch, unless a remembered answer
    /// already allows the request.
    async fn preflight(&self, engine: &Engine, names: &[String]) -> Result<(), Reason> {
        let origin = self.origin();
        let with_credentials = self.start.credentials == Credentials::Include;
        if engine.preflights.borrow_mut().allows(&origin, &self.url, with_credentials, &self.method, names, Instant::now()) {
            return Ok(());
        }
        let mut headers: Vec<Header> = vec![("access-control-request-method".to_owned(), self.method.clone().into_bytes())];
        if !names.is_empty() {
            headers.push(("access-control-request-headers".to_owned(), names.join(",").into_bytes()));
        }
        self.common_headers(&mut headers);
        // A preflight carries the origin whatever its method, and never
        // credentials.
        set(&mut headers, "origin", origin.clone());
        let answer =
            engine.pool.send(&Outgoing { method: "OPTIONS", url: &self.url, headers: &headers, body: None, page: self.asker(), credentials: false }).await?;
        let allowed = (cors::cors_check(&answer.headers, &origin, with_credentials) && (200..300).contains(&answer.status))
            .then(|| Allowed::read(&answer.headers, with_credentials, Instant::now()))
            .flatten()
            .filter(|allowed| allowed.allows(&self.method, names))
            .ok_or(Reason::Preflight)?;
        engine.preflights.borrow_mut().remember(&origin, &self.url, with_credentials, allowed, Instant::now());
        Ok(())
    }

    /// The standard's HTTP-redirect fetch, as far as the next request:
    /// whether `answer` redirects, the request then being the one to
    /// send next.
    fn redirect(&mut self, answer: &Answer, jar: &Jar) -> Result<bool, Reason> {
        // The standard's "location URL" (section 2.2.6): `Location` allows
        // one header, so two are a failure, a network error. Its value is
        // bytes, each the code point of its number, which the address
        // parser percent-encodes above 0x7F: a `Location` that is not
        // UTF-8 is followed, not refused.
        let mut locations = answer.headers.iter().filter(|(name, _)| name == "location").map(|(_, value)| value);
        let Some(location) = locations.next() else { return Ok(false) };
        if locations.next().is_some() {
            return Err(Reason::Redirect);
        }
        let location: String = location.iter().copied().map(char::from).collect();
        let mut target = self.url.join(&location).map_err(|_| Reason::Redirect)?;
        if target.fragment().is_none() {
            target.set_fragment(self.url.fragment());
        }
        if !is_web(&target) {
            return Err(Reason::Redirect);
        }
        if self.redirects >= MAX_REDIRECTS {
            return Err(Reason::RedirectLimit);
        }
        let has_credentials = !target.username().is_empty() || target.password().is_some();
        if has_credentials && (self.tainting == ResponseType::Cors || (self.start.mode == Mode::Cors && !self.is_own(&target))) {
            return Err(Reason::Redirect);
        }
        // Never from `https` to `http` (the engine's REQ-SEC-007).
        if self.url.scheme() == "https" && target.scheme() != "https" {
            return Err(Reason::Downgrade);
        }
        // A stream cannot be sent twice.
        if answer.status != 303 && self.body.is_some() && self.start.body == BodySource::Stream {
            return Err(Reason::Redirect);
        }
        if (matches!(answer.status, 301 | 302) && self.method == "POST") || (answer.status == 303 && !matches!(self.method.as_str(), "GET" | "HEAD")) {
            "GET".clone_into(&mut self.method);
            self.body = None;
            self.headers.retain(|(name, _)| !BODY_HEADERS.contains(&name.as_str()));
        }
        let leaves_origin = self.url.origin() != target.origin();
        // `Authorization` stays with the origin it was given for, and
        // with the channel.
        if leaves_origin || self.policy.is_same_origin(&self.url) != self.policy.is_same_origin(&target) {
            self.headers.retain(|(name, _)| name != "authorization");
        }
        if leaves_origin && !self.is_own(&self.url) {
            self.tainted_origin = true;
        }
        if let Some(policy) = policy_of(&answer.headers) {
            self.referrer_policy = policy;
        }
        if let Some(client) = &self.client {
            self.same_site &= cookies::same_site(jar.suffixes(), client, &target);
        }
        self.url = target;
        self.redirects = self.redirects.saturating_add(1);
        Ok(true)
    }

    /// The answer as the script may read it (the standard's filtered
    /// responses), its body decoded and, when the request names integrity
    /// metadata, already checked whole.
    async fn filtered(&self, answer: Answer, response_type: ResponseType) -> Result<Fetched, Reason> {
        let Answer { status, status_text, headers, mut body } = answer;
        if response_type == ResponseType::OpaqueRedirect {
            if integrity::asks(&self.start.integrity) {
                return Err(Reason::Integrity);
            }
            return Ok(Fetched {
                response_type,
                status: 0,
                status_text: String::new(),
                // The address asked, which the standard keeps in an
                // opaque-redirect answer's list of addresses.
                url: self.shown_url(),
                redirected: false,
                headers: Vec::new(),
                body: None,
                sent: self.sent.get(),
            });
        }
        let reader = if self.method == "HEAD" || NULL_BODY_STATUSES.contains(&status) {
            // Nothing to read; reading its end gives the connection back.
            body.piece().await?;
            None
        } else {
            let mut codings = headers
                .iter()
                .filter(|(name, _)| name == "content-encoding")
                .flat_map(|(_, value)| value.split(|&byte| byte == b','))
                .map(<[u8]>::trim_ascii)
                .filter(|coding| !coding.is_empty() && !coding.eq_ignore_ascii_case(b"identity"));
            let decoder = match (codings.next(), codings.next()) {
                (Some(coding), None) => Decoder::of(coding),
                (None, _) => None,
                // One coding is decoded; a body coded twice is refused.
                (Some(_), Some(_)) => return Err(Reason::Coding),
            };
            let mut reader = Reader { source: Some(body), pending: Bytes::new(), decoder, decoded: 0, coded: 0 };
            if integrity::asks(&self.start.integrity) {
                let whole = reader.whole(MAX_INTEGRITY_BODY).await?;
                if !integrity::matches(&self.start.integrity, &whole) {
                    return Err(Reason::Integrity);
                }
                reader = Reader { source: None, pending: Bytes::from(whole), decoder: None, decoded: 0, coded: 0 };
            }
            Some(reader)
        };
        let url = self.shown_url();
        let headers = if response_type == ResponseType::Cors {
            cors::exposed(&headers, self.start.credentials == Credentials::Include)
        } else {
            headers.into_iter().filter(|(name, _)| !is_forbidden_response_header(name)).collect()
        };
        Ok(Fetched { response_type, status, status_text, url, redirected: self.redirects > 0, headers, body: reader, sent: self.sent.get() })
    }
}

/// Fetches what `start` asks for, with `body` as its body: the Fetch
/// standard's main fetch, to the answer's head. The body is then read
/// from the answer's [`Reader`].
pub(crate) async fn fetch(engine: &Engine, start: &Start, body: Option<Bytes>) -> Result<Fetched, Reason> {
    let mut request = Request::new(start, engine, body)?;
    loop {
        if !is_web(&request.url) {
            return Err(Reason::Address);
        }
        request.determine_referrer();
        if request.url.port_or_known_default().is_none_or(is_bad_port) {
            return Err(Reason::BadPort);
        }
        // A page loaded securely asks nothing of an address that is not.
        let secure_client = request.client.as_ref().is_some_and(|client| client.scheme() == "https");
        if secure_client && request.url.scheme() != "https" && !is_loopback(&request.url) {
            return Err(Reason::MixedContent);
        }
        // Main fetch, step 12: which kind of answer the script may have.
        if request.tainting == ResponseType::Basic && request.is_own(&request.url) {
            request.tainting = ResponseType::Basic;
        } else if start.mode == Mode::SameOrigin {
            return Err(Reason::SameOriginMode);
        } else {
            request.tainting = ResponseType::Cors;
            let names = cors::unsafe_header_names(&request.headers);
            if !cors::is_safelisted_method(&request.method) || !names.is_empty() {
                request.preflight(engine, &names).await?;
            }
        }
        let answer = request.network(engine).await?;
        if request.tainting == ResponseType::Cors && !cors::cors_check(&answer.headers, &request.origin(), start.credentials == Credentials::Include) {
            return Err(Reason::Cors);
        }
        if REDIRECT_STATUSES.contains(&answer.status) {
            match start.redirect {
                RedirectMode::Error => return Err(Reason::Redirect),
                // A script is given an answer it cannot read.
                RedirectMode::Manual => return request.filtered(answer, ResponseType::OpaqueRedirect).await,
                RedirectMode::Follow => {
                    let redirected = request.redirect(&answer, &engine.jar.borrow())?;
                    if redirected {
                        continue;
                    }
                }
            }
        }
        let response_type = request.tainting;
        return request.filtered(answer, response_type).await;
    }
}

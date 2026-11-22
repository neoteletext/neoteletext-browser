//! Cookies (neoteletext-common's DEC-7250, DEC-7260 item 8): the `cookie`
//! crate reads `Set-Cookie`, the `cookie_store` crate keeps a jar and
//! matches it to a request by domain, path, expiry and the Secure rule
//! (RFC 6265), and this module adds what the current standard, RFC
//! 6265bis, asks beyond them: `SameSite`, the name prefixes, the cap on a
//! cookie's lifetime, the limits on size and count, a Secure cookie left
//! alone by a plain answer, and the public suffix rule.
//!
//! A process has one jar, its partition's: a channel identity in a jar of
//! windows (DEC-7264 item 2, the browser's REQ-712 and DEC-7268). That is
//! a departure from the standard, which keeps one jar for a profile.
//! Inside the jar the standard's rules apply unchanged.
//!
//! The process writes no file. After a change it writes the jar out as
//! bytes for the trusted core to keep, the cookies that end with the
//! session apart from those that outlive it, and reads back what the
//! trusted core gives it at its start, each cookie through the same rules
//! as one a server set.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use cookie::time::{Date, Duration, Month, OffsetDateTime};
use cookie::{Cookie as RawCookie, SameSite};
use cookie_store::{Cookie, CookieDomain, CookieStore};
use publicsuffix::{List, Psl};
use url::Url;

use super::limits::{
    MAX_COOKIE_ATTRIBUTE, MAX_COOKIE_BYTES, MAX_COOKIE_LIFETIME_SECONDS, MAX_COOKIES_PER_DOMAIN, MAX_COOKIES_PER_JAR, MAX_SUFFIX_LIST_BYTES, MAX_SUFFIX_RULES,
};

/// The public suffix list, when the jar has one. Without it a cookie
/// is kept for the host that set it alone, whatever its `Domain`
/// attribute, and a site is a host: narrower than the standard, never
/// wider. The browser's jars always have one ([`bundled_suffixes`]); a
/// test may leave it out.
pub(crate) type Suffixes = Option<Rc<List>>;

/// The Public Suffix List built in, unchanged from its publisher
/// (`native/fetcher/data/public-suffix/NOTICE.txt`; DEC-7340).
const BUNDLED_SUFFIXES: &str = include_str!("../../data/public-suffix/public_suffix_list.dat");
const _: () = assert!(BUNDLED_SUFFIXES.len() <= MAX_SUFFIX_LIST_BYTES);

/// The list built in, read.
///
/// # Errors
/// When the list has more rules than [`MAX_SUFFIX_RULES`], none, or does
/// not read as a list: the process then does not start, since a jar
/// without its list would keep cookies by another rule than the one the
/// browser states.
pub(crate) fn bundled_suffixes() -> Result<Suffixes, String> {
    // A rule is a line that is neither empty nor a comment (the list's
    // own format, https://publicsuffix.org/list/).
    let rules = BUNDLED_SUFFIXES.lines().filter(|line| !line.trim().is_empty() && !line.starts_with("//")).count();
    if rules == 0 || rules > MAX_SUFFIX_RULES {
        return Err(format!("the public suffix list has {rules} rules, outside 1 to {MAX_SUFFIX_RULES}"));
    }
    let list = BUNDLED_SUFFIXES.parse::<List>().map_err(|error| format!("the public suffix list does not read: {error}"))?;
    Ok(Some(Rc::new(list)))
}

/// What a jar needs to know of the request a cookie comes with or goes
/// with (RFC 6265bis section 5.2, "same-site and cross-site requests").
#[derive(Clone, Copy, Debug)]
pub(crate) struct Context {
    /// Whether the request is same-site: the page asking, and every
    /// address a redirect passed, of the site asked.
    pub(crate) same_site: bool,
    /// Whether the request is the browser's own load of a page, a
    /// top-level navigation.
    pub(crate) navigation: bool,
    /// Whether its method is a safe one (RFC 9110 section 9.2.1).
    pub(crate) safe_method: bool,
}

/// The site of `host`: its registrable domain by the list, or the host
/// itself without one, for an address, or for a name that is a suffix.
fn site_of<'a>(suffixes: &Suffixes, host: &'a str) -> &'a str {
    suffixes
        .as_ref()
        .and_then(|list| list.domain(host.as_bytes()))
        .and_then(|domain| std::str::from_utf8(domain.as_bytes()).ok())
        .filter(|_| host.parse::<std::net::IpAddr>().is_err())
        .unwrap_or(host)
}

/// Whether two addresses are of one site, schemes included (RFC 6265bis
/// section 5.2 with its "schemeful" comparison).
pub(crate) fn same_site(suffixes: &Suffixes, one: &Url, other: &Url) -> bool {
    match (one.host_str(), other.host_str()) {
        (Some(one_host), Some(other_host)) => one.scheme() == other.scheme() && site_of(suffixes, one_host).eq_ignore_ascii_case(site_of(suffixes, other_host)),
        _ => false,
    }
}

/// Whether `domain` is a public suffix by the list: a name with no
/// registrable domain of its own.
fn is_public_suffix(list: &List, domain: &str) -> bool {
    list.suffix(domain.as_bytes()).is_some_and(|suffix| suffix.as_bytes().eq_ignore_ascii_case(domain.as_bytes()))
}

/// The name prefixes (RFC 6265bis section 4.1.3), compared without regard
/// to case (its section 5.7).
const PREFIX_SECURE: &str = "__secure-";
const PREFIX_HOST: &str = "__host-";
const PREFIX_HTTP: &str = "__http-";
const PREFIX_HOST_HTTP: &str = "__host-http-";

/// Whether a cookie whose name carries a prefix keeps the prefix's
/// promise.
fn prefix_holds(cookie: &RawCookie<'_>) -> bool {
    let name = cookie.name().to_ascii_lowercase();
    let secure = cookie.secure().unwrap_or(false);
    let host_only = secure && cookie.domain().is_none() && cookie.path() == Some("/");
    let http_only = cookie.http_only().unwrap_or(false);
    if name.starts_with(PREFIX_HOST_HTTP) {
        host_only && http_only
    } else if name.starts_with(PREFIX_HOST) {
        host_only
    } else if name.starts_with(PREFIX_HTTP) {
        secure && http_only
    } else if name.starts_with(PREFIX_SECURE) {
        secure
    } else {
        true
    }
}

/// Whether `byte` is a control character a cookie may not hold (RFC
/// 6265bis section 5.6: %x00-08, %x0A-1F and %x7F).
const fn is_refused_control(byte: u8) -> bool {
    matches!(byte, 0x00..=0x08 | 0x0A..=0x1F | 0x7F)
}

/// `text` without the attributes whose value is longer than
/// [`MAX_COOKIE_ATTRIBUTE`], which the standard ignores.
fn without_long_attributes(text: &str) -> String {
    let mut parts = text.split(';');
    let mut kept = parts.next().unwrap_or_default().to_owned();
    for attribute in parts {
        let value = attribute.split_once('=').map_or("", |(_, value)| value.trim());
        if value.len() <= MAX_COOKIE_ATTRIBUTE {
            kept.push(';');
            kept.push_str(attribute);
        }
    }
    kept
}

/// The life of a cookie whose `Max-Age` is zero or less: already over.
const ENDED: Duration = Duration::seconds(-1);

/// The seconds a `Max-Age` attribute's value asks for, read as the
/// standard reads it (RFC 6265bis-22 section 5.6.2): a `-` or a digit,
/// then digits alone. An empty value, or any other, is no `Max-Age` at
/// all. A number past what fits is the longest or the shortest there is.
fn max_age(value: &str) -> Option<i64> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    if value.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // A `-` alone is zero, as the standard's "remainder" of no digits is.
    Some(value.parse::<i64>().unwrap_or(if value.starts_with('-') || digits.is_empty() { 0 } else { i64::MAX }))
}

/// Whether `byte` parts the tokens of a cookie date (RFC 6265bis-22
/// section 5.1.1, `delimiter`).
const fn is_date_delimiter(byte: u8) -> bool {
    matches!(byte, 0x09 | 0x20..=0x2F | 0x3B..=0x40 | 0x5B..=0x60 | 0x7B..=0x7E)
}

/// The number `token` starts with, of `least` to `most` digits and then
/// no digit: the standard's `1*2DIGIT [ non-digit *OCTET ]` and its like.
fn leading_number(token: &[u8], least: usize, most: usize) -> Option<u16> {
    let digits = token.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if digits < least || digits > most {
        return None;
    }
    std::str::from_utf8(token.get(..digits)?).ok()?.parse().ok()
}

/// The hours, minutes and seconds of a `time` token: three numbers of one
/// or two digits with colons between, and then no digit.
fn time_of_day(token: &[u8]) -> Option<(u16, u16, u16)> {
    let mut fields = token.splitn(3, |&byte| byte == b':');
    let (hour, minute, second) = (fields.next()?, fields.next()?, fields.next()?);
    let whole = |field: &[u8]| (field.len() <= 2).then(|| leading_number(field, 1, 2)).flatten().filter(|_| field.iter().all(u8::is_ascii_digit));
    Some((whole(hour)?, whole(minute)?, leading_number(second, 1, 2)?))
}

/// The months' names as a cookie date writes them, in order.
const MONTHS: [&[u8; 3]; 12] = [b"jan", b"feb", b"mar", b"apr", b"may", b"jun", b"jul", b"aug", b"sep", b"oct", b"nov", b"dec"];
/// The year before which a cookie date is no date (RFC 6265bis-22 section
/// 5.1.1 step 5), and the two-digit years that are 19xx and 20xx (steps 3
/// and 4).
const FIRST_COOKIE_YEAR: u16 = 1601;
const TWO_DIGIT_1900S: core::ops::RangeInclusive<u16> = 70..=99;
const TWO_DIGIT_2000S: core::ops::RangeInclusive<u16> = 0..=69;

/// The moment a cookie date names, in seconds since 1970, read by the
/// standard's own algorithm (RFC 6265bis-22 section 5.1.1): its tokens in
/// any order, between any delimiters, the first of each kind taken. The
/// `cookie` crate reads only a few fixed forms, and what it does not read
/// would leave a cookie a server meant to end as a session cookie.
fn cookie_date(value: &str) -> Option<i64> {
    let (mut time, mut day, mut month, mut year) = (None, None, None, None);
    for token in value.as_bytes().split(|&byte| is_date_delimiter(byte)).filter(|token| !token.is_empty()) {
        if time.is_none() && time_of_day(token).is_some() {
            time = time_of_day(token);
        } else if day.is_none() && leading_number(token, 1, 2).is_some() {
            day = leading_number(token, 1, 2);
        } else if month.is_none() && token.get(..3).is_some_and(|start| MONTHS.iter().any(|name| start.eq_ignore_ascii_case(*name))) {
            month = MONTHS.iter().position(|name| token.get(..3).is_some_and(|start| start.eq_ignore_ascii_case(*name)));
        } else if year.is_none() && leading_number(token, 2, 4).is_some() {
            year = leading_number(token, 2, 4);
        }
    }
    let ((hour, minute, second), day, month, mut year) = (time?, day?, month?, year?);
    if TWO_DIGIT_1900S.contains(&year) {
        year = year.saturating_add(1900);
    } else if TWO_DIGIT_2000S.contains(&year) {
        year = year.saturating_add(2000);
    }
    if year < FIRST_COOKIE_YEAR {
        return None;
    }
    // The calendar refuses what is no date: a day past its month's, an
    // hour past 23, a minute or second past 59.
    let month = Month::try_from(u8::try_from(month.saturating_add(1)).ok()?).ok()?;
    let date = Date::from_calendar_date(i32::from(year), month, u8::try_from(day).ok()?).ok()?;
    let moment = date.with_hms(u8::try_from(hour).ok()?, u8::try_from(minute).ok()?, u8::try_from(second).ok()?).ok()?;
    Some(moment.assume_utc().unix_timestamp())
}

/// What a `Set-Cookie` asks of its cookie's life.
#[derive(Default)]
struct Asked {
    /// The seconds the last `Max-Age` the standard reads asks for.
    max_age: Option<i64>,
    /// The moment the last `Expires` the standard reads names, in seconds
    /// since 1970.
    expires: Option<i64>,
}

/// `text` without its `Max-Age` and `Expires` attributes, and what they
/// ask. The `cookie` crate reads both otherwise than the standard (an
/// empty `Max-Age` as the longest life; only a few forms of date), so
/// they are read here and the crate never sees them.
fn lifetime_attributes(text: &str) -> (String, Asked) {
    let mut parts = text.split(';');
    let mut kept = parts.next().unwrap_or_default().to_owned();
    let mut asked = Asked::default();
    for attribute in parts {
        let (name, value) = attribute.split_once('=').unwrap_or((attribute, ""));
        let name = name.trim();
        if name.eq_ignore_ascii_case("max-age") {
            asked.max_age = max_age(value.trim()).or(asked.max_age);
        } else if name.eq_ignore_ascii_case("expires") {
            asked.expires = cookie_date(value).or(asked.expires);
        } else {
            kept.push(';');
            kept.push_str(attribute);
        }
    }
    (kept, asked)
}

/// Where a cookie is kept: its domain, path and name.
type Place = (String, String, String);

/// What the jar notes of a cookie beside the store.
#[derive(Clone, Debug)]
struct Kept {
    /// When it was first kept, as a count: the order `Cookie` lists them
    /// in and the order they are evicted in.
    order: u64,
    /// When it ends, in seconds since 1970; none for a cookie that ends
    /// with the session.
    expires: Option<i64>,
    /// The origin that set it, as an address: what a jar written out names
    /// so the cookie can be read back by the same rules.
    origin: String,
    /// Whether it is a Secure cookie, which makes room last.
    secure: bool,
}

/// How long from now until `at`.
fn lifetime_until(at: OffsetDateTime) -> Duration {
    Duration::seconds(at.unix_timestamp().saturating_sub(now_unix()))
}

/// The first `count` bytes of `bytes`, which then starts after them.
fn take<'a>(bytes: &mut &'a [u8], count: usize) -> Option<&'a [u8]> {
    let (taken, rest) = bytes.split_at_checked(count)?;
    *bytes = rest;
    Some(taken)
}

/// The milliseconds of a second.
const MILLIS_PER_SECOND: i64 = 1000;

/// The milliseconds since 1970, now.
fn now_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|since| i64::try_from(since.as_millis()).ok()).unwrap_or(0)
}

/// The seconds since 1970, now.
fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|since| i64::try_from(since.as_secs()).ok()).unwrap_or(0)
}

/// The process's cookies.
pub(crate) struct Jar {
    store: CookieStore,
    created: HashMap<Place, Kept>,
    next: u64,
    suffixes: Suffixes,
    /// Whether it changed since it was last written out.
    changed: bool,
    /// Whether it lives in this process alone: the jar of a page opened
    /// from disk, never written out and never read back.
    unkept: bool,
}

impl Jar {
    pub(crate) fn new(suffixes: Suffixes) -> Self {
        Self { store: CookieStore::new(), created: HashMap::new(), next: 0, suffixes, changed: false, unkept: false }
    }

    /// The jar of a page opened from disk (DEC-7315 version 2, DEC-7401):
    /// a jar as any other for the requests its page makes with
    /// credentials, which are cross-site every one, so that it sends and
    /// keeps only what a cross-site request may (`SameSite=None; Secure`).
    /// It is that page's alone and in memory alone: never written out for
    /// the trusted core to keep, taking nothing handed back, and gone
    /// with the page's web client. The behaviour followed uses the
    /// profile's one jar there; a jar for each partition is DEC-7264 and
    /// DEC-7268.
    pub(crate) fn unkept(suffixes: Suffixes) -> Self {
        Self { unkept: true, ..Self::new(suffixes) }
    }

    /// The public suffix list the jar keeps to.
    pub(crate) const fn suffixes(&self) -> &Suffixes {
        &self.suffixes
    }

    /// Forgets every cookie.
    pub(crate) fn forget(&mut self) {
        self.store = CookieStore::new();
        self.created.clear();
        self.changed = true;
    }

    /// How many cookies the jar keeps.
    pub(crate) fn count(&self) -> usize {
        self.created.len()
    }

    /// Whether the jar changed since this was last asked.
    pub(crate) fn take_changed(&mut self) -> bool {
        core::mem::take(&mut self.changed) && !self.unkept
    }

    /// Keeps the cookie the `Set-Cookie` value `header` of an answer from
    /// `url` sets, if the standard lets it be kept; whether it was.
    pub(crate) fn store(&mut self, url: &Url, header: &[u8], context: Context) -> bool {
        self.read(url, header, context).is_some_and(|cookie| self.keep(url, cookie))
    }

    /// The cookie `header` sets, checked against everything but the jar's
    /// own contents and limits.
    fn read(&self, url: &Url, header: &[u8], context: Context) -> Option<Cookie<'static>> {
        if header.iter().copied().any(is_refused_control) {
            return None;
        }
        let text = std::str::from_utf8(header).ok()?;
        let pair = text.split(';').next().unwrap_or_default();
        let (name, value) = pair.split_once('=')?;
        if name.trim().len().saturating_add(value.trim().len()) > MAX_COOKIE_BYTES {
            return None;
        }
        let (text, asked) = lifetime_attributes(&without_long_attributes(text));
        let mut raw = RawCookie::parse(text).ok()?.into_owned();
        // `Max-Age` goes before `Expires` (RFC 6265bis-22 section 5.7 step
        // 4), and no cookie outlives the cap, whichever attribute asked.
        // A life of zero or less has already ended.
        let seconds = asked.max_age.or_else(|| asked.expires.map(|at| at.saturating_sub(now_unix())));
        if let Some(seconds) = seconds {
            raw.set_max_age(if seconds > 0 { Duration::seconds(seconds.min(MAX_COOKIE_LIFETIME_SECONDS)) } else { ENDED });
        }
        let secure_origin = url.scheme() == "https";
        let secure = raw.secure().unwrap_or(false);
        if (secure && !secure_origin) || !prefix_holds(&raw) {
            return None;
        }
        // A cookie with no SameSite attribute is Lax; `None` needs Secure.
        let same_site = raw.same_site().unwrap_or(SameSite::Lax);
        if same_site == SameSite::None && !secure {
            return None;
        }
        // An answer to a cross-site request sets only `SameSite=None`
        // cookies, unless it answers a navigation.
        if same_site != SameSite::None && !context.same_site && !context.navigation {
            return None;
        }
        let mut cookie = Cookie::try_from_raw_cookie(&raw, url).ok()?;
        if let CookieDomain::Suffix(domain) = &cookie.domain {
            let narrow = match &self.suffixes {
                Some(list) if is_public_suffix(list, domain) => {
                    if !cookie.domain.host_is_identical(url) {
                        return None;
                    }
                    true
                }
                Some(_) => false,
                // Without the list no wider domain can be trusted.
                None => true,
            };
            if narrow {
                cookie.domain = CookieDomain::host_only(url).ok()?;
            }
        }
        Some(cookie.into_owned())
    }

    /// Keeps `cookie` in the jar, within its limits.
    fn keep(&mut self, url: &Url, cookie: Cookie<'static>) -> bool {
        let secure_origin = url.scheme() == "https";
        // RFC 6265bis section 5.7: a plain answer does not replace a
        // Secure cookie of the same name whose domain and path would also
        // match.
        if !secure_origin
            && self.store.iter_unexpired().any(|kept| {
                kept.secure().unwrap_or(false)
                    && kept.name() == cookie.name()
                    && kept.domain.matches(url)
                    && cookie.path.as_ref().starts_with(kept.path.as_ref())
            })
        {
            return false;
        }
        let place: Place = (String::from(&cookie.domain), String::from(cookie.path.clone()), cookie.name().to_owned());
        let expired = cookie.is_expired();
        let secure = cookie.secure().unwrap_or(false);
        let lifetime = cookie.max_age().or_else(|| cookie.expires_datetime().map(lifetime_until));
        // The end is noted in whole seconds, rounded up: cut down, a cookie
        // set late in a second would be read back from a jar written out
        // as ended while most of its last second remained.
        let ends_at = lifetime.map(|lifetime| {
            let end = now_millis().saturating_add(lifetime.whole_seconds().saturating_mul(MILLIS_PER_SECOND));
            end.saturating_add(MILLIS_PER_SECOND.saturating_sub(1)).div_euclid(MILLIS_PER_SECOND)
        });
        if self.store.insert(cookie, url).is_err() {
            return false;
        }
        self.changed = true;
        if expired {
            self.store.remove(&place.0, &place.1, &place.2);
            self.created.remove(&place);
            return true;
        }
        let origin = url.origin().ascii_serialization();
        // A cookie replaced keeps its place in the order.
        if let Some(kept) = self.created.get_mut(&place) {
            (kept.expires, kept.origin, kept.secure) = (ends_at, origin, secure);
        } else {
            self.created.insert(place.clone(), Kept { order: self.next, expires: ends_at, origin, secure });
            self.next = self.next.saturating_add(1);
        }
        self.evict(&place.0);
        true
    }

    /// Removes excess cookies in the standard's order (RFC 6265bis-22
    /// section 5.7, "remove excess cookies"): what has expired; then, of
    /// `domain` past [`MAX_COOKIES_PER_DOMAIN`], the cookies that are not
    /// Secure before any that is, so that a plain answer cannot push a
    /// secure session out by setting many cookies of its own; then of the
    /// jar past [`MAX_COOKIES_PER_JAR`]. Within each, the earliest kept
    /// goes first: the standard orders by last access, and a jar here is
    /// one channel's, read by every request of it, so the order of
    /// keeping is the one that tells its cookies apart.
    fn evict(&mut self, domain: &str) {
        let gone: Vec<Place> = self
            .store
            .iter_any()
            .filter(|cookie| cookie.is_expired())
            .map(|cookie| (String::from(&cookie.domain), String::from(cookie.path.clone()), cookie.name().to_owned()))
            .collect();
        for place in gone {
            self.store.remove(&place.0, &place.1, &place.2);
            self.created.remove(&place);
        }
        loop {
            let of_domain = self.created.keys().filter(|place| place.0 == domain).count();
            let oldest = if of_domain > MAX_COOKIES_PER_DOMAIN {
                self.created.iter().filter(|(place, _)| place.0 == domain).min_by_key(|(_, kept)| (kept.secure, kept.order))
            } else if self.created.len() > MAX_COOKIES_PER_JAR {
                self.created.iter().min_by_key(|(_, kept)| kept.order)
            } else {
                None
            };
            let Some(place) = oldest.map(|(place, _)| place.clone()) else { break };
            self.store.remove(&place.0, &place.1, &place.2);
            self.created.remove(&place);
        }
    }

    /// The `Cookie` header's value for a request to `url`, or none: the
    /// cookies that match it and that `SameSite` lets go with it, those of
    /// longer paths first, then the earlier kept (RFC 6265bis section
    /// 5.8.3).
    pub(crate) fn header(&self, url: &Url, context: Context) -> Option<String> {
        let mut cookies: Vec<&Cookie<'static>> = self
            .store
            .matches(url)
            .into_iter()
            .filter(|cookie| {
                context.same_site
                    || match cookie.same_site().unwrap_or(SameSite::Lax) {
                        SameSite::None => true,
                        SameSite::Lax => context.navigation && context.safe_method,
                        SameSite::Strict => false,
                    }
            })
            .collect();
        if cookies.is_empty() {
            return None;
        }
        let created = |cookie: &Cookie<'static>| {
            self.created.get(&(String::from(&cookie.domain), String::from(cookie.path.clone()), cookie.name().to_owned())).map_or(u64::MAX, |kept| kept.order)
        };
        cookies.sort_by_key(|cookie| (core::cmp::Reverse(cookie.path.as_ref().len()), created(cookie)));
        Some(cookies.iter().map(|cookie| format!("{}={}", cookie.name(), cookie.value())).collect::<Vec<_>>().join("; "))
    }
}

/// A written jar's fields: a cookie's origin after its length in two
/// bytes, its text after its length in four, its place in the order in
/// eight and its end in eight.
const ORDER_BYTES: usize = 8;
const EXPIRES_BYTES: usize = 8;

impl Jar {
    /// The jar written out for the trusted core to keep: the cookies that
    /// end with the session when `session`, else those that outlive it.
    /// Each is its origin, its text as a `Set-Cookie` value without a
    /// lifetime, its place in the order and when it ends.
    pub(crate) fn write(&self, session: bool) -> Vec<u8> {
        let mut places: Vec<(&Place, &Kept)> = self.created.iter().filter(|(_, kept)| kept.expires.is_none() == session).collect();
        places.sort_by_key(|(_, kept)| kept.order);
        let mut out = Vec::new();
        for (place, kept) in places {
            let Some(cookie) = self.store.get(&place.0, &place.1, &place.2) else { continue };
            let mut text = format!("{}={}; Path={}", cookie.name(), cookie.value(), place.1);
            if let CookieDomain::Suffix(domain) = &cookie.domain {
                text.push_str("; Domain=");
                text.push_str(domain);
            }
            if cookie.secure().unwrap_or(false) {
                text.push_str("; Secure");
            }
            if cookie.http_only().unwrap_or(false) {
                text.push_str("; HttpOnly");
            }
            text.push_str(match cookie.same_site().unwrap_or(SameSite::Lax) {
                SameSite::Strict => "; SameSite=Strict",
                SameSite::Lax => "; SameSite=Lax",
                SameSite::None => "; SameSite=None",
            });
            let (Ok(origin_length), Ok(text_length)) = (u16::try_from(kept.origin.len()), u32::try_from(text.len())) else { continue };
            out.extend_from_slice(&origin_length.to_be_bytes());
            out.extend_from_slice(kept.origin.as_bytes());
            out.extend_from_slice(&text_length.to_be_bytes());
            out.extend_from_slice(text.as_bytes());
            out.extend_from_slice(&kept.order.to_be_bytes());
            out.extend_from_slice(&kept.expires.unwrap_or(0).to_be_bytes());
        }
        out
    }

    /// Reads back a jar the trusted core kept. Every cookie goes through
    /// the rules a server's would, so bytes that were changed where they
    /// were kept can set nothing a server could not; what is malformed
    /// ends the reading, and what has ended since is not kept. Reading
    /// leaves the jar unchanged for [`Jar::take_changed`].
    pub(crate) fn read_written(&mut self, mut bytes: &[u8]) {
        /// The cookies as they were kept went with same-site requests.
        const KEPT: Context = Context { same_site: true, navigation: true, safe_method: true };
        if self.unkept {
            return;
        }
        let now = now_unix();
        while !bytes.is_empty() {
            let read = (|| {
                let origin_length = usize::from(u16::from_be_bytes(take(&mut bytes, 2)?.try_into().ok()?));
                let origin = Url::parse(std::str::from_utf8(take(&mut bytes, origin_length)?).ok()?).ok()?;
                let text_length = usize::try_from(u32::from_be_bytes(take(&mut bytes, 4)?.try_into().ok()?)).ok()?;
                let text = std::str::from_utf8(take(&mut bytes, text_length)?).ok()?;
                let order = u64::from_be_bytes(take(&mut bytes, ORDER_BYTES)?.try_into().ok()?);
                let expires = i64::from_be_bytes(take(&mut bytes, EXPIRES_BYTES)?.try_into().ok()?);
                Some((origin, text, order, expires))
            })();
            let Some((origin, text, order, expires)) = read else { break };
            if !matches!(origin.scheme(), "http" | "https") {
                continue;
            }
            let header = match expires {
                0 => text.to_owned(),
                at if at > now => format!("{text}; Max-Age={}", at.saturating_sub(now)),
                _ => continue,
            };
            let before = self.next;
            if self.store(&origin, header.as_bytes(), KEPT) && self.next != before {
                // The cookie just kept takes the place in the order it had.
                if let Some(kept) = self.created.values_mut().find(|kept| kept.order == before) {
                    kept.order = order;
                }
                self.next = before.max(order.saturating_add(1));
            }
        }
        self.changed = false;
    }
}

#[cfg(test)]
mod tests;

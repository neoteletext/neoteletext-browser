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

use cookie::time::{Duration, OffsetDateTime};
use cookie::{Cookie as RawCookie, SameSite};
use cookie_store::{Cookie, CookieDomain, CookieStore};
use publicsuffix::{List, Psl};
use url::Url;

use super::limits::{MAX_COOKIE_ATTRIBUTE, MAX_COOKIE_BYTES, MAX_COOKIE_LIFETIME_SECONDS, MAX_COOKIES_PER_DOMAIN, MAX_COOKIES_PER_JAR};

/// The public suffix list, when the browser has one. Without it a cookie
/// is kept for the host that set it alone, whatever its `Domain`
/// attribute, and a site is a host: narrower than the standard, never
/// wider.
pub(crate) type Suffixes = Option<Rc<List>>;

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
}

impl Jar {
    pub(crate) fn new(suffixes: Suffixes) -> Self {
        Self { store: CookieStore::new(), created: HashMap::new(), next: 0, suffixes, changed: false }
    }

    /// The public suffix list the jar keeps to.
    pub(crate) const fn suffixes(&self) -> &Suffixes {
        &self.suffixes
    }

    /// How many cookies the jar keeps.
    pub(crate) fn count(&self) -> usize {
        self.created.len()
    }

    /// Whether the jar changed since this was last asked.
    pub(crate) fn take_changed(&mut self) -> bool {
        core::mem::take(&mut self.changed)
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
        let mut raw = RawCookie::parse(without_long_attributes(text)).ok()?.into_owned();
        // No cookie outlives the cap, whichever attribute asked.
        let cap = Duration::seconds(MAX_COOKIE_LIFETIME_SECONDS);
        let asked = raw.max_age().or_else(|| raw.expires_datetime().map(lifetime_until));
        if asked.is_some_and(|asked| asked > cap) {
            raw.set_max_age(cap);
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
        let lifetime = cookie.max_age().or_else(|| cookie.expires_datetime().map(lifetime_until));
        let ends_at = lifetime.map(|lifetime| now_unix().saturating_add(lifetime.whole_seconds()));
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
            (kept.expires, kept.origin) = (ends_at, origin);
        } else {
            self.created.insert(place.clone(), Kept { order: self.next, expires: ends_at, origin });
            self.next = self.next.saturating_add(1);
        }
        self.evict(&place.0);
        true
    }

    /// Removes what has expired, then the oldest cookies of `domain` past
    /// [`MAX_COOKIES_PER_DOMAIN`] and of the jar past
    /// [`MAX_COOKIES_PER_JAR`].
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
                self.created.iter().filter(|(place, _)| place.0 == domain).min_by_key(|(_, kept)| kept.order)
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

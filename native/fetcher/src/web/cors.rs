//! The CORS protocol of the Fetch standard (its section 3.2): which
//! requests need a preflight, the check an answer from another origin must
//! pass, what a preflight's answer allows, the preflight cache, and the
//! headers of the answer a script may read.

use std::collections::HashMap;
use std::time::Instant;

use protocol::web::Header;

use super::limits::{DEFAULT_PREFLIGHT_AGE, MAX_PREFLIGHT_AGE, PREFLIGHT_CACHE_ENTRIES};
use super::rules::is_forbidden_response_header;

/// The longest value of a CORS-safelisted request header, in bytes (the
/// Fetch standard, "CORS-safelisted request-header").
const MAX_SAFELISTED_VALUE: usize = 128;
/// The most bytes of safelisted request headers together before none is
/// safelisted (the Fetch standard, "CORS-unsafe request-header names").
const MAX_SAFELISTED_TOTAL: usize = 1024;

/// Whether `method` needs no preflight (the Fetch standard,
/// "CORS-safelisted method").
pub(crate) fn is_safelisted_method(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "POST")
}

/// The bytes a safelisted `Accept`, `Accept-Language` or
/// `Content-Language` may not hold (the Fetch standard, "CORS-unsafe
/// request-header byte").
const fn is_unsafe_byte(byte: u8) -> bool {
    (byte < 0x20 && byte != b'\t') || matches!(byte, b'"' | b'(' | b')' | b':' | b'<' | b'>' | b'?' | b'@' | b'[' | b'\\' | b']' | b'{' | b'}' | 0x7F)
}

/// Whether a `Range` value is the simple range the standard safelists:
/// `bytes=` then digits, `-`, and digits or nothing.
fn is_simple_range(value: &[u8]) -> bool {
    let Some(range) = value.strip_prefix(b"bytes=") else { return false };
    let mut parts = range.splitn(2, |&byte| byte == b'-');
    let (Some(first), Some(last)) = (parts.next(), parts.next()) else { return false };
    !first.is_empty() && first.iter().all(u8::is_ascii_digit) && last.iter().all(u8::is_ascii_digit)
}

/// Whether the request header `name` (in lower case) with `value` needs no
/// preflight (the Fetch standard, "CORS-safelisted request-header").
pub(crate) fn is_safelisted_request_header(name: &str, value: &[u8]) -> bool {
    if value.len() > MAX_SAFELISTED_VALUE {
        return false;
    }
    match name {
        "accept" => !value.iter().copied().any(is_unsafe_byte),
        "accept-language" | "content-language" => {
            value.iter().all(|&byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'*' | b',' | b'-' | b'.' | b';' | b'='))
        }
        "content-type" => {
            if value.iter().copied().any(is_unsafe_byte) {
                return false;
            }
            let essence = value.split(|&byte| byte == b';').next().unwrap_or_default().trim_ascii();
            [&b"application/x-www-form-urlencoded"[..], b"multipart/form-data", b"text/plain"].iter().any(|allowed| essence.eq_ignore_ascii_case(allowed))
        }
        "range" => is_simple_range(value),
        _ => false,
    }
}

/// The names of the headers of `headers` that need a preflight, in lower
/// case, sorted, each once (the Fetch standard, "CORS-unsafe
/// request-header names").
pub(crate) fn unsafe_header_names(headers: &[Header]) -> Vec<String> {
    let mut names = Vec::new();
    let mut safe = Vec::new();
    let mut safe_bytes = 0_usize;
    for (name, value) in headers {
        if is_safelisted_request_header(name, value) {
            safe_bytes = safe_bytes.saturating_add(value.len());
            safe.push(name.clone());
        } else {
            names.push(name.clone());
        }
    }
    if safe_bytes > MAX_SAFELISTED_TOTAL {
        names.append(&mut safe);
    }
    names.sort();
    names.dedup();
    names
}

/// The one value of the header `name` in `headers`, if it is there once.
fn single<'a>(headers: &'a [Header], name: &str) -> Option<&'a [u8]> {
    let mut values = headers.iter().filter(|(header, _)| header == name).map(|(_, value)| value.as_slice());
    let first = values.next()?;
    values.next().is_none().then_some(first)
}

/// Every value of the header `name` in `headers`, split at commas and
/// trimmed, in lower case; `None` when a part is no token (the Fetch
/// standard extracts these lists by their ABNF, and a failure fails the
/// preflight).
fn token_list(headers: &[Header], name: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    for (_, value) in headers.iter().filter(|(header, _)| header == name) {
        for part in value.split(|&byte| byte == b',') {
            let part = part.trim_ascii();
            if part.is_empty() {
                continue;
            }
            if !part.iter().copied().all(protocol::web::is_token_byte) {
                return None;
            }
            tokens.push(String::from_utf8_lossy(part).to_ascii_lowercase());
        }
    }
    Some(tokens)
}

/// The Fetch standard's CORS check (its section 4.9): whether an answer
/// with `headers` (names in lower case) allows a request of the origin
/// written `origin`, with credentials or without.
pub(crate) fn cors_check(headers: &[Header], origin: &str, with_credentials: bool) -> bool {
    let Some(allowed) = single(headers, "access-control-allow-origin") else { return false };
    if !with_credentials && allowed == b"*" {
        return true;
    }
    if allowed != origin.as_bytes() {
        return false;
    }
    !with_credentials || single(headers, "access-control-allow-credentials") == Some(b"true")
}

/// What a preflight's answer allows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Allowed {
    /// The methods, in upper case as sent, or `*`.
    methods: Vec<String>,
    /// The header names, in lower case, or `*`.
    headers: Vec<String>,
    /// Until when it may be remembered.
    until: Instant,
    /// Whether it was given to a request with credentials.
    with_credentials: bool,
}

impl Allowed {
    /// What the preflight answer `headers` allows, read at `now`; `None`
    /// when a list is malformed.
    pub(crate) fn read(headers: &[Header], with_credentials: bool, now: Instant) -> Option<Self> {
        // Methods are case-sensitive: the list is kept as sent.
        let mut methods = Vec::new();
        for (_, value) in headers.iter().filter(|(header, _)| header == "access-control-allow-methods") {
            for part in value.split(|&byte| byte == b',').map(<[u8]>::trim_ascii).filter(|part| !part.is_empty()) {
                if !part.iter().copied().all(protocol::web::is_token_byte) {
                    return None;
                }
                methods.push(String::from_utf8_lossy(part).into_owned());
            }
        }
        let allowed_headers = token_list(headers, "access-control-allow-headers")?;
        let age = single(headers, "access-control-max-age")
            .and_then(|value| std::str::from_utf8(value).ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map_or(DEFAULT_PREFLIGHT_AGE, |seconds| std::time::Duration::from_secs(seconds).min(MAX_PREFLIGHT_AGE));
        Some(Self { methods, headers: allowed_headers, until: now.checked_add(age).unwrap_or(now), with_credentials })
    }

    /// Whether a request with `method` and the unsafe header names
    /// `names` is allowed (the Fetch standard's CORS-preflight fetch,
    /// steps 7.6 to 7.8).
    pub(crate) fn allows(&self, method: &str, names: &[String]) -> bool {
        let wildcard = |list: &[String]| !self.with_credentials && list.iter().any(|entry| entry == "*");
        let method_allowed = is_safelisted_method(method) || self.methods.iter().any(|allowed| allowed == method) || wildcard(&self.methods);
        // `Authorization` is never covered by `*` (the standard's "CORS
        // non-wildcard request-header name").
        let header_allowed = |name: &String| self.headers.contains(name) || (name != "authorization" && wildcard(&self.headers));
        method_allowed && names.iter().all(header_allowed)
    }
}

/// What a remembered preflight answer was for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Asked {
    /// The origin asking, as written.
    origin: String,
    /// The address asked, without its fragment.
    url: String,
    /// Whether with credentials.
    with_credentials: bool,
}

/// The CORS-preflight cache (the Fetch standard, section 4.8), bounded.
/// It is the process's own, and a process serves one partition (the
/// browser's DEC-7268), so one channel's preflights say nothing to
/// another's (neoteletext-common's DEC-7264 item 4).
#[derive(Default)]
pub(crate) struct PreflightCache {
    entries: HashMap<Asked, Allowed>,
}

impl PreflightCache {
    fn asked(origin: &str, url: &url::Url, with_credentials: bool) -> Asked {
        let mut url = url.clone();
        url.set_fragment(None);
        Asked { origin: origin.to_owned(), url: url.into(), with_credentials }
    }

    /// Whether a remembered answer still allows the request.
    pub(crate) fn allows(&mut self, origin: &str, url: &url::Url, with_credentials: bool, method: &str, names: &[String], now: Instant) -> bool {
        let asked = Self::asked(origin, url, with_credentials);
        match self.entries.get(&asked) {
            Some(allowed) if allowed.until > now => allowed.allows(method, names),
            Some(_) => {
                self.entries.remove(&asked);
                false
            }
            None => false,
        }
    }

    /// Remembers a preflight's answer.
    pub(crate) fn remember(&mut self, origin: &str, url: &url::Url, with_credentials: bool, allowed: Allowed, now: Instant) {
        self.entries.retain(|_, entry| entry.until > now);
        if self.entries.len() >= PREFLIGHT_CACHE_ENTRIES {
            // The one that ends soonest makes room.
            if let Some(soonest) = self.entries.iter().min_by_key(|(_, entry)| entry.until).map(|(asked, _)| asked.clone()) {
                self.entries.remove(&soonest);
            }
        }
        self.entries.insert(Self::asked(origin, url, with_credentials), allowed);
    }
}

/// The headers a script may always read of an answer from another origin
/// (the Fetch standard, "CORS-safelisted response-header name").
const SAFELISTED_RESPONSE_HEADERS: [&str; 7] = ["cache-control", "content-language", "content-length", "content-type", "expires", "last-modified", "pragma"];

/// The headers of an answer from another origin that a script may read:
/// the safelisted ones and those `Access-Control-Expose-Headers` names,
/// every one when it says `*` to a request without credentials.
pub(crate) fn exposed(headers: &[Header], with_credentials: bool) -> Vec<Header> {
    let named = token_list(headers, "access-control-expose-headers").unwrap_or_default();
    let every = !with_credentials && named.iter().any(|name| name == "*");
    headers
        .iter()
        .filter(|(name, _)| !is_forbidden_response_header(name) && (every || SAFELISTED_RESPONSE_HEADERS.contains(&name.as_str()) || named.contains(name)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<Header> {
        pairs.iter().map(|(name, value)| ((*name).to_owned(), value.as_bytes().to_vec())).collect()
    }

    #[test]
    fn simple_requests_need_no_preflight() {
        assert!(is_safelisted_method("GET") && is_safelisted_method("POST") && !is_safelisted_method("PUT") && !is_safelisted_method("get"));
        for (name, value) in [
            ("accept", "text/html, */*;q=0.8"),
            ("accept-language", "en-AU, en;q=0.9"),
            ("content-type", "text/plain;charset=UTF-8"),
            ("content-type", "Multipart/Form-Data; boundary=x"),
            ("range", "bytes=0-99"),
            ("range", "bytes=100-"),
        ] {
            assert!(is_safelisted_request_header(name, value.as_bytes()), "{name}: {value}");
        }
        for (name, value) in [
            ("content-type", "application/json"),
            ("content-type", "text/plain; x=\"y\""),
            ("accept", "a(b)"),
            ("accept-language", "en@AU"),
            ("range", "bytes=-5"),
            ("range", "bytes=0-1,5-6"),
            ("authorization", "Bearer x"),
            ("x-custom", "1"),
        ] {
            assert!(!is_safelisted_request_header(name, value.as_bytes()), "{name}: {value}");
        }
        assert!(!is_safelisted_request_header("accept", &[b'a'; MAX_SAFELISTED_VALUE + 1]));
        assert_eq!(
            unsafe_header_names(&headers(&[("x-b", "1"), ("accept", "*/*"), ("authorization", "x"), ("x-b", "2"), ("content-type", "application/json")])),
            ["authorization", "content-type", "x-b"]
        );
        // Safelisted headers of more than 1,024 bytes together are all unsafe.
        let long = "a".repeat(MAX_SAFELISTED_VALUE);
        let many: Vec<(&str, &str)> = (0..9).map(|_| ("accept", long.as_str())).collect();
        assert_eq!(unsafe_header_names(&headers(&many)), ["accept"]);
    }

    #[test]
    fn the_cors_check_holds_an_answer_to_the_origin_and_its_credentials() {
        let origin = "https://a.example";
        assert!(cors_check(&headers(&[("access-control-allow-origin", "*")]), origin, false));
        assert!(cors_check(&headers(&[("access-control-allow-origin", origin)]), origin, false));
        assert!(cors_check(&headers(&[("access-control-allow-origin", origin), ("access-control-allow-credentials", "true")]), origin, true));
        assert!(!cors_check(&headers(&[]), origin, false), "no header");
        assert!(!cors_check(&headers(&[("access-control-allow-origin", "https://b.example")]), origin, false), "another origin");
        assert!(!cors_check(&headers(&[("access-control-allow-origin", "https://A.example")]), origin, false), "byte for byte");
        assert!(!cors_check(&headers(&[("access-control-allow-origin", "*")]), origin, true), "a wildcard with credentials");
        assert!(!cors_check(&headers(&[("access-control-allow-origin", origin)]), origin, true), "credentials not allowed");
        assert!(!cors_check(&headers(&[("access-control-allow-origin", origin), ("access-control-allow-credentials", "True")]), origin, true));
        assert!(!cors_check(&headers(&[("access-control-allow-origin", origin), ("access-control-allow-origin", origin)]), origin, false), "two headers");
        assert!(!cors_check(&headers(&[("access-control-allow-origin", "null")]), origin, false));
        assert!(cors_check(&headers(&[("access-control-allow-origin", "null")]), "null", false), "an opaque origin is written null");
    }

    #[test]
    fn a_preflight_answer_allows_what_it_names() {
        let now = Instant::now();
        let read = |pairs: &[(&str, &str)], with_credentials| Allowed::read(&headers(pairs), with_credentials, now).unwrap();
        let named = read(&[("access-control-allow-methods", "PUT, DELETE"), ("access-control-allow-headers", "X-Custom, Content-Type")], false);
        assert!(named.allows("PUT", &["content-type".into(), "x-custom".into()]));
        assert!(named.allows("GET", &[]), "a safelisted method needs no naming");
        assert!(!named.allows("PATCH", &[]));
        assert!(!named.allows("put", &[]), "methods are case-sensitive");
        assert!(!named.allows("PUT", &["x-other".into()]));
        let every = read(&[("access-control-allow-methods", "*"), ("access-control-allow-headers", "*")], false);
        assert!(every.allows("PATCH", &["x-any".into()]));
        assert!(!every.allows("PATCH", &["authorization".into()]), "the wildcard never covers Authorization");
        let credentialed = read(&[("access-control-allow-methods", "*"), ("access-control-allow-headers", "*")], true);
        assert!(!credentialed.allows("PATCH", &[]) && !credentialed.allows("GET", &["x-any".into()]), "no wildcard with credentials");
        assert!(Allowed::read(&headers(&[("access-control-allow-headers", "x y")]), false, now).is_none(), "a malformed list");
        assert_eq!(read(&[], false).until, now + DEFAULT_PREFLIGHT_AGE);
        assert_eq!(read(&[("access-control-max-age", "600")], false).until, now + std::time::Duration::from_secs(600));
        assert_eq!(read(&[("access-control-max-age", "999999999")], false).until, now + MAX_PREFLIGHT_AGE);
    }

    /// The preflight cache remembers an answer for its origin, address and
    /// credentials alone, until its age passes.
    #[test]
    fn the_preflight_cache_remembers_until_its_age_passes() {
        let now = Instant::now();
        let url = url::Url::parse("https://b.example/api#x").unwrap();
        let allowed = Allowed::read(&headers(&[("access-control-allow-methods", "PUT"), ("access-control-max-age", "60")]), false, now).unwrap();
        let mut cache = PreflightCache::default();
        cache.remember("https://a.example", &url, false, allowed, now);
        assert!(cache.allows("https://a.example", &url::Url::parse("https://b.example/api").unwrap(), false, "PUT", &[], now));
        assert!(!cache.allows("https://a.example", &url, false, "DELETE", &[], now));
        assert!(!cache.allows("https://c.example", &url, false, "PUT", &[], now), "another origin");
        assert!(!cache.allows("https://a.example", &url, true, "PUT", &[], now), "with credentials");
        assert!(!cache.allows("https://a.example", &url::Url::parse("https://b.example/other").unwrap(), false, "PUT", &[], now));
        let later = now + std::time::Duration::from_secs(61);
        assert!(!cache.allows("https://a.example", &url, false, "PUT", &[], later), "its age passed");
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn only_exposed_headers_are_read_across_origins() {
        let answer = headers(&[
            ("content-type", "text/plain"),
            ("x-secret", "1"),
            ("x-shown", "2"),
            ("set-cookie", "a=b"),
            ("access-control-expose-headers", "X-Shown, Set-Cookie"),
        ]);
        assert_eq!(exposed(&answer, false), headers(&[("content-type", "text/plain"), ("x-shown", "2")]));
        let every = headers(&[("x-secret", "1"), ("set-cookie", "a=b"), ("access-control-expose-headers", "*")]);
        assert_eq!(exposed(&every, false), headers(&[("x-secret", "1"), ("access-control-expose-headers", "*")]));
        assert_eq!(exposed(&every, true), headers(&[]), "no wildcard with credentials");
    }
}

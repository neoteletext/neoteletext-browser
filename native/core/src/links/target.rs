//! Link targets (`hyperlinks.md` section 7, requirement 17.12): the parser
//! of the TTI Link Extension's forms and `web:`, the one page data the
//! trusted core reads.

use interface_core::channel::page_index;
use protocol::MAX_ADDRESS;
use protocol::links::{MAX_SERVICE_NAME, MAX_TARGET};

use crate::address::{self, Address};

/// A web address handed to the system's web browser: the address bar's
/// limit.
const MAX_WEB: usize = MAX_ADDRESS;
/// The digits of a page number and of a sub-page number in a target
/// (TTI Link Extension section E.1).
const PAGE_DIGITS: usize = 3;
const SUBPAGE_DIGITS: usize = 4;
const HEXADECIMAL: u32 = 16;
/// The two schemes a link may name, and the mark of a web page.
pub(super) const HTTPS: &str = "https";
const HTTP: &str = "http";
const SCHEME_SEPARATOR: &str = "://";
const WEB: &str = "web:";

/// A link target as a page or a descriptor writes it, parsed: the forms of
/// the TTI Link Extension's section E.1 and `web:` (the engine's decision
/// E.88).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A page of this channel.
    Page(u16),
    /// A page of a service the channel's descriptor names.
    Service {
        /// The service's short name.
        name: String,
        /// The page.
        page: u16,
    },
    /// A teletext channel or page file at an address.
    Channel(Address),
    /// A web page: its address, rebuilt from its parsed parts.
    Web(String),
}

/// Parses a link target. Everything that is not exactly one of the forms
/// is refused: a target is page data, the only page data the trusted core
/// reads (requirement 17.12).
#[must_use]
pub fn parse_target(text: &str) -> Option<Target> {
    // Printable ASCII with no spaces or commas, at most 256 bytes (TTI
    // Link Extension section E.1).
    if text.is_empty() || text.len() > MAX_TARGET || !text.bytes().all(|byte| byte.is_ascii_graphic() && byte != b',') {
        return None;
    }
    if let Some(web) = text.strip_prefix(WEB) {
        return web_address(web).map(Target::Web);
    }
    if text.contains(SCHEME_SEPARATOR) {
        return channel_address(text).map(Target::Channel);
    }
    let mut parts = text.split(':');
    let (first, second, third) = (parts.next()?, parts.next(), parts.next());
    if parts.next().is_some() {
        return None;
    }
    match (second, third) {
        (None, _) => page(first).map(Target::Page),
        // `mpp:ssss`, or `name:mpp`: a page number is three hexadecimal
        // digits and a name of three such letters is read as a page.
        (Some(second), None) => match (page(first), subpage(second)) {
            (Some(page), Some(())) => Some(Target::Page(page)),
            _ => Some(Target::Service { name: name(first)?, page: page(second)? }),
        },
        (Some(second), Some(third)) => {
            subpage(third)?;
            Some(Target::Service { name: name(first)?, page: page(second)? })
        }
    }
}

/// A display page: three hexadecimal digits naming a page of 100 to 899
/// with no digit above 9.
pub(super) fn page(text: &str) -> Option<u16> {
    if text.len() != PAGE_DIGITS || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let page = u16::from_str_radix(text, HEXADECIMAL).ok()?;
    page_index(page).map(|_| page)
}

/// A sub-page: four hexadecimal digits. Links go to a page; its sub-page
/// is shown as the page shows it.
fn subpage(text: &str) -> Option<()> {
    (text.len() == SUBPAGE_DIGITS && text.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(())
}

/// A service's short name: letters, digits and hyphens (TTI Link Extension
/// section E.1).
pub(super) fn name(text: &str) -> Option<String> {
    let valid = !text.is_empty() && text.len() <= MAX_SERVICE_NAME && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    valid.then(|| text.to_owned())
}

/// The scheme of `text`, when it is `https` or `http` written in any case.
fn web_scheme(text: &str) -> Option<(&'static str, &str)> {
    let (scheme, rest) = text.split_once(SCHEME_SEPARATOR)?;
    [HTTPS, HTTP].into_iter().find(|known| scheme.eq_ignore_ascii_case(known)).map(|known| (known, rest))
}

/// A teletext channel's address: `https` or `http` only, by the address
/// rules the address field uses. A link never leads to a file.
pub(super) fn channel_address(text: &str) -> Option<Address> {
    web_scheme(text)?;
    let address = address::parse(text).ok()?;
    address.file.is_none().then_some(address)
}

/// Characters a web address may hold after its host, as RFC 3986 section 2
/// allows them: unreserved, reserved and the percent sign of an escape.
fn uri_character(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&byte)
}

/// A web address, rebuilt from its parsed parts so that what is handed on
/// is exactly what was checked (`hyperlinks.md` section 12): `https` or
/// `http`, a host of letters, digits, hyphens and dots, a numeric port,
/// then only the characters RFC 3986 allows, every `%` starting an escape.
/// Anything else is refused, never repaired.
#[must_use]
pub fn web_address(text: &str) -> Option<String> {
    if text.len() > MAX_WEB {
        return None;
    }
    let (scheme, rest) = web_scheme(text)?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, Some(port.parse::<u16>().ok()?)),
        None => (authority, None),
    };
    let label = |label: &str| !label.is_empty() && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if host.is_empty() || !host.split('.').all(label) {
        return None;
    }
    let bytes = tail.as_bytes();
    let escapes_whole = bytes
        .iter()
        .enumerate()
        .filter(|&(_, &byte)| byte == b'%')
        .all(|(at, _)| bytes.get(at.saturating_add(1)..at.saturating_add(3)).is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit)));
    if !bytes.iter().copied().all(uri_character) || !escapes_whole {
        return None;
    }
    let host = host.to_ascii_lowercase();
    let port = port.map(|port| format!(":{port}")).unwrap_or_default();
    let tail = if tail.is_empty() || tail.starts_with(['?', '#']) { format!("/{tail}") } else { tail.to_owned() };
    let rebuilt = format!("{scheme}{SCHEME_SEPARATOR}{host}{port}{tail}");
    (rebuilt.len() <= MAX_WEB).then_some(rebuilt)
}

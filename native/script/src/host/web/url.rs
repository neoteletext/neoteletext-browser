//! Operations `URL_PARSE` (40) and `URL_SET` (48): the URL standard's
//! parser and its API's setters, by the `url` crate, which the web client
//! parses every address with too, so that a script and the network never
//! read one address two ways (DEC-7260 item 5; script-api.md section 9.1).
//!
//! The prelude's `URL` object holds what this gives it and nothing more:
//! the serialised address and where each component lies in it.

use core::fmt::Write as _;

use url::{Position, Url, quirks};

use super::super::Reply;
use crate::limits::MAX_URL_BYTES;

/// The components operation `URL_SET` sets, by the URL standard's API
/// (section 6.1, "URL class"): each is the setter of that name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Part {
    Protocol,
    Username,
    Password,
    Host,
    Hostname,
    Port,
    Pathname,
    Search,
    Hash,
}

impl Part {
    /// Every part, with the number the prelude names it by.
    pub(super) const ALL: [(&'static str, Self, i32); 9] = [
        ("PROTOCOL", Self::Protocol, 1),
        ("USERNAME", Self::Username, 2),
        ("PASSWORD", Self::Password, 3),
        ("HOST", Self::Host, 4),
        ("HOSTNAME", Self::Hostname, 5),
        ("PORT", Self::Port, 6),
        ("PATHNAME", Self::Pathname, 7),
        ("SEARCH", Self::Search, 8),
        ("HASH", Self::Hash, 9),
    ];

    fn numbered(number: i32) -> Option<Self> {
        Self::ALL.iter().find(|(_, _, own)| *own == number).map(|(_, part, _)| *part)
    }
}

/// The places in a serialised address the prelude cuts it at, in the
/// order they are written: the end of the scheme, then the start and end
/// of the user name, the password, the host, the port, the path, the
/// query and the fragment.
const PLACES: [Position; 13] = [
    Position::AfterScheme,
    Position::BeforeUsername,
    Position::AfterUsername,
    Position::BeforePassword,
    Position::AfterPassword,
    Position::BeforeHost,
    Position::AfterHost,
    Position::BeforePort,
    Position::AfterPort,
    Position::BeforePath,
    Position::AfterPath,
    Position::BeforeQuery,
    Position::AfterQuery,
];

/// The number of places written, for the prelude.
pub(super) const PLACE_COUNT: usize = PLACES.len();

/// An address as the prelude is given it: the places, a line feed, the
/// origin's serialisation (URL standard section 4.7, `null` for an opaque
/// one), a line feed, and the serialised address, which is ASCII and
/// holds no line feed.
fn written(url: &Url) -> Reply {
    let text = url.as_str();
    if text.len() > MAX_URL_BYTES {
        return too_long();
    }
    let mut out = String::with_capacity(text.len().saturating_add(128));
    for place in PLACES {
        // The library's own cut of an address at its password is wrong
        // when it has a user name and no password, as
        // `https://user@example.test/` has: it gives the password as the
        // "@" after the name, and where its debug assertions are compiled
        // in (url 2.5.9, `slicing.rs` line 148) it panics, on an address
        // any script may write (rdm TODO-7386). Without a password both
        // its places are the end of the user name, which is asked for
        // instead and is right in every case.
        let place =
            if matches!(place, Position::BeforePassword | Position::AfterPassword) && url.password().is_none() { Position::AfterUsername } else { place };
        let _ = write!(out, "{},", url[..place].len());
    }
    // The fragment runs from its place to the end.
    let _ = writeln!(out, "{}", url[..Position::BeforeFragment].len());
    out.push_str(&url.origin().ascii_serialization());
    out.push('\n');
    out.push_str(text);
    Reply::Text(out)
}

/// The refusal of an address past its bound. A `TypeError`, as the URL
/// standard's constructor gives for an address it does not take, with
/// the bound's name first as every bound's refusal has (departure D8).
fn too_long() -> Reply {
    Reply::Type("MAX_URL_BYTES: the address is too long".to_owned())
}

/// Cuts `payload` after its first `first` bytes, when that is a place
/// between two characters.
fn cut(payload: &str, first: i32) -> Option<(&str, &str)> {
    payload.split_at_checked(usize::try_from(first).ok()?)
}

/// Operation `URL_PARSE`: `payload` is the input, followed by the base
/// when `input_bytes` is not -1 and is then the input's length in bytes.
/// The address as [`written`] gives it, or undefined when the standard's
/// parser fails.
pub(super) fn parse(input_bytes: i32, payload: &str) -> Reply {
    let (input, base) = if input_bytes == -1 {
        (payload, None)
    } else {
        let Some((input, base)) = cut(payload, input_bytes) else { return Reply::Type("the input's length is not a place in what was given".to_owned()) };
        (input, Some(base))
    };
    if input.len() > MAX_URL_BYTES || base.is_some_and(|base| base.len() > MAX_URL_BYTES) {
        return too_long();
    }
    let base = match base.map(Url::parse) {
        None => None,
        Some(Ok(base)) => Some(base),
        Some(Err(_)) => return Reply::Undefined,
    };
    Url::options().base_url(base.as_ref()).parse(input).map_or(Reply::Undefined, |url| written(&url))
}

/// Operation `URL_SET`: `payload` is a serialised address of
/// `address_bytes` bytes, followed by the value the setter numbered
/// `part` is given. The address after the setter ran, which by the
/// standard is unchanged where the value is not one the component takes.
pub(super) fn set(part: i32, address_bytes: i32, payload: &str) -> Reply {
    let Some(part) = Part::numbered(part) else { return Reply::Type("no such component of an address".to_owned()) };
    let Some((address, value)) = cut(payload, address_bytes) else { return Reply::Type("a setter takes an address and a value".to_owned()) };
    if address.len() > MAX_URL_BYTES || value.len() > MAX_URL_BYTES {
        return too_long();
    }
    let Ok(mut url) = Url::parse(address) else { return Reply::Type("a setter takes a serialised address".to_owned()) };
    let before = url.clone();
    // Each setter leaves the address as it was when it refuses the value,
    // which is what its result says and all it says.
    match part {
        Part::Protocol => drop(quirks::set_protocol(&mut url, value)),
        Part::Username => drop(quirks::set_username(&mut url, value)),
        Part::Password => drop(quirks::set_password(&mut url, value)),
        Part::Host => drop(quirks::set_host(&mut url, value)),
        Part::Hostname => drop(quirks::set_hostname(&mut url, value)),
        Part::Port => drop(quirks::set_port(&mut url, value)),
        Part::Pathname => quirks::set_pathname(&mut url, value),
        Part::Search => quirks::set_search(&mut url, value),
        Part::Hash => quirks::set_hash(&mut url, value),
    }
    // A value that would take the address past its bound changes nothing.
    if url.as_str().len() > MAX_URL_BYTES { written(&before) } else { written(&url) }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::string_slice, clippy::panic, reason = "tests")]
mod tests {
    use super::*;

    fn text(reply: Reply) -> String {
        match reply {
            Reply::Text(text) => text,
            other => panic!("{other:?}"),
        }
    }

    /// URL standard section 4.4 and 4.7: an address is resolved against
    /// its base and serialised, its origin is its scheme, host and port,
    /// and the places cut it into its components.
    #[test]
    fn an_address_is_resolved_serialised_and_cut() {
        let reply = text(parse(10, "../b?q=1#fhttps://user:pw@Example.test:8443/a/c/d"));
        let mut lines = reply.splitn(3, '\n');
        let places: Vec<usize> = lines.next().unwrap().split(',').map(|place| place.parse().unwrap()).collect();
        assert_eq!(places.len(), PLACE_COUNT + 1);
        assert_eq!(lines.next().unwrap(), "https://example.test:8443");
        let href = lines.next().unwrap();
        assert_eq!(href, "https://user:pw@example.test:8443/a/b?q=1#f");
        let cut = |from: usize, to: usize| &href[places[from]..places[to]];
        assert_eq!(&href[..places[0]], "https");
        assert_eq!((cut(1, 2), cut(3, 4), cut(5, 6), cut(7, 8), cut(9, 10), cut(11, 12)), ("user", "pw", "example.test", "8443", "/a/b", "q=1"));
        assert_eq!(&href[places[13]..], "f");
    }

    /// URL standard section 4.1: an address with a user name and no
    /// password has an empty password. The library's own cut gave "@" for
    /// it and panicked where its debug assertions are compiled in (rdm
    /// TODO-7386), so this test fails in such a build without the guard
    /// in `written`, and in every build by the password it finds.
    #[test]
    fn a_user_name_without_a_password_is_cut_with_an_empty_password() {
        let reply = text(parse(-1, "https://user@example.test/a"));
        let mut lines = reply.splitn(3, '\n');
        let places: Vec<usize> = lines.next().unwrap().split(',').map(|place| place.parse().unwrap()).collect();
        let href = lines.nth(1).unwrap();
        assert_eq!(href, "https://user@example.test/a");
        let cut = |from: usize, to: usize| &href[places[from]..places[to]];
        assert_eq!((cut(1, 2), cut(3, 4), cut(5, 6)), ("user", "", "example.test"));
    }

    /// URL standard section 4.4: what the parser fails on is undefined to
    /// the prelude, which throws the constructor's `TypeError`; an input
    /// past the bound is refused by the bound's name.
    #[test]
    fn a_failure_is_undefined_and_a_long_address_names_its_bound() {
        assert_eq!(parse(-1, "/relative"), Reply::Undefined);
        assert_eq!(parse(1, "xnot a base"), Reply::Undefined);
        assert_eq!(parse(-1, "https://exa mple.test/"), Reply::Undefined);
        let long = format!("https://example.test/{}", "a".repeat(MAX_URL_BYTES));
        assert!(matches!(parse(-1, &long), Reply::Type(text) if text.starts_with("MAX_URL_BYTES")));
    }

    /// URL standard section 6.1: the setters, and a refused value leaving
    /// the address as it was.
    #[test]
    fn the_setters_are_the_standards() {
        let href = |reply: Reply| text(reply).splitn(3, '\n').nth(2).unwrap().to_owned();
        assert_eq!(href(set(6, 22, "https://example.test/a8080")), "https://example.test:8080/a");
        assert_eq!(href(set(6, 22, "https://example.test/anonsense")), "https://example.test/a");
        assert_eq!(href(set(7, 22, "https://example.test/a/b c")), "https://example.test/b%20c");
        assert_eq!(href(set(8, 22, "https://example.test/ax=1")), "https://example.test/a?x=1");
        assert_eq!(href(set(9, 22, "https://example.test/atop")), "https://example.test/a#top");
        assert_eq!(href(set(1, 22, "https://example.test/ahttp")), "http://example.test/a");
        assert_eq!(href(set(5, 22, "https://example.test/aother.test")), "https://other.test/a");
        assert!(matches!(set(0, 21, "https://example.test/x"), Reply::Type(_)));
        assert!(matches!(set(7, 99, "no separator"), Reply::Type(_)));
        let long = "a".repeat(MAX_URL_BYTES - 10);
        assert_eq!(href(set(7, 22, &format!("https://example.test/a{long}"))), "https://example.test/a");
    }
}

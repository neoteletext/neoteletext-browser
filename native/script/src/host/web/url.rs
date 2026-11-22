//! Operations `URL_PARSE` (40) and `URL_SET` (48): the URL standard's
//! parser and its API's setters, by the `url` crate, which the web client
//! parses every address with too, so that a script and the network never
//! read one address two ways (DEC-7260 item 5; script-api.md section 9.1).
//!
//! The prelude's `URL` object holds what this gives it and nothing more:
//! the serialised address and each of its components.

use url::{Url, quirks};

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

/// The fields an address is written as for the prelude, a line each.
pub(super) const FIELDS: usize = 11;

/// An address as the prelude is given it, a line for each field: the
/// serialised address, the origin's serialisation (URL standard section
/// 4.7, `null` for an opaque one), then what the URL standard's API
/// gives for `protocol`, `username`, `password`, `host`, `hostname`,
/// `port`, `pathname`, `search` and `hash` (section 6.1). Every field is
/// ASCII and holds no line feed.
///
/// The fields are the crate's own getters'. The crate's slicing by
/// position is not used: after some setters it indexes past the
/// address's end (found by the web-platform-tests' setter cases on 22
/// November 2026, version 2.5.9), and nothing a script gives may make
/// its host panic.
fn written(url: &Url) -> Reply {
    let text = url.as_str();
    if text.len() > MAX_URL_BYTES {
        return too_long();
    }
    let origin = url.origin().ascii_serialization();
    let fields: [&str; FIELDS] = [
        text,
        &origin,
        quirks::protocol(url),
        quirks::username(url),
        quirks::password(url),
        quirks::host(url),
        quirks::hostname(url),
        quirks::port(url),
        quirks::pathname(url),
        quirks::search(url),
        quirks::hash(url),
    ];
    Reply::Text(fields.join("\n"))
}

/// Runs an address operation so that a panic inside the library that
/// parses addresses is a refusal and not the host's end: the library is
/// not this crate's, and a script chooses its every input.
///
/// This is the second layer and nothing rests on it (rdm TODO-7390).
/// The first is by construction: no address is sliced by position, which
/// is where the crate's two known panics are and which the commit gate
/// refuses; an address is read only by getters that do not go through
/// that code; and a setter is applied once to a freshly parsed address,
/// whose result is parsed again before it is read.
fn guarded(operation: impl FnOnce() -> Reply) -> Reply {
    std::panic::catch_unwind(core::panic::AssertUnwindSafe(operation)).unwrap_or_else(|_| Reply::Type("the address could not be handled".to_owned()))
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
    guarded(|| parse_unguarded(input_bytes, payload))
}

fn parse_unguarded(input_bytes: i32, payload: &str) -> Reply {
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
    guarded(|| set_unguarded(part, address_bytes, payload))
}

fn set_unguarded(part: i32, address_bytes: i32, payload: &str) -> Reply {
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
    if url.as_str().len() > MAX_URL_BYTES {
        return written(&before);
    }
    // What is given is the setter's text parsed again, never the address
    // the setter left (rdm TODO-7390). The crate's setters move its
    // stored positions by their own arithmetic, and can leave an address
    // whose fields are not those of its text, or whose text is no address
    // (`x://:p@`, a password before an empty host). Parsed again, every
    // address a script holds has the parser's own positions, its fields
    // are its text's, and the next operation, which is given the text,
    // takes it. A text the parser refuses changes nothing, as a value the
    // component does not take changes nothing.
    Url::parse(url.as_str()).map_or_else(|_| written(&before), |parsed| written(&parsed))
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

    /// URL standard sections 4.4, 4.7 and 6.1: an address is resolved
    /// against its base and serialised, its origin is its scheme, host
    /// and port, and each field is the API's getter of that name.
    #[test]
    fn an_address_is_resolved_serialised_and_given_by_its_fields() {
        let reply = text(parse(10, "../b?q=1#fhttps://user:pw@Example.test:8443/a/c/d"));
        let fields: Vec<&str> = reply.split('\n').collect();
        assert_eq!(
            fields,
            vec![
                "https://user:pw@example.test:8443/a/b?q=1#f",
                "https://example.test:8443",
                "https:",
                "user",
                "pw",
                "example.test:8443",
                "example.test",
                "8443",
                "/a/b",
                "?q=1",
                "#f"
            ]
        );
        assert_eq!(fields.len(), FIELDS);
    }

    /// The setter cases that made the crate's slicing by position index
    /// past an address's end: each is answered with an address.
    #[test]
    fn the_setters_that_broke_position_slicing_give_addresses() {
        for (part, address, value) in [
            (1, "http://test@example.net", "file"),
            (1, "ssh://me@example.net", "http"),
            (2, "http://example.net", "me"),
            (3, "sc://x", "pw"),
            (4, "sc://x/", ""),
            (5, "foo://x", ""),
        ] {
            let payload = format!("{address}{value}");
            let reply = text(set(part, i32::try_from(address.len()).unwrap(), &payload));
            assert_eq!(reply.split('\n').count(), FIELDS, "{reply}");
        }
    }

    /// An address that ends at an empty authority: the crate's slicing at
    /// `Position::BeforePassword` indexes past its end in a release build
    /// (url 2.5.9, lib.rs:2771), and a script's `new URL("foo://")` ended
    /// the script host. It is an address, given by its fields.
    #[test]
    fn an_address_that_ends_at_an_empty_authority_is_given() {
        for (address, protocol) in [("foo://", "foo:"), ("web://", "web:")] {
            let reply = text(parse(-1, address));
            assert_eq!(reply.split('\n').collect::<Vec<_>>(), vec![address, "null", protocol, "", "", "", "", "", "", "", ""]);
        }
    }

    /// rdm TODO-7390: an address a script holds is always what the parser
    /// gives for its text. The crate's setters can leave a text that the
    /// parser refuses (a password before an empty host), which no later
    /// operation could take, and fields that are not those of the text
    /// (a path of `//` after no host, written as an empty authority): the
    /// first changes nothing, as a refused value does, and the second is
    /// given as its text reads.
    #[test]
    fn a_setter_gives_only_what_its_text_parses_to() {
        let after = |part: i32, address: &str, value: &str| text(set(part, i32::try_from(address.len()).unwrap(), &format!("{address}{value}")));
        // A host of nothing after a password: the text `x://:p@` is no address.
        for (part, address, value) in [(4, "x://:p@h", ""), (5, "x://:p@h", ""), (4, "foo://:me@x", "/")] {
            let reply = after(part, address, value);
            assert_eq!(reply.split('\n').next(), Some(address), "{reply}");
        }
        // Whatever a setter leaves, its fields are those of its text, and
        // every later setter takes it.
        for (part, address, value) in
            [(7, "foo:/", "//"), (4, "foo:/.//p", "x"), (4, "foo:/.//p", ""), (7, "foo://h", ""), (1, "foo://h:1/", "http"), (5, "foo://x", "")]
        {
            let reply = after(part, address, value);
            let href = reply.split('\n').next().unwrap();
            assert_eq!(reply, text(parse(-1, href)), "{address} with {value:?}");
            for later in 1..=9 {
                assert_eq!(after(later, href, "a").split('\n').count(), FIELDS, "{href}");
            }
        }
        assert_eq!(after(4, "foo:/.//p", "x").split('\n').next(), Some("foo://x//p"));
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
        let href = |reply: Reply| text(reply).split('\n').next().unwrap().to_owned();
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

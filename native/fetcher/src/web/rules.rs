//! The Fetch standard's fixed lists: the ports blocked, the methods
//! forbidden, the request headers a script may not set, and the answer's
//! headers a script may not read.

/// The ports the Fetch standard blocks (its section 2.9, "bad port"), to
/// which a request is a network error whatever its scheme.
const BAD_PORTS: [u16; 83] = [
    0, 1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101, 102, 103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135,
    137, 139, 143, 161, 179, 389, 427, 465, 512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587, 601, 636, 989, 990, 993, 995, 1719, 1720,
    1723, 2049, 3659, 4045, 4190, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668, 6669, 6679, 6697, 10080,
];

/// Whether `port` is one the Fetch standard blocks.
pub(crate) fn is_bad_port(port: u16) -> bool {
    BAD_PORTS.contains(&port)
}

/// The methods the Fetch standard forbids (its section 2.2.1).
const FORBIDDEN_METHODS: [&str; 3] = ["CONNECT", "TRACE", "TRACK"];

/// Whether `method` is forbidden, in any case.
pub(crate) fn is_forbidden_method(method: &str) -> bool {
    FORBIDDEN_METHODS.iter().any(|forbidden| method.eq_ignore_ascii_case(forbidden))
}

/// The request headers the Fetch standard forbids a script (its section
/// 2.2.2, "forbidden request-header"), in lower case.
const FORBIDDEN_REQUEST_HEADERS: [&str; 21] = [
    "accept-charset",
    "accept-encoding",
    "access-control-request-headers",
    "access-control-request-method",
    "connection",
    "content-length",
    "cookie",
    "cookie2",
    "date",
    "dnt",
    "expect",
    "host",
    "keep-alive",
    "origin",
    "referer",
    "set-cookie",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "via",
];
/// The headers whose value may not name a forbidden method (the same
/// section).
const METHOD_OVERRIDE_HEADERS: [&str; 3] = ["x-http-method", "x-http-method-override", "x-method-override"];

/// A header's value split into its values as the Fetch standard splits
/// one (its section 2.2.2, "get, decode, and split"): at each comma that
/// is not inside an HTTP quoted string, each value without the spaces and
/// tabs at its ends. A quoted string keeps its quotes, as the standard's
/// own example has it (`"1,2", 3` is `"1,2"` and `3`), so a quoted method
/// is no method.
fn split_values(value: &[u8]) -> Vec<&[u8]> {
    let mut values = Vec::new();
    let (mut start, mut position, mut quoted) = (0_usize, 0_usize, false);
    while let Some(&byte) = value.get(position) {
        match byte {
            // Inside a quoted string a backslash takes the byte after it.
            b'\\' if quoted => position = position.saturating_add(1),
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                values.push(value.get(start..position).unwrap_or_default());
                start = position.saturating_add(1);
            }
            _ => {}
        }
        position = position.saturating_add(1);
    }
    values.push(value.get(start..).unwrap_or_default());
    values.into_iter().map(trim_tab_or_space).collect()
}

/// `value` without HTTP tabs or spaces at its ends.
fn trim_tab_or_space(value: &[u8]) -> &[u8] {
    let is_edge = |byte: &u8| matches!(byte, b' ' | b'\t');
    let start = value.iter().position(|byte| !is_edge(byte)).unwrap_or(value.len());
    let end = value.iter().rposition(|byte| !is_edge(byte)).map_or(start, |last| last.saturating_add(1));
    value.get(start..end).unwrap_or_default()
}

/// Whether a script may not set the header `name` (in lower case) to
/// `value`. The script side's `Headers` drops such a header silently, as
/// the standard says; the web client drops it again, trusting no script.
pub(crate) fn is_forbidden_request_header(name: &str, value: &[u8]) -> bool {
    if FORBIDDEN_REQUEST_HEADERS.contains(&name) || name.starts_with("proxy-") || name.starts_with("sec-") {
        return true;
    }
    METHOD_OVERRIDE_HEADERS.contains(&name) && split_values(value).iter().any(|method| std::str::from_utf8(method).is_ok_and(is_forbidden_method))
}

/// Whether a script may never read the answer's header `name` (in lower
/// case): the Fetch standard's forbidden response-header names, which is
/// how a cookie marked `HttpOnly`, and every other, stays out of a
/// script's reach.
pub(crate) fn is_forbidden_response_header(name: &str) -> bool {
    matches!(name, "set-cookie" | "set-cookie2")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blocked_ports_are_blocked_and_the_webs_are_not() {
        for port in [0, 22, 25, 110, 143, 6667, 10080] {
            assert!(is_bad_port(port), "{port}");
        }
        for port in [80, 443, 8000, 8080, 8443, 3000] {
            assert!(!is_bad_port(port), "{port}");
        }
        assert!(BAD_PORTS.is_sorted(), "the list is in the standard's order");
    }

    /// The Fetch standard's own examples of "get, decode, and split"
    /// (section 2.2.2), and a quoted string's backslash and missing end.
    #[test]
    fn a_value_is_split_as_the_standard_splits_it() {
        let split = |value: &str| split_values(value.as_bytes()).into_iter().map(|part| String::from_utf8_lossy(part).into_owned()).collect::<Vec<_>>();
        assert_eq!(split("nosniff,"), ["nosniff", ""]);
        assert_eq!(split(""), [""]);
        assert_eq!(split("text/html;\", x/x"), ["text/html;\", x/x"]);
        assert_eq!(split("x / x,,,1"), ["x / x", "", "", "1"]);
        assert_eq!(split("\"1,2\", 3"), ["\"1,2\"", "3"]);
        assert_eq!(split(" \tPUT\t , TRACE "), ["PUT", "TRACE"]);
        assert_eq!(split("\"a\\\",b\", c"), ["\"a\\\",b\"", "c"], "a backslash takes the quote after it");
        assert_eq!(split("\"a,b"), ["\"a,b"], "a quoted string with no end runs to the value's");
    }

    #[test]
    fn forbidden_methods_and_headers_are_known_in_any_case() {
        assert!(is_forbidden_method("trace") && is_forbidden_method("CONNECT") && is_forbidden_method("Track"));
        assert!(!is_forbidden_method("GET") && !is_forbidden_method("PATCH"));
        for name in ["cookie", "host", "origin", "referer", "content-length", "proxy-authorization", "sec-fetch-site", "accept-encoding"] {
            assert!(is_forbidden_request_header(name, b"x"), "{name}");
        }
        for name in ["accept", "authorization", "content-type", "user-agent", "x-requested-with", "security"] {
            assert!(!is_forbidden_request_header(name, b"x"), "{name}");
        }
        assert!(is_forbidden_request_header("x-http-method-override", b"GET, trace"));
        assert!(!is_forbidden_request_header("x-http-method-override", b"PUT"));
        assert!(is_forbidden_response_header("set-cookie") && !is_forbidden_response_header("content-type"));
    }
}

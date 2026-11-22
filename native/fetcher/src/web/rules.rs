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

/// Whether a script may not set the header `name` (in lower case) to
/// `value`. The script side's `Headers` drops such a header silently, as
/// the standard says; the web client drops it again, trusting no script.
pub(crate) fn is_forbidden_request_header(name: &str, value: &[u8]) -> bool {
    if FORBIDDEN_REQUEST_HEADERS.contains(&name) || name.starts_with("proxy-") || name.starts_with("sec-") {
        return true;
    }
    METHOD_OVERRIDE_HEADERS.contains(&name)
        && value.split(|&byte| byte == b',').any(|method| std::str::from_utf8(method.trim_ascii()).is_ok_and(is_forbidden_method))
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

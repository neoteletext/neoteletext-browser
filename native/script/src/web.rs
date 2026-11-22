//! The web APIs a script is given (work package W2; script-api.md
//! section 9): `fetch`, `Request`, `Response`, `Headers`, `Blob`, `File`,
//! `FormData`, `URL`, `URLSearchParams`, `TextEncoder`, `TextDecoder` and
//! their streams, the Streams standard's classes, `EventSource`,
//! `WebSocket`, `MessageEvent` and `CloseEvent`, each with the names,
//! arguments, results and errors its standard gives but for the numbered
//! departures of section 9.4.
//!
//! The script's side is JavaScript in the prelude (`web/*.js`), written
//! from the standards and citing their sections, with one bundled
//! library, the Streams standard's implementation `web-streams-polyfill`
//! (`web/streams/`, pinned by version and SHA-256; neoteletext-common's
//! DEC-7261). The host's side is [`crate::host::web`]: operations 40 to
//! 49 and events 30 to 41. The network's rules are the web client's and
//! none is here (DEC-7260). [`mock`] stands in for the web client in
//! tests and in the standalone runner.

pub mod mock;

use core::fmt::Write as _;

pub use crate::host::web::{MAX_NETWORK_EVENT_BYTES, NetworkEvent, Web, limits};

/// The Streams standard's implementation, as its package ships it.
const STREAMS: &str = include_str!("web/streams/ponyfill.js");
/// The SHA-256 of each bundled file, as `sha256sum` writes them.
#[cfg(test)]
const STREAMS_SUMS: &str = include_str!("web/streams/SHA256SUMS");
/// The script's side, in the order it is written into the prelude: each
/// part uses the ones before it.
const PARTS: [&str; 3] = [include_str!("web/encoding_url.js"), include_str!("web/fetch.js"), include_str!("web/sockets.js")];

/// Writes the web APIs' part of the prelude after the shared core's,
/// inside the prelude's one function: the bundled streams, held in one
/// function of their own that gives its exports and adds no global; the
/// tables the two halves must agree on; and the script's side.
pub(crate) fn write_prelude(out: &mut String) {
    // The package is a universal module: given `exports` and `module` it
    // puts its classes on `exports` and touches nothing else.
    out.push_str("\nconst WebStreams = (function () {\nconst exports = {};\nconst module = { exports };\n");
    out.push_str(STREAMS);
    out.push_str("\nreturn exports;\n}).call(globalThis);\n");
    for (name, entries) in crate::host::web::prelude_tables() {
        let _ = write!(out, "const {name} = Object.freeze({{");
        for (key, number) in entries {
            let _ = write!(out, "{key}: {number}, ");
        }
        out.push_str("});\n");
    }
    for part in PARTS {
        out.push_str(part);
        out.push('\n');
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, reason = "tests")]
mod tests {
    use super::*;

    /// DEC-7261: the bundled library is pinned by its SHA-256. The bytes
    /// compiled in are the bytes `web/streams/SHA256SUMS` records, which
    /// `web/streams/fetch.py --check` holds to the package.
    #[test]
    fn the_bundled_streams_are_the_pinned_bytes() {
        let recorded = STREAMS_SUMS.lines().find_map(|line| line.strip_suffix("  ponyfill.js")).unwrap();
        let digest = ring::digest::digest(&ring::digest::SHA256, STREAMS.as_bytes());
        let written = digest.as_ref().iter().fold(String::new(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        });
        assert_eq!(written, recorded);
    }

    /// The prelude with the web APIs in it is one script the container
    /// takes: within the bound on a script's length.
    #[test]
    fn the_whole_prelude_is_a_script_the_container_takes() {
        assert!(crate::ops::prelude().len() <= crate::limits::MAX_SCRIPT_BYTES);
    }
}

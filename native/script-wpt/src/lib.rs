//! The web-platform-tests harness (work package W3 of
//! `neoteletext-common/docs/script-api.md`, section 9.4; DEC-7260 item 9:
//! exactness is measured, not claimed).
//!
//! The owner's rule is that a script's `fetch` has exactly the web's API
//! (DEC-7252) and that every difference is a listed departure with its
//! reason. The web-platform-tests are how that is measured: each test
//! file the suite writes for a worker's scope is run as a page's script
//! in the standalone runner, with the suite's own `testharness.js`, and
//! every case's result is compared with the result kept for it in
//! `expectations/`. A case that fails is kept there with the departure
//! that explains it or the to-do that owes its fix; a case that fails
//! with neither fails the harness.
//!
//! The tests themselves are third-party files (3-Clause BSD) and are not
//! in this repository. `fetch.py` fetches them at a pinned commit; the
//! harness reads them from the folder [`ROOT_VARIABLE`] names, and its
//! tests are ignored without it, never passed.
//!
//! The harness is the crate's tests (`tests/wpt.rs`); this library holds
//! what they and the gates name.

/// The environment variable that names the folder of the fetched
/// web-platform-tests: the folder `fetch.py` wrote, which holds
/// `resources/testharness.js`.
pub const ROOT_VARIABLE: &str = "NEOTELETEXT_WPT";
/// The environment variable that, set to `1`, has the harness write the
/// expectations from what it ran instead of comparing with them.
pub const BLESS_VARIABLE: &str = "NEOTELETEXT_WPT_BLESS";
/// The commit of the web-platform-tests the expectations were written
/// against, which `fetch.py` fetches.
pub const COMMIT: &str = "42e27408034f57afe4c726476d1a0bd96b873129";

//! Helpers the trusted core's tests share (`00-conventions.md` section 5).

use std::sync::mpsc;

use neoteletext::bookmarks::Bookmarks;
use neoteletext::outbox::Outbox;
use neoteletext::settings::Settings;

use super::Core;
use crate::session::Session;

/// A trusted core for a test: no tabs, no helpers, the interface a
/// buffer.
#[cfg(unix)]
pub(super) fn test_core() -> Core {
    let (events, _incoming) = mpsc::channel();
    Core::new(Session::new(Bookmarks::default()), Vec::new(), (Settings::default(), None), events, Outbox::new(Vec::new()), None)
}

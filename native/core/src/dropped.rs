//! What a strict reader of a kept file dropped, for the log (finding
//! F-31): the first messages and a count of the rest, so a large malformed
//! file costs a bounded report, not a message and a line of the log for
//! every line of it.

/// The most messages kept of one file's dropped lines.
pub const MAX_DROPPED_REPORTED: usize = 64;

/// What a reader dropped: its first messages, and how many in all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dropped {
    reported: Vec<String>,
    count: usize,
}

impl Dropped {
    /// One thing dropped, said by `message`.
    #[must_use]
    pub fn one(message: String) -> Self {
        Self { reported: vec![message], count: 1 }
    }

    /// Notes one thing dropped; `message` says what, and is made only for
    /// the first [`MAX_DROPPED_REPORTED`].
    pub fn push(&mut self, message: impl FnOnce() -> String) {
        if self.reported.len() < MAX_DROPPED_REPORTED {
            self.reported.push(message());
        }
        self.count = self.count.saturating_add(1);
    }

    /// How many things were dropped in all.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether nothing was dropped.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The lines for the log: the first messages, then the count of the
    /// rest when there were more.
    #[must_use]
    pub fn lines(self) -> Vec<String> {
        let more = self.count.saturating_sub(self.reported.len());
        let mut lines = self.reported;
        if more > 0 {
            lines.push(format!("and {more} more lines dropped"));
        }
        lines
    }
}

impl IntoIterator for Dropped {
    type Item = String;
    type IntoIter = std::vec::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines().into_iter()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    /// Lines of a file, every one broken, after its version line.
    fn broken(version: &str) -> String {
        const LINES: usize = 100_000;
        let mut text = format!("{version}\n");
        for _ in 0..LINES {
            text.push_str("broken\n\n");
        }
        text
    }

    /// What a reader reported: every broken line counted, and at most the
    /// bound's messages and one line with the count of the rest.
    fn bounded(dropped: Dropped) {
        assert_eq!(dropped.len(), 100_000);
        let lines = dropped.lines();
        assert!(lines.len() <= MAX_DROPPED_REPORTED + 1, "{} messages", lines.len());
        assert_eq!(lines.last().unwrap(), &format!("and {} more lines dropped", 100_000 - MAX_DROPPED_REPORTED));
    }

    /// Finding F-31: a large malformed file costs a bounded report, the
    /// first messages and a count, in every strict reader, its empty lines
    /// skipped; before, one message a line.
    #[test]
    fn a_malformed_file_is_reported_within_a_bound() {
        bounded(crate::history::History::read(&broken(crate::history::VERSION_LINE)).1);
        bounded(crate::settings::Settings::read(&broken(crate::settings::VERSION_LINE)).1);
        bounded(crate::restore::Saved::read(&broken(crate::restore::VERSION_LINE)).1);
        bounded(crate::bookmarks::Bookmarks::read(&broken(crate::bookmarks::VERSION_LINE)).1);
        let cookies = format!("{}\n", broken(crate::cookies::FILE_VERSION).replace("\n\n", "\n"));
        bounded(crate::cookies::Store::new().import(cookies.as_bytes(), 0));
    }
}

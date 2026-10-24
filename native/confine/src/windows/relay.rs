//! A helper's standard error, relayed by the broker (decision 100 item 1,
//! finding F-22). A helper starts detached from any console, so it has no
//! console to type into; what it reports on its standard error comes to
//! the broker through a pipe and goes on to the broker's own, a line at a
//! time, with its control characters removed, so a helper cannot write a
//! terminal's escape sequences either, some of which have the terminal
//! answer into its own input.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read as _, Write};

/// The longest line passed on whole; a longer one goes on in pieces of
/// this length. Far longer than any line a helper reports.
pub(super) const MAX_LINE: usize = 4096;
/// A relay's stack: it holds one line and a buffer, not the default 2 MB
/// for each of up to `broker::MAX_HELPERS` helpers.
const RELAY_STACK: usize = 64 * 1024;

/// Copies `pipe`, the broker's end of a helper's standard error, to the
/// broker's standard error on a thread of its own, which ends when the
/// helper and every copy of its end are gone.
///
/// # Errors
/// When the thread cannot be started.
pub(super) fn relay(pipe: File) -> io::Result<()> {
    std::thread::Builder::new()
        .name(String::from("helper standard error"))
        .stack_size(RELAY_STACK)
        .spawn(move || pass_on(BufReader::new(pipe), &mut io::stderr()))
        .map(drop)
}

/// Passes `from`'s lines on to `to`, each cleaned and whole in one write,
/// until `from` ends. A write that fails loses that line only: the helper
/// is never held up by a full pipe.
pub(super) fn pass_on(mut from: impl BufRead, to: &mut impl Write) {
    let mut line = Vec::new();
    loop {
        line.clear();
        match (&mut from).take(MAX_LINE as u64).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let _ = to.write_all(clean(&line).as_bytes());
            }
        }
    }
}

/// A line as text with every control character removed but the tab, and
/// one line end.
fn clean(line: &[u8]) -> String {
    let mut text: String = String::from_utf8_lossy(line).chars().filter(|character| *character == '\t' || !character.is_control()).collect();
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a helper writes reaches the broker as text: an escape sequence
    /// (here a request for the cursor's place, which a terminal answers
    /// into its input) loses its control characters, and a line longer
    /// than the bound goes on in pieces.
    #[test]
    fn lines_go_on_without_their_control_characters() {
        let long = "x".repeat(MAX_LINE + 10);
        let written = format!("neoteletext-decoder: confined\r\n\u{1b}[6n\u{7}tab\there\u{9b}6n\n{long}\nno line end");
        let mut passed = Vec::new();
        pass_on(written.as_bytes(), &mut passed);
        let passed = String::from_utf8(passed).unwrap();
        let expected = format!("neoteletext-decoder: confined\n[6ntab\there6n\n{}\n{}\nno line end\n", "x".repeat(MAX_LINE), "x".repeat(10));
        assert_eq!(passed, expected);
    }
}

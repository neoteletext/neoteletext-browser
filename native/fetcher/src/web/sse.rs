//! Server-sent events (the HTML standard, section 9.2.6, "interpreting an
//! event stream"): an answer's body read as lines, and the lines as
//! events. Nothing here touches the network; the web client feeds it the
//! body's pieces.

use protocol::web::{MAX_EVENT_DATA, MAX_EVENT_ID, MAX_EVENT_TYPE};

/// An event, as the script is given it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Event {
    /// Its type, `message` when the stream named none.
    pub(crate) kind: String,
    /// Its data.
    pub(crate) data: String,
    /// The last event ID.
    pub(crate) last_id: String,
}

/// A line, an event's data or an ID past its bound.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TooLong;

/// The byte order mark a stream may start with, which is not part of it.
const BYTE_ORDER_MARK: [u8; 3] = [0xEF, 0xBB, 0xBF];
/// The longest line kept: an event's whole data in one `data:` line, with
/// its field name.
const MAX_LINE: usize = MAX_EVENT_DATA + 8;

/// An event stream being read.
#[derive(Default)]
pub(crate) struct EventStream {
    /// The bytes of the line not yet ended.
    line: Vec<u8>,
    /// Whether the stream's first bytes, where a byte order mark may be,
    /// are passed.
    started: bool,
    /// Whether the last byte was a carriage return, whose line feed, if
    /// one follows, ends the same line.
    after_carriage_return: bool,
    /// The data of the event being read, each line with a line feed after.
    data: String,
    /// Its type.
    event: String,
    /// The last event ID buffer, kept across events and reconnections.
    id_buffer: String,
    /// The last event ID as the last event dispatched set it, which a
    /// reconnection sends.
    last_id: String,
    /// The reconnection time the stream asked for, in milliseconds.
    retry: Option<u64>,
}

impl EventStream {
    /// The last event ID, for the `Last-Event-ID` header of a
    /// reconnection.
    pub(crate) fn last_id(&self) -> &str {
        &self.last_id
    }

    /// The reconnection time the stream asked for, in milliseconds.
    pub(crate) const fn retry(&self) -> Option<u64> {
        self.retry
    }

    /// Forgets the unfinished line and event of a connection that ended:
    /// the standard discards an event not ended by a blank line. The last
    /// event ID and the reconnection time stay.
    pub(crate) fn connection_ended(&mut self) {
        self.line.clear();
        self.data.clear();
        self.event.clear();
        self.id_buffer.clone_from(&self.last_id);
        self.started = false;
        self.after_carriage_return = false;
    }

    /// Reads the body's next bytes, giving each event they complete to
    /// `emit`.
    ///
    /// # Errors
    /// [`TooLong`] when a line or an event's data passes its bound; the
    /// stream is then to be closed.
    pub(crate) fn push(&mut self, bytes: &[u8], emit: &mut dyn FnMut(Event)) -> Result<(), TooLong> {
        for &byte in bytes {
            if core::mem::take(&mut self.after_carriage_return) && byte == b'\n' {
                continue;
            }
            match byte {
                b'\r' | b'\n' => {
                    self.after_carriage_return = byte == b'\r';
                    let line = core::mem::take(&mut self.line);
                    self.started = true;
                    self.take_line(&line, emit)?;
                }
                _ => {
                    if self.line.len() >= MAX_LINE {
                        return Err(TooLong);
                    }
                    self.line.push(byte);
                    // A byte order mark at the very start is dropped once
                    // it is whole.
                    if !self.started && self.line.len() == BYTE_ORDER_MARK.len() {
                        self.started = true;
                        if self.line == BYTE_ORDER_MARK {
                            self.line.clear();
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// One line, without its end.
    fn take_line(&mut self, line: &[u8], emit: &mut dyn FnMut(Event)) -> Result<(), TooLong> {
        if line.is_empty() {
            self.dispatch(emit);
            return Ok(());
        }
        // The stream is UTF-8; bytes that are not are replaced.
        let line = String::from_utf8_lossy(line);
        let (field, value) = match line.split_once(':') {
            // A line that starts with a colon is a comment.
            Some(("", _)) => return Ok(()),
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line.as_ref(), ""),
        };
        match field {
            "event" => {
                if value.len() > MAX_EVENT_TYPE {
                    return Err(TooLong);
                }
                value.clone_into(&mut self.event);
            }
            "data" => {
                if self.data.len().saturating_add(value.len()).saturating_add(1) > MAX_EVENT_DATA.saturating_add(1) {
                    return Err(TooLong);
                }
                self.data.push_str(value);
                self.data.push('\n');
            }
            // An ID with a NUL is ignored.
            "id" if !value.contains('\0') => {
                if value.len() > MAX_EVENT_ID {
                    return Err(TooLong);
                }
                value.clone_into(&mut self.id_buffer);
            }
            "retry" if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
                // Digits beyond what a number holds ask for the longest wait.
                self.retry = Some(value.parse::<u64>().unwrap_or(u64::MAX));
            }
            _ => {}
        }
        Ok(())
    }

    /// A blank line: the event read so far is dispatched.
    fn dispatch(&mut self, emit: &mut dyn FnMut(Event)) {
        self.last_id.clone_from(&self.id_buffer);
        let mut data = core::mem::take(&mut self.data);
        let event = core::mem::take(&mut self.event);
        if data.is_empty() {
            return;
        }
        // The line feed after the last line of data is not part of it.
        data.pop();
        emit(Event { kind: if event.is_empty() { "message".to_owned() } else { event }, data, last_id: self.last_id.clone() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The events of `stream`, given `at_a_time` bytes a call.
    fn events(stream: &[u8], at_a_time: usize) -> (Vec<(String, String, String)>, EventStream) {
        let mut reader = EventStream::default();
        let mut found = Vec::new();
        for piece in stream.chunks(at_a_time) {
            reader.push(piece, &mut |event| found.push((event.kind, event.data, event.last_id))).unwrap();
        }
        (found, reader)
    }

    fn event(event: &str, data: &str, last_id: &str) -> (String, String, String) {
        (event.to_owned(), data.to_owned(), last_id.to_owned())
    }

    /// The HTML standard's own examples (section 9.2.6), and its rules for
    /// each field.
    #[test]
    fn a_stream_is_read_as_the_standard_says() {
        let stream = b"\xEF\xBB\xBF: a comment\n\
data: first\n\n\
data\n\n\
data\ndata\n\n\
data:second line one\ndata:  second line two\r\n\r\n\
event: add\ndata: 73857293\rid: 7\r\r\
data: kept ID\n\n\
id\ndata: cleared ID\n\n\
retry: 1500\nretry: soon\nid: with\0nul\nunknown: field\ndata: last\n\n\
data: never dispatched";
        let expected = vec![
            event("message", "first", ""),
            event("message", "", ""),
            event("message", "\n", ""),
            event("message", "second line one\n second line two", ""),
            event("add", "73857293", "7"),
            event("message", "kept ID", "7"),
            event("message", "cleared ID", ""),
            event("message", "last", ""),
        ];
        // The second "data" block is `data` alone then a blank: one empty
        // line of data, which dispatches as the empty string.
        for at_a_time in [1, 2, 3, 7, stream.len()] {
            let (found, reader) = events(stream, at_a_time);
            assert_eq!(found, expected, "{at_a_time} bytes at a time");
            assert_eq!(reader.retry(), Some(1500));
            assert_eq!(reader.last_id(), "");
        }
    }

    /// The last event ID a reconnection sends is the last dispatched one;
    /// an event cut by the connection's end is discarded.
    #[test]
    fn a_connections_end_discards_the_unfinished_event() {
        let (found, mut reader) = events(b"id: 1\ndata: a\n\nid: 2\ndata: cut", 4);
        assert_eq!(found, [event("message", "a", "1")]);
        assert_eq!(reader.last_id(), "1", "the ID of an event not dispatched is not the last");
        reader.connection_ended();
        let mut after = Vec::new();
        reader.push(b"\xEF\xBB\xBFdata: b\n\n", &mut |event| after.push((event.kind, event.data, event.last_id))).unwrap();
        assert_eq!(after, [event("message", "b", "1")]);
    }

    #[test]
    fn bytes_that_are_not_utf8_are_replaced_and_bounds_hold() {
        let (found, _) = events(b"data: a\xFFb\n\n", 1);
        assert_eq!(found, [event("message", "a\u{FFFD}b", "")]);
        let mut reader = EventStream::default();
        let mut sink = |_: Event| {};
        assert_eq!(reader.push(&vec![b'a'; MAX_LINE + 1], &mut sink), Err(TooLong), "a line with no end");
        let mut reader = EventStream::default();
        let line = format!("data: {}\n", "a".repeat(MAX_EVENT_DATA / 2));
        assert_eq!(reader.push(line.as_bytes(), &mut sink), Ok(()));
        assert_eq!(reader.push(line.as_bytes(), &mut sink), Err(TooLong), "data past its bound over two lines");
        let mut reader = EventStream::default();
        assert_eq!(reader.push(format!("event: {}\n", "e".repeat(MAX_EVENT_TYPE + 1)).as_bytes(), &mut sink), Err(TooLong));
        assert_eq!(reader.push(format!("id: {}\n", "i".repeat(MAX_EVENT_ID + 1)).as_bytes(), &mut sink), Err(TooLong));
        let mut whole = EventStream::default();
        let mut found = Vec::new();
        whole.push(format!("data: {}\n\n", "a".repeat(MAX_EVENT_DATA)).as_bytes(), &mut |event| found.push(event.data.len())).unwrap();
        assert_eq!(found, [MAX_EVENT_DATA], "an event of exactly the bound passes");
    }
}

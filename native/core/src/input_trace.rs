//! The input trace (the owner, 27 October 2026): with `--input-trace`, the
//! trusted core describes each input event it sends a channel, for the
//! open page's row 0, with the number sent so far and, while the same
//! event goes again, how many times running. Each message is read back
//! with the engine's reader as it was sent, after the session's modes
//! filtered it, so the trace shows exactly what went. It never shows what
//! was typed, as the console never does (requirement 19.9): text, a paste
//! and a preedit by their length, a key by its name, a character key as
//! such.

use std::fmt::Write as _;

use neoteletext_input::event::{Action, Event, Item, KeyCode, MouseAction, read_message};
use neoteletext_input::keys;
use protocol::clean;
use protocol::interaction::{MAX_TRACE, TraceLine};

/// What went last and how often, while the trace is on.
#[derive(Debug, Default)]
pub struct InputTrace {
    last: Option<String>,
    repeats: u32,
    sent: u64,
}

impl InputTrace {
    /// Counts the input message `message`, sent to a channel, and gives its line.
    pub fn note(&mut self, message: &[u8]) -> TraceLine {
        let mut parts: Vec<String> = Vec::new();
        if read_message(message, &mut |item| parts.push(describe(&item))).is_err() {
            parts.push(String::from("unreadable"));
        }
        let text = clean(&parts.join(", "), MAX_TRACE);
        self.sent = self.sent.saturating_add(1);
        if self.last.as_deref() == Some(text.as_str()) {
            self.repeats = self.repeats.saturating_add(1);
        } else {
            self.last = Some(text.clone());
            self.repeats = 1;
        }
        TraceLine { text, repeats: self.repeats, sent: self.sent }
    }
}

fn action_name(action: Action) -> &'static str {
    match action {
        Action::Press => "press",
        Action::Repeat => "repeat",
        Action::Release => "release",
    }
}

/// One item as the trace says it.
fn describe(item: &Item<'_>) -> String {
    let event = match item {
        Item::Text(text) => return format!("TEXT {} chars", text.chars().count()),
        Item::Event(event) => event,
    };
    match event {
        Event::Key(key) => {
            let name = match key.key {
                KeyCode::Functional(code) => keys::name(code).unwrap_or("unnamed"),
                KeyCode::Character(_) => "a character",
            };
            let mut text = format!("KEY {name} {}", action_name(key.action));
            if key.mods != 0 {
                let _ = write!(text, " mods {:#04x}", key.mods);
            }
            text
        }
        Event::Mouse(mouse) => {
            let action = match mouse.action {
                MouseAction::Press => "press",
                MouseAction::Release => "release",
                MouseAction::Move => "move",
            };
            format!("MOUSE {action} b{} {},{}", mouse.button, mouse.at.column, mouse.at.row)
        }
        Event::Wheel(wheel) => format!("WHEEL {},{} {},{}", wheel.dx, wheel.dy, wheel.at.column, wheel.at.row),
        Event::Paste(text) => format!("PASTE {} chars", text.chars().count()),
        Event::FocusIn => String::from("FOCUS_IN"),
        Event::FocusOut => String::from("FOCUS_OUT"),
        Event::Resize { columns, rows, .. } => format!("RESIZE {columns}x{rows}"),
        Event::Compose { text, .. } => format!("COMPOSE {} chars", text.chars().count()),
        Event::Hello { version, .. } => format!("HELLO v{version}"),
        Event::Page(page) => format!("PAGE {page:03X}"),
        Event::Hover { leave, column, row } => format!("HOVER {} {column},{row}", if *leave { "leave" } else { "enter" }),
        Event::Extension { kind, .. } => format!("EXTENSION {kind:#04x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neoteletext_input::event::{Mouse, Point, write};

    fn message(item: &Item<'_>) -> Vec<u8> {
        let mut out = [0_u8; 64];
        let length = write(item, &mut out).unwrap();
        out.get(..length).unwrap().to_vec()
    }

    /// The same event again counts up; another starts afresh; the total
    /// counts every one; what was typed is never shown.
    #[test]
    fn the_trace_counts_the_same_event_and_never_shows_typing() {
        let mut trace = InputTrace::default();
        let at = Point { column: 12, row: 5, pixels: None, mods: 0 };
        let moved = message(&Item::Event(Event::Mouse(Mouse { action: MouseAction::Move, button: 0, at })));
        assert_eq!(trace.note(&moved), TraceLine { text: "MOUSE move b0 12,5".into(), repeats: 1, sent: 1 });
        assert_eq!(trace.note(&moved).repeats, 2);
        assert_eq!(trace.note(&moved).repeats, 3);
        let typed = trace.note(&message(&Item::Text("secret")));
        assert_eq!(typed, TraceLine { text: "TEXT 6 chars".into(), repeats: 1, sent: 4 });
        let key = neoteletext_input::event::Key { action: Action::Press, key: KeyCode::Character('s'), mods: 0, text: None, base: None };
        assert_eq!(trace.note(&message(&Item::Event(Event::Key(key)))).text, "KEY a character press");
        assert_eq!(trace.note(&message(&Item::Event(Event::Page(0x128)))).text, "PAGE 128");
    }
}

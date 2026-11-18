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

use neoteletext_input::event::{Action, Device, Event, Item, KeyCode, MouseAction, read_message};
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
            let device = match mouse.device {
                Device::Mouse => "",
                Device::Touch => " touch",
                Device::Pen => " pen",
            };
            format!("MOUSE {}{device} b{} {},{}", pointer_action(mouse.action), mouse.button, mouse.at.column, mouse.at.row)
        }
        Event::Touch(touch) => format!("TOUCH {} {} {},{}", pointer_action(touch.action), touch.id, touch.column, touch.row),
        Event::Resume(_) => String::from("RESUME"),
        Event::Wheel(wheel) => format!("WHEEL {},{} {},{}", wheel.dx, wheel.dy, wheel.at.column, wheel.at.row),
        Event::Paste { text, .. } => format!("PASTE {} chars", text.chars().count()),
        Event::FocusIn => String::from("FOCUS_IN"),
        Event::FocusOut => String::from("FOCUS_OUT"),
        Event::Resize { columns, rows, .. } => format!("RESIZE {columns}x{rows}"),
        Event::Compose { text, .. } => format!("COMPOSE {} chars", text.chars().count()),
        Event::Hello { version, .. } => format!("HELLO v{version}"),
        Event::Select(select, "") => format!("SELECT {:03X}/{:04X}", select.page, select.sub),
        Event::Select(select, parameters) => format!("SELECT {:03X}/{:04X}?{parameters}", select.page, select.sub),
        Event::Hover { leave, column, row, .. } => format!("HOVER {} {column},{row}", if *leave { "leave" } else { "enter" }),
        Event::Pick(pick) => {
            format!("PICK {} {},{} at {},{} c{}", pick.drawing, pick.x, pick.y, pick.picture_x, pick.picture_y, pick.commits)
        }
        Event::DrawingHover { leave, drawing, commits, .. } => {
            format!("DRAWING_HOVER {} {drawing} c{commits}", if *leave { "leave" } else { "enter" })
        }
        // Viewer state's messages by their kind alone: never a key or a
        // signature (VS-209).
        Event::StateProof(_) => String::from("STATE_PROOF"),
        Event::StateNone(reason) => format!("STATE_NONE {}", reason.code()),
        Event::StateOffer => String::from("STATE_OFFER"),
        // A rule's report (page-rules.md RULE-46): its rule and tick, and
        // the input it carries, described as that input always was.
        Event::RuleEvent(rule) => {
            let carried = match (rule.input(), rule.body) {
                (Some((item, pick)), _) => {
                    let mut text = describe(&item);
                    if let Some(pick) = pick {
                        let _ = write!(text, " + {}", describe(&Item::Event(Event::Pick(pick))));
                    }
                    text
                }
                (None, neoteletext_input::rules::RuleBody::Value { reading, .. }) => format!("value {reading}"),
                (None, _) => String::from("fired"),
            };
            format!("RULE {} t{}: {carried}", rule.rule, rule.tick)
        }
        Event::PageState(_) => String::from("PAGE_STATE"),
        Event::RulesFault(fault) => format!("RULES_FAULT {} x{}", fault.fault.name(), fault.count),
        Event::Extension { kind, .. } => format!("EXTENSION {kind:#04x}"),
    }
}

/// A pointer's action as the trace names it (DOC-1938).
fn pointer_action(action: MouseAction) -> &'static str {
    match action {
        MouseAction::Press => "press",
        MouseAction::Release => "release",
        MouseAction::Move => "move",
        MouseAction::Gone => "gone",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neoteletext_input::event::{Mouse, Point, Stamp, write};

    /// The page on screen.
    const ON: Stamp = Stamp { page: 0x100, sub: 0 };

    fn message(item: &Item<'_>) -> Vec<u8> {
        let mut out = [0_u8; 64];
        let length = write(item, &mut out).unwrap();
        out.get(..length).unwrap().to_vec()
    }

    /// The report of rule 2 at page tick 7 that carries `item`, a viewer's
    /// input, as it goes under push protocol 10 (page-rules.md RULE-46).
    fn reported(item: &Item<'_>) -> Vec<u8> {
        use neoteletext_input::rules::{RuleBody, RuleEvent};
        let carried = message(item);
        let event = RuleEvent {
            on: ON,
            generation: 1,
            rule: 2,
            row: neoteletext_rules::limits::NO_RULE,
            tick: 7,
            target: neoteletext_rules::table::Target::NONE,
            body: RuleBody::Input(&carried),
        };
        message(&Item::Event(Event::RuleEvent(event)))
    }

    /// The same event again counts up; another starts afresh; the total
    /// counts every one; what was typed is never shown.
    #[test]
    fn the_trace_counts_the_same_event_and_never_shows_typing() {
        let mut trace = InputTrace::default();
        let at = Point { column: 12, row: 5, pixels: None, mods: 0 };
        let moved = reported(&Item::Event(Event::Mouse(Mouse { on: ON, action: MouseAction::Move, button: 0, at, device: Device::Mouse })));
        assert_eq!(trace.note(&moved), TraceLine { text: "RULE 2 t7: MOUSE move b0 12,5".into(), repeats: 1, sent: 1 });
        assert_eq!(trace.note(&moved).repeats, 2);
        assert_eq!(trace.note(&moved).repeats, 3);
        let typed = trace.note(&reported(&Item::Text("secret")));
        assert_eq!(typed, TraceLine { text: "RULE 2 t7: TEXT 6 chars".into(), repeats: 1, sent: 4 });
        let key = neoteletext_input::event::Key { on: ON, action: Action::Press, key: KeyCode::Character('s'), mods: 0, text: None, base: None, lost: false };
        assert_eq!(trace.note(&reported(&Item::Event(Event::Key(key)))).text, "RULE 2 t7: KEY a character press");
        // RULE-46: input outside a report is not what this core sends.
        assert_eq!(trace.note(&message(&Item::Event(Event::Key(key)))).text, "unreadable");
        let select = neoteletext_input::event::Select::new(0x128, 0).unwrap();
        assert_eq!(trace.note(&message(&Item::Event(Event::Select(select, "")))).text, "SELECT 128/0000");
        assert_eq!(trace.note(&message(&Item::Event(Event::Select(select, "a=1")))).text, "SELECT 128/0000?a=1");
    }
}

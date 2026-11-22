//! The input script (`--input FILE`, script-api.md section 16): every
//! event of section 4.2, each on the tick it is given. One event a line:
//!
//! ```text
//! TICK kind fields
//! ```
//!
//! `TICK` is the tick whose step carries the event, counted from 0, and
//! never goes down from one line to the next. A line that is empty or
//! begins with `#` is skipped. The kinds, with `[...]` optional:
//!
//! | Line | Event |
//! |---|---|
//! | `keydown NAME [shift] [ctrl] [alt] [meta] [repeat]` | `keydown` |
//! | `keyup NAME [shift] [ctrl] [alt] [meta] [lost]` | `keyup` |
//! | `key NAME [shift] [ctrl] [alt] [meta]` | a `keydown` and its `keyup` |
//! | `text TEXT` | `text`; the rest of the line, with `\n`, `\t` and `\\` |
//! | `pointer ACTION X Y [button=N] [buttons=N] [type=mouse\|touch\|pen] [id=N] [secondary] [thing=H] [local=X,Y] [under=H,H,...]` | `pointerdown`, `pointerup`, `pointermove`, `pointerenter`, `pointerleave`, `pointercancel` for the actions `down`, `up`, `move`, `enter`, `leave`, `cancel` |
//! | `wheel DX DY [thing=H]` | `wheel` |
//! | `focus`, `blur` | `focus`, `blur` |
//! | `hide`, `show` | `visibilitychange`: the tab hidden (the step is the last before the pause), or shown again |
//! | `navigate CAUSE DESTINATION` | `navigate`; the cause is `script`, `link`, `key` or `browser` |
//! | `ui H WHAT [JSON]` | a component's event (section 8) |
//! | `close` | the contexts close and the run ends after this tick |
//!
//! And what a decoder reports to a script of what its renderer did, so
//! that a script's handling of each can be tested with no renderer:
//!
//! | Line | Report |
//! |---|---|
//! | `painted H` | `painted` on the thing `H` |
//! | `cycleend H` | `cycleend` on the thing `H` |
//! | `cue VOICE CUE`, `ended VOICE` | `cue` and `ended` on `nt.sound` |
//! | `fault CODE BOUND` | `fault` |
//! | `bounds H X Y WIDTH HEIGHT` | a tracked drawing's bounds |
//!
//! A thing is named by its handle number, as the developer view writes
//! it; `x` and `y` are in the renderer's own units (section 4.4). The
//! browser's developer view records a session in this form.

use neoteletext_script::limits::{MAX_KEY_NAME_BYTES, MAX_POINTER_OBJECTS, MAX_TEXT_EVENT_BYTES, MAX_URL_BYTES};
use neoteletext_script::ops::{KeyBit, NavigateCause, PointerAction};
use protocol::script::{Bounds, Event, EventKind, FaultReport, MAX_BOUND_NAME_BYTES, MAX_EVENT_PAYLOAD_BYTES, SoundReport};

use crate::limits::{MAX_INPUT_BYTES, MAX_INPUT_EVENTS, MAX_INPUT_LINE_BYTES, MAX_TICKS};

/// Where a pointer event's first number holds its button: bits 4 to 7,
/// above the action (DEC-7334's packing of event 5, which the script
/// library's prelude unpacks; the runner's tests give an event through
/// the prelude and read every field back).
pub const POINTER_BUTTON_SHIFT: u32 = 4;
/// The most a button number is: four bits.
pub const MAX_POINTER_BUTTON: i32 = 15;
/// Where it holds the pointer's type: bits 8 and 9.
pub const POINTER_TYPE_SHIFT: u32 = 8;
/// The bit that says the pointer is the primary one.
pub const POINTER_PRIMARY_BIT: i32 = 1 << 10;
/// The pointer types, by their codes: `mouse` 0, `touch` 1, `pen` 2.
pub const POINTER_TYPES: [&str; 3] = ["mouse", "touch", "pen"];

/// One thing an input script puts on a tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// An event the step carries.
    Event(Event),
    /// The tab is hidden: this step is the last before the pause.
    Hide,
    /// The tab is shown again.
    Show,
    /// The contexts close after this tick, and the run ends.
    Close,
    /// A watched thing was painted.
    Painted(i32),
    /// A thing's cycle ended.
    CycleEnded(i32),
    /// A voice's cue or end.
    Sound(SoundReport),
    /// A bound the decoder held the script to.
    Fault(FaultReport),
    /// A tracked drawing's bounds.
    Bounds(Bounds),
}

impl Item {
    /// Whether it is the viewer pressing a key or the pointer: what lets
    /// a tab's sound be heard from then on (section 3.5).
    #[must_use]
    pub fn is_press(&self) -> bool {
        match self {
            Self::Event(event) if event.kind == EventKind::KeyDown => true,
            Self::Event(event) if event.kind == EventKind::Pointer => {
                event.numbers.first().is_some_and(|first| first & POINTER_ACTION_MASK == PointerAction::DOWN.number())
            }
            _ => false,
        }
    }
}

/// The bits of a pointer event's first number that hold its action.
const POINTER_ACTION_MASK: i32 = (1 << POINTER_BUTTON_SHIFT) - 1;

/// An input script, read: its items in the order written, each with its
/// tick, the ticks never going down.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputScript {
    /// The items.
    pub items: Vec<(u64, Item)>,
}

impl InputScript {
    /// The items of `tick`, in the order written.
    pub fn at(&self, tick: u64) -> impl Iterator<Item = &Item> {
        let start = self.items.partition_point(|(at, _)| *at < tick);
        self.items.iter().skip(start).take_while(move |(at, _)| *at == tick).map(|(_, item)| item)
    }
}

fn whole(word: &str, what: &str) -> Result<i32, String> {
    word.parse::<i32>().map_err(|_| format!("{what} `{word}` is not a whole number"))
}

fn text_event(kind: EventKind, numbers: [i32; 4], text: &str) -> Event {
    Event { kind, numbers, payload: text.as_bytes().to_vec() }
}

/// A key line's name and modifier bits; `last` names the one word a
/// `keydown` (`repeat`) or a `keyup` (`lost`) may add.
fn key(words: &[&str], last: Option<&str>) -> Result<(String, i32), String> {
    let (name, modifiers) = words.split_first().ok_or("a key line names its key")?;
    if name.is_empty() || name.len() > MAX_KEY_NAME_BYTES {
        return Err(format!("MAX_KEY_NAME_BYTES: the key name `{name}` is not 1 to {MAX_KEY_NAME_BYTES} bytes"));
    }
    let mut bits = 0;
    for word in modifiers {
        bits |= match *word {
            "shift" => KeyBit::SHIFT.number(),
            "ctrl" => KeyBit::CTRL.number(),
            "alt" => KeyBit::ALT.number(),
            "meta" => KeyBit::META.number(),
            other if Some(other) == last => KeyBit::REPEAT_OR_LOST.number(),
            other => return Err(format!("`{other}` is not a modifier this key line takes")),
        };
    }
    Ok(((*name).to_owned(), bits))
}

/// The text of a `text` line, its escapes read.
fn unescaped(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        out.push(match characters.next() {
            Some('n') => '\n',
            Some('t') => '\t',
            Some('\\') => '\\',
            _ => return Err("a `\\` in text is followed by `n`, `t` or `\\`".into()),
        });
    }
    Ok(out)
}

fn handles(list: &str) -> Result<Vec<i32>, String> {
    list.split(',').map(|word| whole(word, "the handle")).collect()
}

fn pointer(words: &[&str]) -> Result<Event, String> {
    let [action, x, y, options @ ..] = words else { return Err("a pointer line is `pointer ACTION X Y` and its options".into()) };
    let action = match *action {
        "down" => PointerAction::DOWN,
        "up" => PointerAction::UP,
        "move" => PointerAction::MOVE,
        "enter" => PointerAction::ENTER,
        "leave" => PointerAction::LEAVE,
        "cancel" => PointerAction::CANCEL,
        other => return Err(format!("`{other}` is not a pointer action: down, up, move, enter, leave or cancel")),
    };
    let (mut button, mut buttons, mut kind, mut id, mut primary, mut thing, mut local, mut under) = (0, 0, 0, 1, true, 0, (0, 0), Vec::new());
    for option in options {
        match option.split_once('=') {
            None if *option == "secondary" => primary = false,
            Some(("button", value)) => {
                button = whole(value, "the button")?;
                if !(0..=MAX_POINTER_BUTTON).contains(&button) {
                    return Err(format!("the button {button} is not 0 to {MAX_POINTER_BUTTON}"));
                }
            }
            Some(("buttons", value)) => buttons = whole(value, "the buttons")?,
            Some(("type", value)) => {
                let code =
                    POINTER_TYPES.iter().position(|name| *name == value).ok_or_else(|| format!("`{value}` is not a pointer type: mouse, touch or pen"))?;
                kind = i32::try_from(code).unwrap_or(0);
            }
            Some(("id", value)) => id = whole(value, "the pointer id")?,
            Some(("thing", value)) => thing = whole(value, "the thing's handle")?,
            Some(("local", value)) => {
                let (local_x, local_y) = value.split_once(',').ok_or("`local=` takes X,Y")?;
                local = (whole(local_x, "the local x")?, whole(local_y, "the local y")?);
            }
            Some(("under", value)) => {
                under = handles(value)?;
                if under.len() > MAX_POINTER_OBJECTS {
                    return Err(format!("MAX_POINTER_OBJECTS: `under=` names more than {MAX_POINTER_OBJECTS} things"));
                }
            }
            _ => return Err(format!("`{option}` is not an option of a pointer line")),
        }
    }
    let first = action.number() | (button << POINTER_BUTTON_SHIFT) | (kind << POINTER_TYPE_SHIFT) | if primary { POINTER_PRIMARY_BIT } else { 0 };
    let ints = [id, buttons, local.0, local.1].into_iter().chain(under);
    Ok(Event {
        kind: EventKind::Pointer,
        numbers: [first, whole(x, "the pointer's x")?, whole(y, "the pointer's y")?, thing],
        payload: ints.flat_map(i32::to_be_bytes).collect(),
    })
}

fn voice(word: &str) -> Result<u8, String> {
    word.parse::<u8>().map_err(|_| format!("the voice `{word}` is not a small whole number"))
}

/// The items of one line's kind and fields.
fn items(kind: &str, rest: &str) -> Result<Vec<Item>, String> {
    let words: Vec<&str> = rest.split_ascii_whitespace().collect();
    let event = |event: Event| Ok(vec![Item::Event(event)]);
    let plain = |kind: EventKind| Event { kind, numbers: [0; 4], payload: Vec::new() };
    let none = |item: Item| if words.is_empty() { Ok(vec![item]) } else { Err(format!("`{kind}` takes no fields")) };
    match kind {
        "keydown" => {
            let (name, bits) = key(&words, Some("repeat"))?;
            event(text_event(EventKind::KeyDown, [bits, 0, 0, 0], &name))
        }
        "keyup" => {
            let (name, bits) = key(&words, Some("lost"))?;
            event(text_event(EventKind::KeyUp, [bits, 0, 0, 0], &name))
        }
        "key" => {
            let (name, bits) = key(&words, None)?;
            Ok(vec![Item::Event(text_event(EventKind::KeyDown, [bits, 0, 0, 0], &name)), Item::Event(text_event(EventKind::KeyUp, [bits, 0, 0, 0], &name))])
        }
        "text" => {
            let text = unescaped(rest)?;
            if text.is_empty() || text.len() > MAX_TEXT_EVENT_BYTES {
                return Err(format!("MAX_TEXT_EVENT_BYTES: the text is not 1 to {MAX_TEXT_EVENT_BYTES} bytes"));
            }
            event(text_event(EventKind::Text, [0; 4], &text))
        }
        "pointer" => event(pointer(&words)?),
        "wheel" => {
            let (delta_x, delta_y, thing) = match words.as_slice() {
                [delta_x, delta_y] => (delta_x, delta_y, 0),
                [delta_x, delta_y, option] => {
                    (delta_x, delta_y, whole(option.strip_prefix("thing=").ok_or("a wheel line's option is `thing=H`")?, "the thing's handle")?)
                }
                _ => return Err("a wheel line is `wheel DX DY [thing=H]`".into()),
            };
            event(Event {
                kind: EventKind::Wheel,
                numbers: [whole(delta_x, "the wheel's x")?, whole(delta_y, "the wheel's y")?, thing, 0],
                payload: Vec::new(),
            })
        }
        "focus" => none(Item::Event(plain(EventKind::Focus))),
        "blur" => none(Item::Event(plain(EventKind::Blur))),
        "hide" => none(Item::Hide),
        "show" => none(Item::Show),
        "close" => none(Item::Close),
        "navigate" => {
            let [cause, destination] = words.as_slice() else { return Err("a navigate line is `navigate CAUSE DESTINATION`".into()) };
            let cause = match *cause {
                "script" => NavigateCause::SCRIPT,
                "link" => NavigateCause::LINK,
                "key" => NavigateCause::KEY,
                "browser" => NavigateCause::BROWSER,
                other => return Err(format!("`{other}` is not a cause: script, link, key or browser")),
            };
            if destination.len() > MAX_URL_BYTES {
                return Err(format!("MAX_URL_BYTES: the destination is longer than {MAX_URL_BYTES} bytes"));
            }
            event(text_event(EventKind::Navigate, [cause.number(), 0, 0, 0], destination))
        }
        "ui" => {
            let (handle, after) = rest.split_once(' ').unwrap_or((rest, ""));
            let (what, value) = after.trim_start().split_once(' ').unwrap_or((after.trim_start(), ""));
            if value.len() > MAX_EVENT_PAYLOAD_BYTES {
                return Err(format!("MAX_EVENT_PAYLOAD_BYTES: the value is longer than {MAX_EVENT_PAYLOAD_BYTES} bytes"));
            }
            event(text_event(EventKind::Ui, [whole(handle, "the component's handle")?, whole(what, "the component event's number")?, 0, 0], value.trim()))
        }
        "painted" | "cycleend" => {
            let [handle] = words.as_slice() else { return Err(format!("a {kind} line is `{kind} H`")) };
            let handle = whole(handle, "the thing's handle")?;
            Ok(vec![if kind == "painted" { Item::Painted(handle) } else { Item::CycleEnded(handle) }])
        }
        "cue" => {
            let [voice_word, cue] = words.as_slice() else { return Err("a cue line is `cue VOICE CUE`".into()) };
            Ok(vec![Item::Sound(SoundReport::Cue { voice: voice(voice_word)?, cue: whole(cue, "the cue")? })])
        }
        "ended" => {
            let [voice_word] = words.as_slice() else { return Err("an ended line is `ended VOICE`".into()) };
            Ok(vec![Item::Sound(SoundReport::Ended { voice: voice(voice_word)? })])
        }
        "fault" => {
            let [code, bound] = words.as_slice() else { return Err("a fault line is `fault CODE BOUND`".into()) };
            let named = |byte: u8| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_';
            if bound.len() > MAX_BOUND_NAME_BYTES || !bound.bytes().all(named) {
                return Err(format!("the bound `{bound}` is not a name in capitals of at most {MAX_BOUND_NAME_BYTES} bytes"));
            }
            Ok(vec![Item::Fault(FaultReport { code: whole(code, "the fault's code")?, bound: (*bound).to_owned() })])
        }
        "bounds" => {
            let [handle, x, y, width, height] = words.as_slice() else { return Err("a bounds line is `bounds H X Y WIDTH HEIGHT`".into()) };
            Ok(vec![Item::Bounds(Bounds {
                handle: whole(handle, "the drawing's handle")?,
                x: whole(x, "x")?,
                y: whole(y, "y")?,
                width: whole(width, "the width")?,
                height: whole(height, "the height")?,
            })])
        }
        other => Err(format!("`{other}` is not a kind of input line")),
    }
}

/// Reads an input script.
///
/// # Errors
/// The first line that is wrong, by its number, and why.
pub fn parse(bytes: &[u8]) -> Result<InputScript, String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!("the input script is longer than {MAX_INPUT_BYTES} bytes"));
    }
    let text = core::str::from_utf8(bytes).map_err(|_| "the input script is not UTF-8")?;
    let mut script = InputScript::default();
    let mut last = 0_u64;
    for (index, line) in text.lines().enumerate() {
        let number = index.saturating_add(1);
        let fail = |why: String| format!("line {number} of the input script: {why}");
        if line.len() > MAX_INPUT_LINE_BYTES {
            return Err(fail(format!("longer than {MAX_INPUT_LINE_BYTES} bytes")));
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (tick, after) = line.split_once(' ').ok_or_else(|| fail("a line is `TICK kind fields`".into()))?;
        let tick = tick
            .parse::<u64>()
            .ok()
            .filter(|tick| *tick < MAX_TICKS)
            .ok_or_else(|| fail(format!("`{tick}` is not a tick from 0 to {}", MAX_TICKS.saturating_sub(1))))?;
        if tick < last {
            return Err(fail(format!("tick {tick} comes after tick {last}: the ticks never go down")));
        }
        last = tick;
        let (kind, rest) = after.split_once(' ').unwrap_or((after, ""));
        for item in items(kind, rest).map_err(fail)? {
            if script.items.len() >= MAX_INPUT_EVENTS {
                return Err(fail(format!("more than {MAX_INPUT_EVENTS} events")));
            }
            script.items.push((tick, item));
        }
    }
    Ok(script)
}

//! The host boundary's numbers (script-api.md sections 10.2 to 10.4):
//! every operation a script may ask of its host and every event the host
//! gives a context, each defined once, here. The prelude (`prelude.js`) is
//! given the same numbers by [`prelude`], which writes them in front of
//! it, so the two halves cannot disagree. A number is never reused; a
//! change is a new API version.

/// Defines a set of numbers: each name and number once.
macro_rules! numbered {
    ($(#[$meta:meta])* $kind:ident: $($(#[$doc:meta])* $name:ident = $number:literal,)*) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(i32)]
        #[allow(non_camel_case_types, reason = "each name is the prelude's JavaScript constant, written as JavaScript writes constants")]
        pub enum $kind {
            $(
                $(#[$doc])*
                $name = $number,
            )*
        }

        impl $kind {
            /// Every one, with its name.
            pub const ALL: &[(&str, $kind)] = &[$((stringify!($name), $kind::$name),)*];

            /// The one numbered `number`.
            #[must_use]
            pub fn from_number(number: i32) -> Option<Self> {
                Self::ALL.iter().find(|(_, one)| *one as i32 == number).map(|(_, one)| *one)
            }

            /// Its number.
            #[must_use]
            pub const fn number(self) -> i32 {
                self as i32
            }
        }
    };
}

numbered! {
    /// An operation a script asks of its host (section 10.2): the shared
    /// core's, 1 to 21, and sound and synthesizers, 30 to 35.
    Op:
    /// A log line: a level 0 to 4 and its text.
    LOG = 1,
    /// The context's tick count.
    NOW = 2,
    /// A field of `nt.page`, `nt.channel`, `nt.level` or `nt.version`.
    INFO = 3,
    /// Sets a timer: ticks and whether it repeats; gives its number.
    TIMER = 4,
    /// Cancels a timer.
    CANCEL = 5,
    /// The budget report, as JSON text the host writes.
    BUDGET = 6,
    /// Whether a claimed key is down (0) or went down this step (1).
    KEY_STATE = 7,
    /// The pointer as of this step: text `x,y,buttons,inside,h`.
    POINTER_STATE = 8,
    /// Sets the page's title.
    TITLE = 9,
    /// Asks the browser to go to another page.
    NAVIGATE = 10,
    /// Begins a scope; gives its number.
    SCOPE_BEGIN = 11,
    /// Stops assigning new things to a scope.
    SCOPE_LEAVE = 12,
    /// Removes everything in a scope.
    SCOPE_END = 13,
    /// A value of the shared store, or undefined.
    SHARED_GET = 14,
    /// Sets a value of the shared store: the key's length, then the key and the JSON text.
    SHARED_SET = 15,
    /// Deletes a key of the shared store.
    SHARED_DELETE = 16,
    /// The shared store's keys, as JSON text.
    SHARED_KEYS = 17,
    /// Random bytes from the context's cryptographic generator.
    RANDOM = 18,
    /// The time: text, milliseconds since the epoch, to the tick.
    CLOCK = 19,
    /// Minutes east of UTC.
    ZONE = 20,
    /// Tells the host a thing has, or no longer has, a listener for a kind of event.
    LISTEN = 21,
    /// Plays one of the channel's sounds.
    SOUND_PLAY = 30,
    /// Stops what a voice plays.
    SOUND_STOP = 31,
    /// Starts a synthesizer on a part.
    SYNTH_START = 32,
    /// A MIDI channel message for a part.
    SYNTH_MIDI = 33,
    /// A note held for a number of ticks.
    SYNTH_NOTE = 34,
    /// All notes off and the instrument released.
    SYNTH_STOP = 35,
}

numbered! {
    /// An event the host gives a context (section 10.3), 1 to 21, and
    /// `SOUND_ALLOWED`, which section 3.5's `allowed` event needs and
    /// section 10.3 does not number (this session's choice).
    EventKind:
    /// A presentation tick the manifest asked for: the tick and how many were missed.
    TICK = 1,
    /// A claimed key went down: modifiers and repeat; the key's name.
    KEY_DOWN = 2,
    /// A claimed key came up: modifiers and lost; the key's name.
    KEY_UP = 3,
    /// Committed text.
    TEXT = 4,
    /// The pointer: action, button and type packed; x; y; the thing's handle.
    POINTER = 5,
    /// The wheel: delta x, delta y, the thing's handle.
    WHEEL = 6,
    /// The page took the keyboard.
    FOCUS = 7,
    /// The page lost the keyboard.
    BLUR = 8,
    /// The tab was hidden or shown.
    VISIBILITY = 9,
    /// A timer is due.
    TIMER = 10,
    /// The browser is about to go to another page.
    NAVIGATE = 11,
    /// A page is shown (the channel context only).
    PAGE_SHOW = 12,
    /// The page shown is left (the channel context only).
    PAGE_HIDE = 13,
    /// The context is closing.
    CLOSE = 14,
    /// The other context changed a key of the shared store.
    SHARED = 15,
    /// A watched thing was painted.
    PAINTED = 16,
    /// A thing's `once` cycle ended.
    CYCLE_END = 17,
    /// A cue point in a playing sound.
    SOUND_CUE = 18,
    /// A sound ended.
    SOUND_ENDED = 19,
    /// A renderer's fault.
    FAULT = 20,
    /// A component's event.
    UI = 21,
    /// The viewer has pressed a key or the pointer in this tab, so sound is allowed.
    SOUND_ALLOWED = 22,
}

impl EventKind {
    /// The kind a step's event names. Every kind a step can carry has its
    /// number here, the same number (the crate's tests compare them);
    /// event 22 is the host's own and no step carries it.
    #[must_use]
    pub const fn of(kind: protocol::script::EventKind) -> Self {
        use protocol::script::EventKind as Carried;
        match kind {
            Carried::Tick => Self::TICK,
            Carried::KeyDown => Self::KEY_DOWN,
            Carried::KeyUp => Self::KEY_UP,
            Carried::Text => Self::TEXT,
            Carried::Pointer => Self::POINTER,
            Carried::Wheel => Self::WHEEL,
            Carried::Focus => Self::FOCUS,
            Carried::Blur => Self::BLUR,
            Carried::Visibility => Self::VISIBILITY,
            Carried::Timer => Self::TIMER,
            Carried::Navigate => Self::NAVIGATE,
            Carried::PageShow => Self::PAGE_SHOW,
            Carried::PageHide => Self::PAGE_HIDE,
            Carried::Close => Self::CLOSE,
            Carried::Shared => Self::SHARED,
            Carried::Painted => Self::PAINTED,
            Carried::CycleEnd => Self::CYCLE_END,
            Carried::SoundCue => Self::SOUND_CUE,
            Carried::SoundEnded => Self::SOUND_ENDED,
            Carried::Fault => Self::FAULT,
            Carried::Ui => Self::UI,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two crates number the events of section 10.3 alike: each kind
    /// a step can carry has the number the script library gives it, and
    /// the library's one further kind, `SOUND_ALLOWED`, is the first
    /// number after them, which the message's reader refuses.
    #[test]
    fn a_steps_event_kinds_are_the_librarys_numbers() {
        use protocol::script::EventKind as Carried;
        for carried in Carried::ALL {
            assert_eq!(EventKind::of(*carried).number(), i32::from(carried.code()), "{carried:?}");
        }
        let own: Vec<EventKind> =
            EventKind::ALL.iter().map(|(_, kind)| *kind).filter(|kind| !Carried::ALL.iter().any(|carried| EventKind::of(*carried) == *kind)).collect();
        assert_eq!(own, vec![EventKind::SOUND_ALLOWED]);
        assert!(Carried::of(u8::try_from(EventKind::SOUND_ALLOWED.number()).unwrap()).is_err());
    }

    /// The families a script may start are the ones the batch's reader
    /// takes: the rule table's first to its last.
    #[test]
    fn the_families_are_the_batchs() {
        let codes: Vec<u8> = FAMILIES.iter().map(|(_, codec)| codec.code()).collect();
        assert_eq!(codes, (neoteletext_rules::limits::FIRST_FAMILY..=neoteletext_rules::limits::LAST_FAMILY).collect::<Vec<u8>>());
    }
}

numbered! {
    /// A field operation `INFO` gives (this session's numbering; section
    /// 10.2 names the fields without numbering them).
    Info:
    /// The host's highest API version.
    VERSION = 1,
    /// The context's level: 1 the channel's, 2 the page's.
    LEVEL = 2,
    /// The channel's address.
    CHANNEL_URL = 3,
    /// The channel's title.
    CHANNEL_TITLE = 4,
    /// The page's number, three hexadecimal digits.
    PAGE_NUMBER = 5,
    /// The page's sub-page, four hexadecimal digits, or undefined.
    PAGE_SUBPAGE = 6,
    /// The page file's address.
    PAGE_URL = 7,
    /// The page's renderer: `teletext`, `regis` or `naplps`.
    PAGE_RENDERER = 8,
    /// The page's title.
    PAGE_TITLE = 9,
    /// Whether the tab is hidden.
    HIDDEN = 10,
    /// Whether sound is allowed yet.
    SOUND_ALLOWED = 11,
    /// The API version the script's manifest names.
    API = 12,
    /// The script's own address.
    SCRIPT_URL = 13,
    /// The language text is encoded for.
    LANGUAGE = 14,
    /// The time the context started: text, milliseconds since the epoch.
    ORIGIN = 15,
}

numbered! {
    /// A property of a thing (section 10.4).
    Property:
    /// Its horizontal position.
    X = 1,
    /// Its vertical position.
    Y = 2,
    /// Whether it is shown.
    VISIBLE = 3,
    /// Its depth among its siblings.
    DEPTH = 4,
    /// Its layer.
    LAYER = 5,
    /// Its parent.
    PARENT = 6,
    /// What it draws.
    SOURCE = 7,
    /// Whether the pointer can hit it.
    DETECTABLE = 8,
    /// Its paint rate.
    PAINT_RATE = 9,
    /// Its look under the pointer.
    HOVER = 10,
    /// Its look while pressed.
    PRESSED = 11,
    /// Its alternative text.
    ALTERNATIVE = 12,
    /// Whether it takes the wheel.
    WHEEL = 13,
    /// Whether its bounds are reported.
    TRACK = 14,
}

numbered! {
    /// What a pointer event says the pointer did, in the low four bits of
    /// its first whole number (this session's packing; section 10.3 says
    /// only "action, button and type packed"): bits 4 to 7 hold the
    /// button, bits 8 and 9 the pointer's type (0 mouse, 1 touch, 2 pen)
    /// and bit 10 whether it is the primary pointer.
    PointerAction:
    /// A button or contact went down.
    DOWN = 1,
    /// It came up.
    UP = 2,
    /// The pointer moved.
    MOVE = 3,
    /// It entered a thing.
    ENTER = 4,
    /// It left a thing.
    LEAVE = 5,
    /// The press was taken away.
    CANCEL = 6,
}

numbered! {
    /// The bits of a key event's first whole number (this session's
    /// packing; section 10.3 says "modifiers and repeat" and "modifiers
    /// and lost").
    KeyBit:
    /// Shift is held.
    SHIFT = 1,
    /// Control is held.
    CTRL = 2,
    /// Alt is held.
    ALT = 4,
    /// Meta is held.
    META = 8,
    /// `KEY_DOWN`: the key repeats. `KEY_UP`: the release was not seen.
    REPEAT_OR_LOST = 16,
}

numbered! {
    /// The bits of what the prelude's dispatch function returns for an
    /// event (section 10.3: bit 0 is "the default was cancelled").
    DispatchBit:
    /// The default action was cancelled.
    CANCELLED = 1,
    /// A listener was called.
    HANDLED = 2,
    /// A listener threw.
    THREW = 4,
}

numbered! {
    /// What a `navigate` event says caused it.
    NavigateCause:
    /// A script's `nt.navigate`.
    SCRIPT = 1,
    /// A link of the page.
    LINK = 2,
    /// A key the page claimed.
    KEY = 3,
    /// The browser's own controls: never cancelable.
    BROWSER = 4,
}

numbered! {
    /// The handles of the two things every context has without making
    /// them: what operation `LISTEN` names as the target of the shared
    /// core's own events. Things the renderers add take positive numbers.
    Handle:
    /// `nt` itself.
    NT = 0,
    /// `nt.sound`.
    SOUND = -1,
}

/// The synthesizer families a script may start, each by the name the API
/// gives it and the engine's codec of that number (script-api.md section
/// 3.5; `neoteletext-audio/src/codec.rs`).
pub const FAMILIES: [(&str, neoteletext_audio::Codec); 7] = [
    ("sid", neoteletext_audio::Codec::Synth6581),
    ("dx7", neoteletext_audio::Codec::Synth7),
    ("juno", neoteletext_audio::Codec::Synth106),
    ("synthd", neoteletext_audio::Codec::SynthD),
    ("synth53", neoteletext_audio::Codec::Synth53),
    ("synth8", neoteletext_audio::Codec::Synth8),
    ("synth80", neoteletext_audio::Codec::Synth80),
];

/// The prelude with the numbers and bounds in front of it, as each
/// context's realm runs it before any script.
#[must_use]
pub fn prelude() -> String {
    use core::fmt::Write as _;

    use crate::limits;
    // One function holds the tables and the prelude, so that none of
    // their names is a global of the script's realm.
    let mut out = String::from("(function () {\n\"use strict\";\n");
    let mut table = |name: &str, entries: &mut dyn Iterator<Item = (&str, i64)>| {
        let _ = write!(out, "const {name} = Object.freeze({{");
        for (key, number) in entries {
            let _ = write!(out, "{key}: {number}, ");
        }
        out.push_str("});\n");
    };
    table("OP", &mut Op::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table("EV", &mut EventKind::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table("INFO", &mut Info::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table("PROP", &mut Property::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table("POINTER_ACTION", &mut PointerAction::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table("KEY_BIT", &mut KeyBit::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table("DISPATCH", &mut DispatchBit::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table("CAUSE", &mut NavigateCause::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table("HANDLE", &mut Handle::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table("FAMILY", &mut FAMILIES.iter().map(|(name, codec)| (*name, i64::from(codec.code()))));
    // The bounds the prelude itself needs: to round delays to ticks, to
    // cut a log line before it is sent, and to give the standard's errors
    // where a standard names one.
    let bounds: [(&str, i64); 7] = [
        ("TICKS_PER_SECOND", i64::try_from(limits::TICKS_PER_SECOND).unwrap_or(i64::MAX)),
        ("MAX_TIMER_TICKS", i64::from(limits::MAX_TIMER_TICKS)),
        ("MAX_LOG_CHARS", i64::try_from(limits::MAX_LOG_CHARS).unwrap_or(i64::MAX)),
        ("MAX_RANDOM_BYTES", i64::try_from(limits::MAX_RANDOM_BYTES).unwrap_or(i64::MAX)),
        ("MAX_NOTE_TICKS", i64::from(limits::MAX_NOTE_TICKS)),
        ("MAX_GAIN", i64::from(limits::MAX_GAIN)),
        ("MAX_MIDI_BEND", i64::from(limits::MAX_MIDI_BEND)),
    ];
    table("BOUND", &mut bounds.iter().copied());
    out.push_str(include_str!("prelude.js"));
    out.push_str("\n})();\n");
    out
}

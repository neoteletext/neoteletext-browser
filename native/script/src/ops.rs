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
    /// Parses an address against a base: the serialised address and where its components lie, or undefined.
    URL_PARSE = 40,
    /// Starts a fetch or an event stream: its flags and its body's length; its head. Gives its number.
    FETCH = 41,
    /// The next bytes of a request's body.
    FETCH_BODY = 42,
    /// Gives up a fetch or an event stream.
    FETCH_ABORT = 43,
    /// Makes room for more of an answer's body.
    FETCH_CREDIT = 44,
    /// Opens a socket: its address and subprotocols. Gives its number.
    WS_OPEN = 45,
    /// A piece of a message for a socket.
    WS_SEND = 46,
    /// Closes a socket: its code; its reason.
    WS_CLOSE = 47,
    /// Sets a component of an address, as the URL standard's setter of
    /// that name does (work package W2's number: section 10.2 has none
    /// for the setters, and 48 was free between the web's operations).
    URL_SET = 48,
    /// Decodes bytes in an encoding of the Encoding standard, names a label's encoding, or encodes a string as UTF-8.
    DECODE = 49,
    /// Components: makes one of a kind at an area, with its options as JSON; gives its handle.
    UI_CREATE = 60,
    /// Components: changes a component's options.
    UI_SET = 61,
    /// Components: removes a component.
    UI_REMOVE = 62,
    /// Components: gives a component the focus.
    UI_FOCUS = 63,
    /// Components: the names of the keys a kind uses, as JSON text.
    UI_KEYS = 64,
    /// Components: a component's value, as JSON text.
    UI_VALUE = 65,
    /// Components: offers a key, typed text or the pointer to a component; gives what it would do, as JSON text.
    UI_OFFER = 66,
    /// Components: carries out, or drops, what a component would do.
    UI_SETTLE = 67,
    /// Teletext: text on the page plane, its style's spacing attributes before it.
    TT_PUT = 100,
    /// Teletext: raw seven-bit codes on a row.
    TT_CODES = 101,
    /// Teletext: the whole plane, rows 1 to 23.
    TT_BLIT = 102,
    /// Teletext: the code of a cell of the host's plane.
    TT_AT = 103,
    /// Teletext: fills rows with spaces.
    TT_CLEAR = 104,
    /// Teletext: moves the rows of a band up or down.
    TT_SCROLL = 105,
    /// Teletext: the default screen and row colours and black background substitution.
    TT_SETTINGS = 106,
    /// Teletext: characters with non-spacing attributes on the overlay.
    TT_OVERLAY_PUT = 107,
    /// Teletext: colours a rectangle of the overlay.
    TT_OVERLAY_FILL = 108,
    /// Teletext: removes the overlay, wholly or in a rectangle.
    TT_OVERLAY_CLEAR = 109,
    /// Teletext: defines the next free Colour Map entry of 16 to 31.
    TT_COLOUR_DEFINE = 110,
    /// Teletext: redefines a Colour Map entry.
    TT_COLOUR_SET = 111,
    /// Teletext: makes a sprite of frames given as blocks' cells; gives its handle.
    TT_SPRITE = 112,
    /// Teletext: labels a sprite or a control.
    TT_LABEL = 113,
    /// Teletext: an animation operation on a sprite or a control.
    TT_ANIMATE = 114,
    /// Teletext: removes a sprite or a control.
    TT_REMOVE = 115,
    /// Teletext: declares a control.
    TT_CONTROL = 116,
    /// Teletext: the cells a text takes.
    TT_MEASURE = 117,
    /// Teletext: the languages the engine writes, as JSON text.
    TT_LANGUAGES = 118,
    /// Teletext: the receiver's presentation level.
    TT_LEVEL = 119,
    /// Teletext: a text as the cells of a block, three bytes a cell.
    TT_BLOCK_TEXT = 120,
    /// The teletext sixel bitmap: the rows it owns.
    BM_ROWS = 130,
    /// The sixel bitmap: fills the back buffer and frees its text cells.
    BM_CLEAR = 131,
    /// The sixel bitmap: one sixel of the back buffer.
    BM_SET = 132,
    /// The sixel bitmap: the colour of a sixel of the back buffer.
    BM_GET = 133,
    /// The sixel bitmap: a line.
    BM_LINE = 134,
    /// The sixel bitmap: a rectangle, filled or outlined.
    BM_RECT = 135,
    /// The sixel bitmap: a rectangle of sixels, one byte each.
    BM_BLIT = 136,
    /// The sixel bitmap: cells reserved for characters.
    BM_TEXT = 137,
    /// The sixel bitmap: shows the back buffer; the cells changed plus 65,536 times the cells that lost detail.
    BM_PRESENT = 138,
    /// ReGIS: names a colour and sets it; gives its entry.
    RG_COLOUR_DEFINE = 200,
    /// ReGIS: sets a colour entry.
    RG_COLOUR_SET = 201,
    /// ReGIS: makes a numbered drawing; gives its handle.
    RG_DRAWING = 202,
    /// ReGIS: sets a property of a drawing.
    RG_SET = 203,
    /// ReGIS: sets or removes a drawing's clip rectangle.
    RG_CLIP = 204,
    /// ReGIS: sets or removes a drawing's frame list.
    RG_FRAMES = 205,
    /// ReGIS: sets or removes a drawing's label.
    RG_LABEL = 206,
    /// ReGIS: defines a drawing's contents from a painter program.
    RG_DEFINE = 207,
    /// ReGIS: removes a drawing and its children.
    RG_REMOVE = 208,
    /// ReGIS: removes every drawing of the context.
    RG_CLEAR = 209,
    /// ReGIS: an animation operation on a drawing.
    RG_ANIMATE = 210,
    /// ReGIS: defines a macrograph (work package R2).
    RG_MACROGRAPH = 211,
    /// ReGIS: measures text (work package R2).
    RG_MEASURE = 212,
    /// ReGIS: a tracked drawing's rectangle as of the last step.
    RG_BOUNDS = 213,
    /// ReGIS: a property or other field of a drawing as the host holds it.
    RG_GET = 214,
    /// NAPLPS: defines a colour; its 24 bits and its name; gives its entry.
    NP_COLOUR_DEFINE = 300,
    /// NAPLPS: redefines a colour entry.
    NP_COLOUR_SET = 301,
    /// NAPLPS: an entry blinks to another, or (to -1) is steady.
    NP_BLINK = 302,
    /// NAPLPS: appends a painter program to the backdrop.
    NP_PAINT = 303,
    /// NAPLPS: starts a new generation with a painter program.
    NP_REPAINT = 304,
    /// NAPLPS: defines or redefines a symbol from a painter program; gives its handle.
    NP_SYMBOL = 305,
    /// NAPLPS: defines a DRCS character from a painter program; gives its handle.
    NP_DRCS = 306,
    /// NAPLPS: defines a texture from a painter program; gives its handle.
    NP_TEXTURE = 307,
    /// NAPLPS: makes or redefines an actor or a region; gives its handle.
    NP_ACTOR = 308,
    /// NAPLPS: labels an actor or a region.
    NP_LABEL = 309,
    /// NAPLPS: an animation operation on an actor or a region.
    NP_ANIMATE = 310,
    /// NAPLPS: removes an actor, a region or a symbol.
    NP_REMOVE = 311,
    /// NAPLPS: measures text; gives `width,height` in dots.
    NP_MEASURE = 312,
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
    /// An answer's head: the request, the status, the response type and whether redirected; the address, status text and headers.
    FETCH_HEAD = 30,
    /// A piece of an answer's body.
    FETCH_CHUNK = 31,
    /// An answer's body is complete.
    FETCH_END = 32,
    /// A request, an event stream or a socket failed: which error the script is given.
    FETCH_FAILED = 33,
    /// Bytes of a request's body were written; with no bytes, the request may send more of its body.
    FETCH_SENT = 34,
    /// A socket is open: its subprotocol.
    WS_OPENED = 35,
    /// A piece of a socket's message: binary or text, and whether the last.
    WS_MESSAGE = 36,
    /// A socket closed: its code and whether cleanly; its reason.
    WS_CLOSED = 37,
    /// Bytes of a socket's messages were written.
    WS_SENT = 38,
    /// An event stream is open: its final address, or empty.
    SSE_OPENED = 39,
    /// An event of an event stream: the lengths of its type and of its last event ID; the type, the ID and the data.
    SSE_EVENT = 40,
    /// An event stream was lost and is being reconnected.
    SSE_RETRYING = 41,
    /// ReGIS: the last step's commit failed and its changes were
    /// discarded (section 6.2, "Commit"): the commit error's code and the
    /// drawing concerned; the drawings that are gone and those that are back.
    RG_COMMIT_FAILED = 50,
}

impl EventKind {
    /// Whether the kind is one of the network's (30 to 41), which the
    /// host makes from the web client's answers and no step carries.
    #[must_use]
    pub const fn is_network(self) -> bool {
        self.number() >= Self::FETCH_HEAD.number() && self.number() <= Self::SSE_RETRYING.number()
    }

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
        // The network's events are the host's own too: it makes them from the web client's answers.
        assert_eq!(own.iter().copied().filter(|kind| !kind.is_network()).collect::<Vec<_>>(), vec![EventKind::SOUND_ALLOWED, EventKind::RG_COMMIT_FAILED]);
        assert!(own.iter().filter(|kind| kind.is_network()).all(|kind| Carried::of(u8::try_from(kind.number()).unwrap()).is_err()));
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
    /// What operation `RG_GET` gives of a drawing beside the properties
    /// of section 10.4, which it gives by their own numbers (work package
    /// R1's numbering; the specification has no operation that reads).
    RegisField:
    /// Its clip rectangle: text `x,y,width,height`, or undefined.
    CLIP = 20,
    /// Its frame list: the frames' handles as text with commas between, or undefined.
    FRAMES = 21,
    /// Its label, or undefined.
    LABEL = 22,
    /// The frame last set.
    FRAME = 23,
    /// The look last forced: 0 none, 1 hover, 2 pressed, 3 alternative.
    LOOK = 24,
}

numbered! {
    /// A call of a ReGIS painter program (sections 6.3 and 10.2): one for
    /// each call of section 6.3's table, in its order from 1. Work
    /// package R1 encodes the first eight, `FILL` and `INK`; the rest are
    /// work package R2's.
    RegisPaint:
    /// `moveTo(x, y)`.
    MOVE_TO = 1,
    /// `moveBy(dx, dy)`.
    MOVE_BY = 2,
    /// `lineTo(x, y)`.
    LINE_TO = 3,
    /// `lineBy(dx, dy)`.
    LINE_BY = 4,
    /// `dot()`.
    DOT = 5,
    /// `polyline(points)`: the count of points, then each point's x and y.
    POLYLINE = 6,
    /// `polygon(points)`: as `POLYLINE`.
    POLYGON = 7,
    /// `rect(x, y, width, height)`.
    RECT = 8,
    /// `vector(digits, { multiplier })`.
    VECTOR = 9,
    /// `circle(radius)`.
    CIRCLE = 10,
    /// `circleThrough(x, y)`.
    CIRCLE_THROUGH = 11,
    /// `arc(cx, cy, degrees)`.
    ARC = 12,
    /// `curve(points, { closed })`.
    CURVE = 13,
    /// `fill(fn)`: the length of its body in whole numbers, then the body.
    FILL = 14,
    /// `ink(entry)`.
    INK = 15,
    /// `mode(m)`.
    MODE = 16,
    /// `negative(on)`.
    NEGATIVE = 17,
    /// `pattern(p, { multiplier })`.
    PATTERN = 18,
    /// `planes(mask)`.
    PLANES = 19,
    /// `shade(s)`.
    SHADE = 20,
    /// `text(string, options)`.
    TEXT = 21,
    /// `call(macrograph)`.
    CALL = 22,
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

/// The parts of the prelude beside the shared core. Every context's realm
/// runs them all ([`crate::Container::open`]); a choice of them is for
/// measuring what each costs a context's start (`tests/start.rs`, rdm
/// TODO-7386).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "four independent parts, each named where it is chosen")]
pub struct Parts {
    /// `nt.teletext` with the sixel bitmap and sprites.
    pub teletext: bool,
    /// `nt.regis`.
    pub regis: bool,
    /// `nt.naplps`.
    pub naplps: bool,
    /// The web APIs with the bundled streams.
    pub web: bool,
}

impl Parts {
    /// Every part.
    pub const ALL: Self = Self { teletext: true, regis: true, naplps: true, web: true };
    /// The shared core alone.
    pub const CORE: Self = Self { teletext: false, regis: false, naplps: false, web: false };
}

/// The whole prelude: the shared core and every part.
#[must_use]
pub fn prelude() -> String {
    prelude_of(Parts::ALL)
}

/// The prelude with the numbers and bounds in front of it, as a context's
/// realm runs it before any script: the shared core and the parts named.
/// A part left out leaves in its place only the names the core reads it
/// by, undefined.
#[must_use]
pub fn prelude_of(parts: Parts) -> String {
    use crate::limits;
    // One function holds the tables and the prelude, so that none of
    // their names is a global of the script's realm.
    let mut out = String::from("(function () {\n\"use strict\";\n");
    table(&mut out, "OP", &mut Op::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table(&mut out, "EV", &mut EventKind::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table(&mut out, "INFO", &mut Info::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table(&mut out, "PROP", &mut Property::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table(&mut out, "POINTER_ACTION", &mut PointerAction::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table(&mut out, "KEY_BIT", &mut KeyBit::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table(&mut out, "DISPATCH", &mut DispatchBit::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table(&mut out, "CAUSE", &mut NavigateCause::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table(&mut out, "HANDLE", &mut Handle::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
    table(&mut out, "FAMILY", &mut FAMILIES.iter().map(|(name, codec)| (*name, i64::from(codec.code()))));
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
    table(&mut out, "BOUND", &mut bounds.iter().copied());
    out.push_str(include_str!("prelude.js"));
    if parts.naplps {
        table(&mut out, "NP", &mut crate::host::naplps::program::table());
        table(&mut out, "NP_BOUND", &mut crate::host::naplps::prelude_bounds().into_iter());
        out.push_str(include_str!("host/naplps/prelude.js"));
    } else {
        out.push_str("\nconst naplpsApi = undefined;\n");
    }
    if parts.teletext {
        table(&mut out, "TT", &mut crate::teletext::prelude_numbers().iter().copied());
        table(&mut out, "TTB", &mut crate::teletext::bitmap::prelude_numbers().iter().copied());
        out.push_str(crate::teletext::bitmap::PRELUDE);
        out.push_str(crate::teletext::PRELUDE);
    } else {
        out.push_str("\nconst teletextApi = undefined;\n");
    }
    if parts.regis {
        table(&mut out, "RG_FIELD", &mut RegisField::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
        table(&mut out, "RG_PAINT", &mut RegisPaint::ALL.iter().map(|(name, one)| (*name, i64::from(one.number()))));
        table(&mut out, "RG_BOUND", &mut crate::host::regis_prelude_bounds().into_iter());
        out.push_str(include_str!("regis.js"));
    }
    // The components (work package U1) are the same on every renderer and
    // the core's dispatcher names them: every realm has them.
    table(&mut out, "UI", &mut crate::host::ui_prelude_numbers().into_iter());
    out.push_str(include_str!("ui.js"));
    if parts.web {
        // The web APIs (work package W2), inside the same function.
        crate::web::write_prelude(&mut out);
    } else {
        // No event of the network reaches a realm without them.
        out.push_str("\nfunction webDispatch() { return 0; }\n");
    }
    out.push_str("\n})();\n");
    out
}

/// Writes a table of numbers into the prelude: a frozen object of the
/// name given.
fn table(out: &mut String, name: &str, entries: &mut dyn Iterator<Item = (&str, i64)>) {
    use core::fmt::Write as _;
    let _ = write!(out, "const {name} = Object.freeze({{");
    for (key, number) in entries {
        let _ = write!(out, "{key}: {number}, ");
    }
    out.push_str("});\n");
}

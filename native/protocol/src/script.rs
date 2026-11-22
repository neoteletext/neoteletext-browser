//! Messages between a decoder and its script host (DEC-7266, DEC-7287
//! items 11 and 12; the script API's specification section 11.2): the
//! decoder opens a context, gives it its script's source and steps it,
//! and the script host answers each step with one batch of typed drawing
//! commands for the page that is open.
//!
//! - **A script's output never takes the channel's stream path** (the
//!   security review's finding S-08, TODO-7274). A [`Batch`] holds only
//!   the sections of [`batch`], none of which names a page or a sub-code
//!   and none of which can carry a record or a packet: the decoder builds
//!   those itself, for the open page alone.
//! - **Every field has its bound, by name,** from the specification's
//!   section 10.6. A bound that is the engine's is used by the engine's
//!   name; one this crate cannot reach is asserted equal in the tests.
//! - **Strict both ways.** A message that breaks a bound, has bytes left
//!   over, or names an unknown kind or section is refused whole, and its
//!   reader's caller ends the link: the decoder treats the script as
//!   stopped.
//! - **The trusted core relays** these between the two confined processes
//!   by kind and length alone ([`is_to_host_kind`],
//!   [`is_to_decoder_kind`]); it reads no payload.
//!
//! Integers are big-endian; a text or byte field is a four-byte length and
//! its bytes; a list is a two-byte count and its items.

use std::io::{Read, Write};

use neoteletext_rules::limits::{MAX_EVENTS_PER_TICK, MAX_KEYS_DOWN, TICKS_PER_SECOND};

use crate::coded::coded;
use crate::web::MAX_URL_BYTES;
use crate::{Fields, MAX_PAYLOAD, ProtocolError, push_bytes, push_count, read_frame, write_frame};

pub mod batch;
pub mod teletext;

pub use batch::Batch;

// The script link's kinds: numbers no pipe used on 22 November 2026 and
// none the web client's messages use (`crate::web`), since a script host
// reads and writes both on its one pair of pipes, and a decoder reads and
// writes these among its own.
const KIND_HELLO: u8 = 220;
const KIND_CONTEXT_OPEN: u8 = 221;
const KIND_SOURCE: u8 = 222;
const KIND_STEP: u8 = 223;
const KIND_CONTEXT_CLOSE: u8 = 224;
const KIND_SHUTDOWN: u8 = 225;
const KIND_READY: u8 = 226;
const KIND_BATCH: u8 = 227;
const KIND_FAILED: u8 = 228;

/// Every kind of this module, for the test that no two pipes that share a
/// process confuse a kind.
#[cfg(test)]
pub(crate) const KINDS: &[u8] =
    &[KIND_HELLO, KIND_CONTEXT_OPEN, KIND_SOURCE, KIND_STEP, KIND_CONTEXT_CLOSE, KIND_SHUTDOWN, KIND_READY, KIND_BATCH, KIND_FAILED];

/// The version of these messages. A side that is told another ends the
/// link.
pub const LINK_VERSION: u16 = 1;
/// The longest script, in bytes (the specification's `MAX_SCRIPT_BYTES`).
pub const MAX_SCRIPT_BYTES: usize = 1024 * 1024;
/// The most bytes of a script's source in one message.
pub const MAX_SOURCE_PIECE_BYTES: usize = 128 * 1024;
/// The greatest tick divisor: a tick event a second, at the page's
/// [`TICKS_PER_SECOND`].
pub const MAX_TICK_DIVISOR: u8 = 60;
const _: () = assert!(MAX_TICK_DIVISOR as u32 == TICKS_PER_SECOND);
/// The longest language tag, in bytes.
pub const MAX_LANGUAGE_BYTES: usize = 16;
/// The longest title, a channel's or a page's, in bytes of UTF-8: the
/// renderers' limit.
pub const MAX_TITLE_BYTES: usize = 64;
/// The longest label of a thing, in bytes of UTF-8: the renderers' limit.
pub const MAX_LABEL_BYTES: usize = 64;
/// The bytes of the seed a context's random numbers start from.
pub const SEED_BYTES: usize = 32;
/// The farthest zone from UTC either way, in minutes: less than a day.
pub const MAX_ZONE_MINUTES: i16 = 24 * 60 - 1;
/// The most keys a script claims: the bound of a page's key asks.
pub const MAX_KEY_ASKS: usize = crate::rules::MAX_KEY_ASKS;
/// The longest key name, in bytes.
pub const MAX_KEY_NAME_BYTES: usize = 32;
/// The most sound handles a context is given: every sound number.
pub const MAX_SOUND_HANDLES: usize = 255;
/// The most events one step carries: a tick's events, and the release of
/// every key that was down.
pub const MAX_STEP_EVENTS: usize = MAX_EVENTS_PER_TICK + MAX_KEYS_DOWN;
/// The longest payload of an event, in bytes: the longest text the input
/// stream carries at once (the specification's `MAX_TEXT_EVENT_BYTES`).
pub const MAX_EVENT_PAYLOAD_BYTES: usize = 4096;
/// The most drawings whose painting is watched, and so the most a step
/// reports painted: the ReGIS renderer's `MAX_WATCHED_PAINTINGS`, asserted
/// equal in the tests.
pub const MAX_WATCHED_PAINTINGS: usize = 32;
/// The most cycles that end in one step: every object that may cycle.
pub const MAX_CYCLE_ENDS: usize = neoteletext_core::limits::MAX_CYCLING_OBJECTS;
/// The most drawings whose bounds a step reports.
pub const MAX_TRACKED_DRAWINGS: usize = 64;
/// The most sound cues and ends a step reports.
pub const MAX_STEP_SOUND_REPORTS: usize = 8;
/// The most faults a step reports.
pub const MAX_STEP_FAULTS: usize = 8;
/// The longest name of a bound, in bytes.
pub const MAX_BOUND_NAME_BYTES: usize = 64;
/// The most lines of the developer log in one step.
pub const MAX_LOG_LINES: usize = 64;
/// The longest line of the developer log, in characters.
pub const MAX_LOG_CHARS: usize = 400;
/// The most bytes of UTF-8 a character takes.
const UTF8_MAX: usize = 4;
/// The longest line of the developer log, in bytes.
pub const MAX_LOG_BYTES: usize = MAX_LOG_CHARS * UTF8_MAX;
/// The two contexts of a container: the channel's and the page's.
pub const CONTEXTS: usize = ContextId::ALL.len();

/// The bytes of a context's opening outside its addresses, title, keys
/// and sounds: more than its fixed fields take.
const CONTEXT_OPEN_FIXED: usize = 256;
/// The bytes of an event outside its payload: its kind, four numbers and
/// the payload's length.
const EVENT_FIXED: usize = 1 + 4 * 4 + 4;
/// The bytes of a step outside its events: more than its numbers, its
/// reports at their bounds and its faults take.
const STEP_FIXED: usize = 8 * 1024;

// Every message fits one payload.
const _: () = assert!(CONTEXT_OPEN_FIXED + 3 * MAX_URL_BYTES + MAX_TITLE_BYTES + MAX_KEY_ASKS * (4 + MAX_KEY_NAME_BYTES) + MAX_SOUND_HANDLES <= MAX_PAYLOAD);
const _: () = assert!(MAX_SOURCE_PIECE_BYTES + 16 <= MAX_PAYLOAD);
const _: () = assert!(STEP_FIXED + MAX_STEP_EVENTS * (EVENT_FIXED + MAX_EVENT_PAYLOAD_BYTES) <= MAX_PAYLOAD);
const _: () = assert!(
    4 + MAX_WATCHED_PAINTINGS * 4
        + MAX_CYCLE_ENDS * 4
        + MAX_TRACKED_DRAWINGS * 20
        + MAX_STEP_SOUND_REPORTS * 6
        + MAX_STEP_FAULTS * (8 + MAX_BOUND_NAME_BYTES)
        + 64
        <= STEP_FIXED
);

coded! {
    /// One of a container's two contexts (DEC-7287 item 3).
    ContextId, "context" {
        /// The channel's, which lives while the tab is in the channel.
        Channel = 1,
        /// The page's, which lives while one page or sub-page is shown.
        Page = 2,
    }
}

coded! {
    /// The renderer of the page a context draws on.
    Renderer, "renderer" {
        /// Teletext.
        Teletext = 1,
        /// ReGIS.
        Regis = 2,
        /// NAPLPS.
        Naplps = 3,
    }
}

coded! {
    /// The teletext presentation level in force, as the script API's
    /// `nt.level` gives it: ten times the level.
    Level, "level" {
        /// Level 1.
        One = 10,
        /// Level 1.5.
        OneFive = 15,
        /// Level 2.5.
        TwoFive = 25,
        /// Level 3.5.
        ThreeFive = 35,
    }
}

coded! {
    /// Why a context stopped for good.
    FailReason, "failure reason" {
        /// A time limit passed.
        Time = 1,
        /// A memory limit passed.
        Memory = 2,
        /// The stack limit passed.
        Stack = 3,
        /// The container trapped.
        Trap = 4,
        /// Too many callbacks failed one after another.
        Failures = 5,
        /// The link broke: a message refused, or a version not spoken.
        Link = 6,
    }
}

coded! {
    /// An event given to a context: the kinds of the specification's
    /// section 10.3 that a step carries, by their numbers there. The
    /// network's events (30 to 41) are not among them: they reach a script
    /// host as the web client's messages (`crate::web`). Nor is the
    /// script host's own event 22, which tells a context that sound is
    /// allowed: a step says so with [`StepFlags::sound_allowed`], and the
    /// script host makes the event.
    EventKind, "event kind" {
        /// A tick.
        Tick = 1,
        /// A key went down.
        KeyDown = 2,
        /// A key came up.
        KeyUp = 3,
        /// Text was typed.
        Text = 4,
        /// The pointer.
        Pointer = 5,
        /// The wheel.
        Wheel = 6,
        /// The page took the focus.
        Focus = 7,
        /// The page lost the focus.
        Blur = 8,
        /// The tab was hidden or shown.
        Visibility = 9,
        /// A timer is due.
        Timer = 10,
        /// A navigation is about to happen.
        Navigate = 11,
        /// A page is shown.
        PageShow = 12,
        /// A page is hidden.
        PageHide = 13,
        /// The context is closing.
        Close = 14,
        /// The shared store changed.
        Shared = 15,
        /// A watched thing was painted.
        Painted = 16,
        /// A cycle ended.
        CycleEnd = 17,
        /// A sound reached a cue.
        SoundCue = 18,
        /// A sound ended.
        SoundEnded = 19,
        /// A bound was passed.
        Fault = 20,
        /// A component changed.
        Ui = 21,
    }
}

/// What a context asked for in its manifest and what it is allowed, as
/// the decoder read and decided them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "seven independent flags of one byte on the wire, each named where it is read")]
pub struct ContextFlags {
    /// The pointer's presses.
    pub pointer_press: bool,
    /// The pointer's moves.
    pub pointer_move: bool,
    /// The wheel.
    pub wheel: bool,
    /// Typed text.
    pub text: bool,
    /// The network.
    pub network: bool,
    /// The page was opened from disk.
    pub from_disk: bool,
    /// The viewer has pressed a key or the pointer in this tab, so the
    /// context's sound and synthesizers sound from its top level on (the
    /// specification's section 3.5; DEC-7334 item 17). Until it is so,
    /// they do nothing.
    pub sound_allowed: bool,
}

/// The bits of [`ContextFlags`], in its fields' order from bit 0.
const CONTEXT_FLAG_BITS: u32 = 7;

impl ContextFlags {
    fn list(self) -> [bool; CONTEXT_FLAG_BITS as usize] {
        [self.pointer_press, self.pointer_move, self.wheel, self.text, self.network, self.from_disk, self.sound_allowed]
    }

    fn of(byte: u8) -> Result<Self, ProtocolError> {
        let [pointer_press, pointer_move, wheel, text, network, from_disk, sound_allowed] = unpack(byte, "context flags")?;
        Ok(Self { pointer_press, pointer_move, wheel, text, network, from_disk, sound_allowed })
    }
}

/// What a step is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "five independent flags of one byte on the wire, each named where it is read")]
pub struct StepFlags {
    /// A tick of the page's clock, and not input between ticks.
    pub tick: bool,
    /// The tab is being hidden: the last step until it is shown.
    pub pausing: bool,
    /// The tab is shown again.
    pub resumed: bool,
    /// The contexts are closing.
    pub closing: bool,
    /// The viewer has pressed a key or the pointer in this tab: said in
    /// every step from the first after it happened, so that the script
    /// host raises `allowed` on `nt.sound` once and lets sound through
    /// (the specification's section 3.5; DEC-7334 item 17).
    pub sound_allowed: bool,
}

/// The bits of [`StepFlags`], in its fields' order from bit 0.
const STEP_FLAG_BITS: u32 = 5;

impl StepFlags {
    fn list(self) -> [bool; STEP_FLAG_BITS as usize] {
        [self.tick, self.pausing, self.resumed, self.closing, self.sound_allowed]
    }

    fn of(byte: u8) -> Result<Self, ProtocolError> {
        let [tick, pausing, resumed, closing, sound_allowed] = unpack(byte, "step flags")?;
        Ok(Self { tick, pausing, resumed, closing, sound_allowed })
    }
}

/// Flags as one byte, the first at bit 0.
pub(crate) fn pack<const N: usize>(flags: [bool; N]) -> u8 {
    flags.iter().rev().fold(0_u8, |byte, &flag| (byte << 1) | u8::from(flag))
}

/// The `N` flags of `byte`, the first at bit 0; `what` names a byte with
/// any other bit set.
pub(crate) fn unpack<const N: usize>(byte: u8, what: &'static str) -> Result<[bool; N], ProtocolError> {
    let mut flags = [false; N];
    let mut rest = byte;
    for flag in &mut flags {
        *flag = rest & 1 == 1;
        rest >>= 1;
    }
    if rest == 0 { Ok(flags) } else { Err(ProtocolError::BadValue(what)) }
}

/// Whether `text` is plain: no control character, so that it can be
/// shown, logged or spoken as it stands.
pub(crate) fn is_plain(text: &str) -> bool {
    !text.chars().any(char::is_control)
}

/// Holds `text` to at most `max` bytes of plain text.
pub(crate) fn check_plain(text: &str, max: usize, what: &'static str) -> Result<(), ProtocolError> {
    if text.len() <= max && is_plain(text) { Ok(()) } else { Err(ProtocolError::BadValue(what)) }
}

/// A line of the developer log: plain, and at most [`MAX_LOG_CHARS`].
pub(crate) fn check_log(text: &str) -> Result<(), ProtocolError> {
    if text.len() <= MAX_LOG_BYTES && is_plain(text) && text.chars().count() <= MAX_LOG_CHARS { Ok(()) } else { Err(ProtocolError::BadValue("log line")) }
}

/// An address: visible ASCII of at most [`MAX_URL_BYTES`], as the URL
/// standard writes one.
pub(crate) fn check_address(text: &str, what: &'static str) -> Result<(), ProtocolError> {
    if text.len() <= MAX_URL_BYTES && text.bytes().all(|byte| byte.is_ascii_graphic()) { Ok(()) } else { Err(ProtocolError::BadValue(what)) }
}

/// A context's opening: what `nt` tells its script of where it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextOpen {
    /// Which context.
    pub context: ContextId,
    /// The API version the script's manifest names.
    pub api: u16,
    /// The renderer of the page shown.
    pub renderer: Renderer,
    /// The presentation level in force.
    pub level: Level,
    /// The page shown, a page a channel may have.
    pub page: u16,
    /// Its sub-code, at most the last a sub-page has.
    pub subcode: u16,
    /// What the context asked for and is allowed.
    pub flags: ContextFlags,
    /// How many of the page's ticks make one tick event, at most
    /// [`MAX_TICK_DIVISOR`]; 0 for no tick events.
    pub tick_divisor: u8,
    /// The page's language: letters, digits and hyphens, at most
    /// [`MAX_LANGUAGE_BYTES`]; empty for none.
    pub language: String,
    /// The channel's address.
    pub channel_url: String,
    /// The page's address.
    pub page_url: String,
    /// The script's address.
    pub script_url: String,
    /// The channel's title, plain text of at most [`MAX_TITLE_BYTES`].
    pub channel_title: String,
    /// What the context's random numbers start from.
    pub seed: [u8; SEED_BYTES],
    /// The time, in seconds since the epoch.
    pub epoch_seconds: u64,
    /// The zone, in minutes east of UTC, within [`MAX_ZONE_MINUTES`].
    pub zone_minutes: i16,
    /// The keys the script claims, at most [`MAX_KEY_ASKS`] names of at
    /// most [`MAX_KEY_NAME_BYTES`].
    pub keys: Vec<String>,
    /// The sounds the page has, by handle, at most [`MAX_SOUND_HANDLES`].
    pub sounds: Vec<u8>,
    /// The length of the script's source, which follows in
    /// [`ToScriptHost::Source`] messages: 1 to [`MAX_SCRIPT_BYTES`].
    pub source_bytes: u32,
}

impl ContextOpen {
    /// Whether every field keeps to its bounds.
    ///
    /// # Errors
    /// [`ProtocolError::BadValue`] naming the first that does not.
    pub fn check(&self) -> Result<(), ProtocolError> {
        crate::fields::check_page(self.page)?;
        if self.subcode > neoteletext_rules::limits::LAST_SUBCODE {
            return Err(ProtocolError::BadValue("sub-code"));
        }
        if self.tick_divisor > MAX_TICK_DIVISOR {
            return Err(ProtocolError::BadValue("tick divisor"));
        }
        if self.language.len() > MAX_LANGUAGE_BYTES || !self.language.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') {
            return Err(ProtocolError::BadValue("language"));
        }
        check_address(&self.channel_url, "channel address")?;
        check_address(&self.page_url, "page address")?;
        check_address(&self.script_url, "script address")?;
        check_plain(&self.channel_title, MAX_TITLE_BYTES, "channel title")?;
        if !(-MAX_ZONE_MINUTES..=MAX_ZONE_MINUTES).contains(&self.zone_minutes) {
            return Err(ProtocolError::BadValue("zone"));
        }
        if self.keys.len() > MAX_KEY_ASKS {
            return Err(ProtocolError::BadValue("too many keys"));
        }
        for key in &self.keys {
            if key.is_empty() {
                return Err(ProtocolError::BadValue("key name"));
            }
            check_plain(key, MAX_KEY_NAME_BYTES, "key name")?;
        }
        if self.sounds.len() > MAX_SOUND_HANDLES {
            return Err(ProtocolError::BadValue("too many sounds"));
        }
        check_source_length(self.source_bytes)
    }

    fn write(&self, payload: &mut Vec<u8>) -> Result<(), ProtocolError> {
        self.check()?;
        payload.push(self.context.code());
        payload.extend_from_slice(&self.api.to_be_bytes());
        payload.extend_from_slice(&[self.renderer.code(), self.level.code()]);
        payload.extend_from_slice(&self.page.to_be_bytes());
        payload.extend_from_slice(&self.subcode.to_be_bytes());
        payload.extend_from_slice(&[pack(self.flags.list()), self.tick_divisor]);
        push_bytes(payload, self.language.as_bytes())?;
        push_bytes(payload, self.channel_url.as_bytes())?;
        push_bytes(payload, self.page_url.as_bytes())?;
        push_bytes(payload, self.script_url.as_bytes())?;
        push_bytes(payload, self.channel_title.as_bytes())?;
        payload.extend_from_slice(&self.seed);
        payload.extend_from_slice(&self.epoch_seconds.to_be_bytes());
        payload.extend_from_slice(&self.zone_minutes.to_be_bytes());
        push_count(payload, self.keys.len(), MAX_KEY_ASKS, "too many keys")?;
        for key in &self.keys {
            push_bytes(payload, key.as_bytes())?;
        }
        push_count(payload, self.sounds.len(), MAX_SOUND_HANDLES, "too many sounds")?;
        payload.extend_from_slice(&self.sounds);
        payload.extend_from_slice(&self.source_bytes.to_be_bytes());
        Ok(())
    }

    fn read(fields: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let context = ContextId::of(fields.u8()?)?;
        let api = fields.u16()?;
        let renderer = Renderer::of(fields.u8()?)?;
        let level = Level::of(fields.u8()?)?;
        let page = fields.u16()?;
        let subcode = fields.u16()?;
        let flags = ContextFlags::of(fields.u8()?)?;
        let tick_divisor = fields.u8()?;
        let language = fields.string(MAX_LANGUAGE_BYTES, "language")?;
        let channel_url = fields.string(MAX_URL_BYTES, "channel address")?;
        let page_url = fields.string(MAX_URL_BYTES, "page address")?;
        let script_url = fields.string(MAX_URL_BYTES, "script address")?;
        let channel_title = fields.string(MAX_TITLE_BYTES, "channel title")?;
        let seed = fields.array()?;
        let epoch_seconds = fields.u64()?;
        let zone_minutes = fields.i16()?;
        let count = fields.count_u16(MAX_KEY_ASKS, "too many keys")?;
        let mut keys = Vec::with_capacity(count);
        for _ in 0..count {
            keys.push(fields.string(MAX_KEY_NAME_BYTES, "key name")?);
        }
        let count = fields.count_u16(MAX_SOUND_HANDLES, "too many sounds")?;
        let sounds = fields.take(count)?.to_vec();
        let source_bytes = fields.u32()?;
        let open = Self {
            context,
            api,
            renderer,
            level,
            page,
            subcode,
            flags,
            tick_divisor,
            language,
            channel_url,
            page_url,
            script_url,
            channel_title,
            seed,
            epoch_seconds,
            zone_minutes,
            keys,
            sounds,
            source_bytes,
        };
        open.check()?;
        Ok(open)
    }
}

/// A script's length: not none, since an empty script is no script, and
/// at most [`MAX_SCRIPT_BYTES`].
fn check_source_length(length: u32) -> Result<(), ProtocolError> {
    if length == 0 || usize::try_from(length).is_ok_and(|length| length > MAX_SCRIPT_BYTES) { Err(ProtocolError::BadValue("source length")) } else { Ok(()) }
}

/// A piece of a source: not none, at most [`MAX_SOURCE_PIECE_BYTES`], and
/// ending within a script's length.
fn check_source_piece(offset: u32, bytes: &[u8]) -> Result<(), ProtocolError> {
    let end = usize::try_from(offset).ok().and_then(|offset| offset.checked_add(bytes.len()));
    if bytes.is_empty() || bytes.len() > MAX_SOURCE_PIECE_BYTES || end.is_none_or(|end| end > MAX_SCRIPT_BYTES) {
        Err(ProtocolError::BadValue("source piece"))
    } else {
        Ok(())
    }
}

/// An event of a step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// Its kind.
    pub kind: EventKind,
    /// Its four numbers, as the specification's section 10.3 says for the
    /// kind; those it does not use are 0.
    pub numbers: [i32; 4],
    /// Its text or bytes, at most [`MAX_EVENT_PAYLOAD_BYTES`].
    pub payload: Vec<u8>,
}

/// Where a tracked drawing is, from the renderer's last painting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    /// The drawing's handle.
    pub handle: i32,
    /// Its left.
    pub x: i32,
    /// Its top.
    pub y: i32,
    /// Its width.
    pub width: i32,
    /// Its height.
    pub height: i32,
}

/// What a voice did since the last step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundReport {
    /// A sound reached a cue.
    Cue {
        /// The voice.
        voice: u8,
        /// The cue.
        cue: i32,
    },
    /// A sound ended.
    Ended {
        /// The voice.
        voice: u8,
    },
}

const SOUND_CUE: u8 = 1;
const SOUND_ENDED: u8 = 2;

/// A bound the decoder held a script to since the last step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultReport {
    /// What happened, as the specification's `fault` event numbers it.
    pub code: i32,
    /// The bound's name: capital letters, digits and underscores, at most
    /// [`MAX_BOUND_NAME_BYTES`].
    pub bound: String,
}

/// One step: the events since the last, and what the renderers report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// The step's number, which its batch answers with.
    pub sequence: u32,
    /// The page's tick.
    pub tick: u64,
    /// The ticks missed since the last step.
    pub missed: u16,
    /// What the step is.
    pub flags: StepFlags,
    /// The events, at most [`MAX_STEP_EVENTS`], in order.
    pub events: Vec<Event>,
    /// The watched things painted, at most [`MAX_WATCHED_PAINTINGS`].
    pub painted: Vec<i32>,
    /// The things whose cycle ended, at most [`MAX_CYCLE_ENDS`].
    pub cycle_ended: Vec<i32>,
    /// The tracked drawings' bounds, at most [`MAX_TRACKED_DRAWINGS`].
    pub bounds: Vec<Bounds>,
    /// The voices' cues and ends, at most [`MAX_STEP_SOUND_REPORTS`].
    pub sound: Vec<SoundReport>,
    /// The faults, at most [`MAX_STEP_FAULTS`].
    pub faults: Vec<FaultReport>,
}

fn check_bound_name(name: &str) -> Result<(), ProtocolError> {
    let named = |byte: u8| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_';
    if name.is_empty() || name.len() > MAX_BOUND_NAME_BYTES || !name.bytes().all(named) { Err(ProtocolError::BadValue("bound name")) } else { Ok(()) }
}

pub(crate) fn push_handles(payload: &mut Vec<u8>, handles: &[i32], most: usize, what: &'static str) -> Result<(), ProtocolError> {
    push_count(payload, handles.len(), most, what)?;
    for handle in handles {
        payload.extend_from_slice(&handle.to_be_bytes());
    }
    Ok(())
}

pub(crate) fn take_handles(fields: &mut Fields<'_>, most: usize, what: &'static str) -> Result<Vec<i32>, ProtocolError> {
    let count = fields.count_u16(most, what)?;
    (0..count).map(|_| fields.i32()).collect()
}

impl Step {
    /// Whether every field keeps to its bounds.
    ///
    /// # Errors
    /// [`ProtocolError::BadValue`] naming the first that does not.
    pub fn check(&self) -> Result<(), ProtocolError> {
        let within = [
            (self.events.len(), MAX_STEP_EVENTS, "too many events"),
            (self.painted.len(), MAX_WATCHED_PAINTINGS, "too many painted"),
            (self.cycle_ended.len(), MAX_CYCLE_ENDS, "too many cycle ends"),
            (self.bounds.len(), MAX_TRACKED_DRAWINGS, "too many bounds"),
            (self.sound.len(), MAX_STEP_SOUND_REPORTS, "too many sound reports"),
            (self.faults.len(), MAX_STEP_FAULTS, "too many faults"),
        ];
        if let Some(&(_, _, what)) = within.iter().find(|(count, most, _)| count > most) {
            return Err(ProtocolError::BadValue(what));
        }
        if self.events.iter().any(|event| event.payload.len() > MAX_EVENT_PAYLOAD_BYTES) {
            return Err(ProtocolError::BadValue("event payload"));
        }
        self.faults.iter().try_for_each(|fault| check_bound_name(&fault.bound))
    }

    fn write(&self, payload: &mut Vec<u8>) -> Result<(), ProtocolError> {
        self.check()?;
        payload.extend_from_slice(&self.sequence.to_be_bytes());
        payload.extend_from_slice(&self.tick.to_be_bytes());
        payload.extend_from_slice(&self.missed.to_be_bytes());
        payload.push(pack(self.flags.list()));
        push_count(payload, self.events.len(), MAX_STEP_EVENTS, "too many events")?;
        for event in &self.events {
            payload.push(event.kind.code());
            for number in event.numbers {
                payload.extend_from_slice(&number.to_be_bytes());
            }
            push_bytes(payload, &event.payload)?;
        }
        push_handles(payload, &self.painted, MAX_WATCHED_PAINTINGS, "too many painted")?;
        push_handles(payload, &self.cycle_ended, MAX_CYCLE_ENDS, "too many cycle ends")?;
        push_count(payload, self.bounds.len(), MAX_TRACKED_DRAWINGS, "too many bounds")?;
        for bounds in &self.bounds {
            for number in [bounds.handle, bounds.x, bounds.y, bounds.width, bounds.height] {
                payload.extend_from_slice(&number.to_be_bytes());
            }
        }
        push_count(payload, self.sound.len(), MAX_STEP_SOUND_REPORTS, "too many sound reports")?;
        for report in &self.sound {
            match *report {
                SoundReport::Cue { voice, cue } => {
                    payload.extend_from_slice(&[SOUND_CUE, voice]);
                    payload.extend_from_slice(&cue.to_be_bytes());
                }
                SoundReport::Ended { voice } => payload.extend_from_slice(&[SOUND_ENDED, voice]),
            }
        }
        push_count(payload, self.faults.len(), MAX_STEP_FAULTS, "too many faults")?;
        for fault in &self.faults {
            payload.extend_from_slice(&fault.code.to_be_bytes());
            push_bytes(payload, fault.bound.as_bytes())?;
        }
        Ok(())
    }

    fn read(fields: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let sequence = fields.u32()?;
        let tick = fields.u64()?;
        let missed = fields.u16()?;
        let flags = StepFlags::of(fields.u8()?)?;
        let count = fields.count_u16(MAX_STEP_EVENTS, "too many events")?;
        let mut events = Vec::with_capacity(count);
        for _ in 0..count {
            let kind = EventKind::of(fields.u8()?)?;
            let numbers = [fields.i32()?, fields.i32()?, fields.i32()?, fields.i32()?];
            events.push(Event { kind, numbers, payload: fields.bytes(MAX_EVENT_PAYLOAD_BYTES, "event payload")?.to_vec() });
        }
        let painted = take_handles(fields, MAX_WATCHED_PAINTINGS, "too many painted")?;
        let cycle_ended = take_handles(fields, MAX_CYCLE_ENDS, "too many cycle ends")?;
        let count = fields.count_u16(MAX_TRACKED_DRAWINGS, "too many bounds")?;
        let mut bounds = Vec::with_capacity(count);
        for _ in 0..count {
            bounds.push(Bounds { handle: fields.i32()?, x: fields.i32()?, y: fields.i32()?, width: fields.i32()?, height: fields.i32()? });
        }
        let count = fields.count_u16(MAX_STEP_SOUND_REPORTS, "too many sound reports")?;
        let mut sound = Vec::with_capacity(count);
        for _ in 0..count {
            sound.push(match fields.u8()? {
                SOUND_CUE => SoundReport::Cue { voice: fields.u8()?, cue: fields.i32()? },
                SOUND_ENDED => SoundReport::Ended { voice: fields.u8()? },
                _ => return Err(ProtocolError::BadValue("sound report")),
            });
        }
        let count = fields.count_u16(MAX_STEP_FAULTS, "too many faults")?;
        let mut faults = Vec::with_capacity(count);
        for _ in 0..count {
            faults.push(FaultReport { code: fields.i32()?, bound: fields.string(MAX_BOUND_NAME_BYTES, "bound name")? });
        }
        let step = Self { sequence, tick, missed, flags, events, painted, cycle_ended, bounds, sound, faults };
        step.check()?;
        Ok(step)
    }
}

/// From a decoder to its script host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToScriptHost {
    /// The link's version, said first.
    Hello {
        /// The version of these messages the decoder speaks.
        link_version: u16,
    },
    /// A context opens; its source follows.
    ContextOpen(Box<ContextOpen>),
    /// A piece of a context's source. The context starts when a piece
    /// ends at the length its opening gave.
    Source {
        /// Which context.
        context: ContextId,
        /// Where in the source the piece starts.
        offset: u32,
        /// At most [`MAX_SOURCE_PIECE_BYTES`] bytes, and not none.
        bytes: Vec<u8>,
    },
    /// A step.
    Step(Box<Step>),
    /// A context closes.
    ContextClose(ContextId),
    /// The script host is to end.
    Shutdown,
}

/// From a script host to its decoder. A script host is untrusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromScriptHost {
    /// It is running, said first.
    Ready {
        /// The version of these messages it speaks.
        link_version: u16,
        /// The highest API version it has.
        api_max: u16,
    },
    /// A step's whole output.
    Batch(Box<Batch>),
    /// A context stopped for good.
    Failed {
        /// Which context.
        context: ContextId,
        /// Why.
        reason: FailReason,
        /// What to say in the developer log: plain text of at most
        /// [`MAX_LOG_CHARS`] characters.
        text: String,
    },
}

/// Whether the trusted core relays a decoder's message of `kind` to its
/// script host.
#[must_use]
pub const fn is_to_host_kind(kind: u8) -> bool {
    matches!(kind, KIND_HELLO | KIND_CONTEXT_OPEN | KIND_SOURCE | KIND_STEP | KIND_CONTEXT_CLOSE | KIND_SHUTDOWN)
}

/// Whether the trusted core relays a script host's message of `kind` to
/// its decoder.
#[must_use]
pub const fn is_to_decoder_kind(kind: u8) -> bool {
    matches!(kind, KIND_READY | KIND_BATCH | KIND_FAILED)
}

/// # Errors
/// When writing fails or a field is out of its bounds.
pub fn write_to_host(out: &mut dyn Write, message: &ToScriptHost) -> Result<(), ProtocolError> {
    match message {
        ToScriptHost::Hello { link_version } => write_frame(out, KIND_HELLO, &link_version.to_be_bytes()),
        ToScriptHost::ContextOpen(open) => {
            let mut payload = Vec::new();
            open.write(&mut payload)?;
            write_frame(out, KIND_CONTEXT_OPEN, &payload)
        }
        ToScriptHost::Source { context, offset, bytes } => {
            check_source_piece(*offset, bytes)?;
            let mut payload = vec![context.code()];
            payload.extend_from_slice(&offset.to_be_bytes());
            push_bytes(&mut payload, bytes)?;
            write_frame(out, KIND_SOURCE, &payload)
        }
        ToScriptHost::Step(step) => {
            let mut payload = Vec::new();
            step.write(&mut payload)?;
            write_frame(out, KIND_STEP, &payload)
        }
        ToScriptHost::ContextClose(context) => write_frame(out, KIND_CONTEXT_CLOSE, &[context.code()]),
        ToScriptHost::Shutdown => write_frame(out, KIND_SHUTDOWN, &[]),
    }
}

/// Reads one message from the decoder, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind a decoder does not send a script host, and any
/// payload that is malformed, out of its bounds or longer than its
/// fields.
pub fn read_to_host(input: &mut dyn Read) -> Result<Option<ToScriptHost>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_HELLO => ToScriptHost::Hello { link_version: fields.u16()? },
        KIND_CONTEXT_OPEN => ToScriptHost::ContextOpen(Box::new(ContextOpen::read(&mut fields)?)),
        KIND_SOURCE => {
            let context = ContextId::of(fields.u8()?)?;
            let offset = fields.u32()?;
            let bytes = fields.bytes(MAX_SOURCE_PIECE_BYTES, "source piece")?;
            check_source_piece(offset, bytes)?;
            ToScriptHost::Source { context, offset, bytes: bytes.to_vec() }
        }
        KIND_STEP => ToScriptHost::Step(Box::new(Step::read(&mut fields)?)),
        KIND_CONTEXT_CLOSE => ToScriptHost::ContextClose(ContextId::of(fields.u8()?)?),
        KIND_SHUTDOWN => ToScriptHost::Shutdown,
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// # Errors
/// When writing fails or a field is out of its bounds.
pub fn write_to_decoder(out: &mut dyn Write, message: &FromScriptHost) -> Result<(), ProtocolError> {
    match message {
        FromScriptHost::Ready { link_version, api_max } => {
            let mut payload = link_version.to_be_bytes().to_vec();
            payload.extend_from_slice(&api_max.to_be_bytes());
            write_frame(out, KIND_READY, &payload)
        }
        FromScriptHost::Batch(batch) => write_frame(out, KIND_BATCH, &batch.write()?),
        FromScriptHost::Failed { context, reason, text } => {
            check_log(text)?;
            let mut payload = vec![context.code(), reason.code()];
            push_bytes(&mut payload, text.as_bytes())?;
            write_frame(out, KIND_FAILED, &payload)
        }
    }
}

/// Reads one message from the script host, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind a script host may not send a decoder, and any
/// payload that is malformed, out of its bounds or longer than its
/// fields. The decoder then treats the script as stopped.
pub fn read_to_decoder(input: &mut dyn Read) -> Result<Option<FromScriptHost>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_READY => FromScriptHost::Ready { link_version: fields.u16()?, api_max: fields.u16()? },
        KIND_BATCH => FromScriptHost::Batch(Box::new(Batch::read(&mut fields)?)),
        KIND_FAILED => {
            let context = ContextId::of(fields.u8()?)?;
            let reason = FailReason::of(fields.u8()?)?;
            let text = fields.string(MAX_LOG_BYTES, "log line")?;
            check_log(&text)?;
            FromScriptHost::Failed { context, reason, text }
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

#[cfg(test)]
mod tests;

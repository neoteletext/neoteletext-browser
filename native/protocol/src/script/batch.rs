//! A step's output: one message of named sections, each at most once and
//! in their order, each a tag, a four-byte length and a body (the script
//! API's specification section 11.2). A section is a typed drawing
//! command for the page and sub-page that are open. None names a page,
//! and none carries a record or a packet: the decoder builds every packet
//! and record itself, so a script host that has been subverted cannot
//! write another page, a row 0 or 24, or a record of any other kind
//! (TODO-7274).
//!
//! What is checked here is each field's own bound and shape. What only
//! the page's state can decide (that a sprite's object exists, that a
//! handle is live, that the rates hold) is the decoder's to check, as the
//! authority.

use neoteletext_core::consts::{COLUMNS, LOCAL_TRIPLETS, OBJECT_BODY_TRIPLETS, OBJECT_NUMBER_BITS, PAGE_HEIGHT, PAGE_WIDTH, ROWS};
use neoteletext_core::limits::LAST_LAYER;
/// The most frames a sprite or a NAPLPS object has, and the most sprites
/// a page has: the engine's, by its names.
pub use neoteletext_core::limits::{MAX_OBJECT_FRAMES, MAX_SPRITES};
use neoteletext_rules::animate::{self, MAX_OPERATION_BYTES, Operation};
use neoteletext_rules::limits::{
    AUDIO_CHANNELS, FIRST_FAMILY, LAST_FAMILY, MAX_ANIMATE_OPERATIONS, MAX_GAIN, MAX_MIDI_DATA, MAX_PART, MAX_POINTER_OBJECTS, MAX_VARIANT,
};

use super::{
    CONTEXTS, ContextId, FailReason, MAX_LABEL_BYTES, MAX_LOG_BYTES, MAX_LOG_LINES, MAX_STEP_EVENTS, MAX_TITLE_BYTES, MAX_TRACKED_DRAWINGS,
    MAX_WATCHED_PAINTINGS, check_log, check_plain, pack, push_handles, take_handles, unpack,
};
use crate::web::MAX_URL_BYTES;
use crate::{Fields, MAX_PAYLOAD, ProtocolError, push_bytes, push_count};

/// The most bytes of one step's output, the whole message: the engine's
/// largest frame, asserted equal in the tests. A batch is always one
/// payload.
pub const MAX_BATCH_BYTES: usize = MAX_PAYLOAD;
const _: () = assert!(MAX_BATCH_BYTES <= MAX_PAYLOAD);

/// `value` as a byte, at compile time; a constant that did not fit would
/// fail the build here.
const fn narrow(value: usize) -> u8 {
    assert!(value <= u8::MAX as usize);
    #[expect(clippy::cast_possible_truncation, reason = "the assertion above holds it to 8 bits")]
    let narrowed = value as u8;
    narrowed
}

/// `value` as 16 bits, at compile time, as [`narrow`].
const fn narrow_wide(value: usize) -> u16 {
    assert!(value <= u16::MAX as usize);
    #[expect(clippy::cast_possible_truncation, reason = "the assertion above holds it to 16 bits")]
    let narrowed = value as u16;
    narrowed
}

/// The first row a script writes: row 0, the header, is never a script's.
pub const FIRST_SCRIPT_ROW: u8 = 1;
/// The last row a script writes: row 24 is never a script's.
pub const LAST_SCRIPT_ROW: u8 = narrow(ROWS - 2);
/// The most rows of one batch: every row a script writes.
pub const MAX_SCRIPT_ROWS: usize = (LAST_SCRIPT_ROW - FIRST_SCRIPT_ROW + 1) as usize;
/// The greatest character code of a row: seven bits (EN 300 706 clause
/// 7.1).
pub const MAX_ROW_CODE: u8 = 0x7F;
/// The greatest address of an enhancement triplet: six bits (EN 300 706
/// clause 12.3.1).
pub const MAX_TRIPLET_ADDRESS: u8 = 0x3F;
/// The greatest mode of an enhancement triplet: five bits.
pub const MAX_TRIPLET_MODE: u8 = 0x1F;
/// The greatest data of an enhancement triplet: seven bits.
pub const MAX_TRIPLET_DATA: u8 = 0x7F;
/// The most triplets of a page's local enhancement data: the engine's.
pub const MAX_LOCAL_TRIPLETS: usize = LOCAL_TRIPLETS as usize;
/// The most triplets of one object's body: the engine's.
pub const MAX_OBJECT_TRIPLETS: usize = OBJECT_BODY_TRIPLETS as usize;
/// The most objects of one batch: a frame for every frame of every
/// sprite.
pub const MAX_TELETEXT_OBJECTS: usize = MAX_SPRITES * MAX_OBJECT_FRAMES;
/// The greatest object number: nine bits (EN 300 706 Table 28).
pub const MAX_OBJECT_NUMBER: u16 = (1 << OBJECT_NUMBER_BITS) - 1;
/// The greatest colour-map entry (EN 300 706 clause 12.4): 32 entries.
pub const LAST_COLOUR_ENTRY: u8 = 31;
/// The first colour-map entry a page may redefine: those of CLUTs 2 and 3
/// (EN 300 706 clause 9.4.2.2).
pub const FIRST_DEFINABLE_ENTRY: u8 = 16;
/// The most colours a page redefines.
pub const MAX_DEFINABLE_COLOURS: usize = (LAST_COLOUR_ENTRY - FIRST_DEFINABLE_ENTRY + 1) as usize;
/// The greatest colour: four bits each of red, green and blue.
pub const MAX_RGB12: u16 = 0x0FFF;
/// The farthest column of a sprite's origin, either way: one short of the
/// page's columns (the engine's `SPRITES` record, ANIM-7).
pub const MAX_SPRITE_COLUMN: u8 = narrow(COLUMNS - 1);
/// The farthest row of a sprite's origin, either way.
pub const MAX_SPRITE_ROW: u8 = narrow(ROWS - 1);
/// The widest sprite, in cells: the page.
pub const MAX_SPRITE_WIDTH: u8 = narrow(COLUMNS);
/// The tallest sprite, in cells: the page.
pub const MAX_SPRITE_HEIGHT: u8 = narrow(ROWS);
/// The most NAPLPS objects a sub-page has: the figure of the engine's
/// `NAPLPS_OBJECTS` record, asserted equal in the tests.
pub const MAX_NAPLPS_OBJECTS: usize = 64;
/// The widest NAPLPS object, in dots: the page's raster (RAS-4).
pub const RASTER_WIDTH: u16 = narrow_wide(PAGE_WIDTH);
/// The tallest NAPLPS object, in lines.
pub const RASTER_HEIGHT: u16 = narrow_wide(PAGE_HEIGHT);
/// The first macro code (ANSI X3.110-1983 section 6.2): 2/0.
pub const FIRST_MACRO: u8 = 0x20;
/// The last macro code: 7/15.
pub const LAST_MACRO: u8 = 0x7F;
/// The looks an object has beside its frames: hover, pressed and
/// alternative.
pub const LOOKS: usize = 3;
/// The most sound operations of one step.
pub const MAX_SOUND_OPERATIONS: usize = 64;
/// The most things that take the wheel.
pub const MAX_WHEEL_THINGS: usize = MAX_POINTER_OBJECTS;
/// The most labels of one batch.
pub const MAX_LABELS: usize = 128;
/// The level of a log line a script made with `console.debug`.
pub const LOG_DEBUG: u8 = 0;
/// With `console.log` and `nt.log`.
pub const LOG_LOG: u8 = 1;
/// With `console.info`.
pub const LOG_INFO: u8 = 2;
/// With `console.warn`, and a warning of the script host's own.
pub const LOG_WARN: u8 = 3;
/// With `console.error`, and an uncaught exception or an unhandled
/// rejection.
pub const LOG_ERROR: u8 = 4;
/// The greatest level of a log line: 0 to 4.
pub const MAX_LOG_LEVEL: u8 = LOG_ERROR;

/// The level of the developer log a script's line of `level` is shown
/// at (TODO-7414): the one mapping, for the script host that numbers a
/// line and the trusted core that shows it. An error is an error, a
/// warning a warning, and everything quieter is information.
#[must_use]
pub const fn console_level(level: u8) -> crate::console::Level {
    match level {
        LOG_ERROR => crate::console::Level::Error,
        LOG_WARN => crate::console::Level::Warning,
        _ => crate::console::Level::Info,
    }
}
/// The MIDI statuses a script sends, each with a part for its low four
/// bits: note off, note on, control change, program change, pitch bend.
pub const MIDI_STATUSES: [u8; 5] = [0x80, 0x90, 0xB0, 0xC0, 0xE0];

// The sections' tags, in the order a batch holds them.
pub(super) const VERDICTS: u8 = 1;
pub(super) const TELETEXT_ROWS: u8 = 2;
pub(super) const TELETEXT_ENHANCEMENT: u8 = 3;
pub(super) const TELETEXT_SETTINGS: u8 = 4;
pub(super) const TELETEXT_OBJECTS: u8 = 5;
pub(super) const SPRITES: u8 = 6;
pub(super) const ANIMATE: u8 = 7;
pub(super) const REGIS: u8 = 8;
pub(super) const NAPLPS: u8 = 9;
pub(super) const NAPLPS_OBJECTS: u8 = 10;
pub(super) const SOUND: u8 = 11;
pub(super) const WATCH: u8 = 12;
pub(super) const LABELS: u8 = 13;
pub(super) const TITLE: u8 = 14;
pub(super) const NAVIGATE: u8 = 15;
pub(super) const LOG: u8 = 16;
pub(super) const STATE: u8 = 17;

/// What became of one event of the step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    /// A handler cancelled the event's default action.
    pub cancelled: bool,
    /// A listener handled the event.
    pub handled: bool,
}

/// One row of the page, whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TeletextRow {
    /// The row, [`FIRST_SCRIPT_ROW`] to [`LAST_SCRIPT_ROW`].
    pub row: u8,
    /// Its codes, each at most [`MAX_ROW_CODE`].
    pub codes: [u8; COLUMNS],
}

/// An enhancement triplet (EN 300 706 clause 12.3.1), unchecked beyond
/// its fields' widths: the decoder checks and orders a page's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Triplet {
    /// The address, at most [`MAX_TRIPLET_ADDRESS`].
    pub address: u8,
    /// The mode, at most [`MAX_TRIPLET_MODE`].
    pub mode: u8,
    /// The data, at most [`MAX_TRIPLET_DATA`].
    pub data: u8,
}

/// The page's settings, whole.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeletextSettings {
    /// The default screen colour, a colour-map entry.
    pub screen_colour: u8,
    /// The default row colour, a colour-map entry.
    pub row_colour: u8,
    /// Black background colour substitution.
    pub substitution: bool,
    /// The colours redefined: entries [`FIRST_DEFINABLE_ENTRY`] to
    /// [`LAST_COLOUR_ENTRY`] in rising order, each with its 12-bit colour.
    pub colours: Vec<(u8, u16)>,
}

/// A passive object: one frame of a sprite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeletextObject {
    /// Its number, at most [`MAX_OBJECT_NUMBER`]; a batch's objects are in
    /// rising order of it.
    pub number: u16,
    /// Its body, at most [`MAX_OBJECT_TRIPLETS`].
    pub triplets: Vec<Triplet>,
}

/// A teletext sprite: the fields of the engine's `SPRITES` record for one
/// sprite, with object numbers for its frames and no page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sprite {
    /// Its number, 1 to 65,535, distinct among a batch's.
    pub number: u16,
    /// Whether it is shown.
    pub shown: bool,
    /// Whether the pointer sees it.
    pub detectable: bool,
    /// Its layer, 1 to the last.
    pub layer: u8,
    /// Its order within the layer.
    pub order: i8,
    /// The column of its origin, within [`MAX_SPRITE_COLUMN`] either way.
    pub column: i8,
    /// The row of its origin, within [`MAX_SPRITE_ROW`] either way.
    pub row: i8,
    /// Its width in cells, 1 to [`MAX_SPRITE_WIDTH`].
    pub width: u8,
    /// Its height in cells, 1 to [`MAX_SPRITE_HEIGHT`].
    pub height: u8,
    /// Its frames' object numbers: 1 to the engine's `MAX_OBJECT_FRAMES`
    /// of them, each at most [`MAX_OBJECT_NUMBER`].
    pub objects: Vec<u16>,
}

/// NAPLPS bytes to append to the open page's stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NaplpsAppend {
    /// Whether they start a new generation of the picture.
    pub new_generation: bool,
    /// The bytes.
    pub bytes: Vec<u8>,
}

/// What a NAPLPS object is (ANIM-11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NaplpsObjectKind {
    /// Drawn by executing its frame's macro.
    Macro,
    /// A rectangle of the stream's own picture.
    Region,
}

const KIND_MACRO: u8 = 1;
const KIND_REGION: u8 = 2;

/// A NAPLPS object: the per-object fields of the engine's
/// `NAPLPS_OBJECTS` record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NaplpsObject {
    /// Its number, 1 to 65,535, distinct among a batch's.
    pub number: u16,
    /// A macro object or a region.
    pub kind: NaplpsObjectKind,
    /// Whether it is shown.
    pub shown: bool,
    /// Whether the pointer sees it.
    pub detectable: bool,
    /// Its layer, 1 to the last.
    pub layer: u8,
    /// Its order within the layer.
    pub order: i8,
    /// The raster dot of its rectangle's left.
    pub x: i16,
    /// The raster line of its rectangle's top.
    pub y: i16,
    /// Its width in dots, 1 to [`RASTER_WIDTH`].
    pub width: u16,
    /// Its height in lines, 1 to [`RASTER_HEIGHT`].
    pub height: u16,
    /// Where its macro puts the rectangle's top left.
    pub origin: (i16, i16),
    /// A region's colour-map entry while it is hidden.
    pub cover: u8,
    /// The hover, pressed and alternative looks: each a macro's code, or 0
    /// for none.
    pub looks: [u8; LOOKS],
    /// The frames' macro codes: 1 to the engine's `MAX_OBJECT_FRAMES` for
    /// a macro object, none for a region.
    pub frames: Vec<u8>,
    /// What a screen reader says for it: plain text of at most
    /// [`MAX_LABEL_BYTES`], or empty.
    pub label: String,
}

/// The open page's NAPLPS objects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NaplpsObjects {
    /// Whether the one object given replaces the page's object of its
    /// number and every other is kept; else these are all the page's.
    pub one: bool,
    /// The picture's title: plain text of at most [`MAX_TITLE_BYTES`], or
    /// empty; always empty with `one`.
    pub title: String,
    /// The objects, at most [`MAX_NAPLPS_OBJECTS`]; with `one`, exactly
    /// one.
    pub objects: Vec<NaplpsObject>,
}

/// One sound operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundOperation {
    /// Plays a sound on a voice.
    Play {
        /// The voice, one of the audio profile's channels.
        voice: u8,
        /// The sound's handle.
        sound: u8,
        /// The gain, at most the audio profile's.
        gain: u8,
    },
    /// Stops a voice.
    Stop {
        /// The voice.
        voice: u8,
    },
    /// Starts a synthesizer on a part.
    SynthStart {
        /// The part.
        part: u8,
        /// The synthesizer's family.
        family: u8,
        /// Its variant.
        variant: u8,
        /// The gain.
        gain: u8,
    },
    /// A MIDI message for a part.
    Midi {
        /// The part.
        part: u8,
        /// The status, one of [`MIDI_STATUSES`].
        status: u8,
        /// The first data byte.
        data1: u8,
        /// The second data byte.
        data2: u8,
    },
    /// Stops a part's synthesizer.
    SynthStop {
        /// The part.
        part: u8,
    },
}

const SOUND_PLAY: u8 = 1;
const SOUND_STOP: u8 = 2;
const SOUND_SYNTH_START: u8 = 3;
const SOUND_MIDI: u8 = 4;
const SOUND_SYNTH_STOP: u8 = 5;

/// The lists the decoder watches for a script, each sent whole when it
/// changed and absent when it did not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Watch {
    /// The things whose painting is reported, at most
    /// [`MAX_WATCHED_PAINTINGS`].
    pub painted: Option<Vec<i32>>,
    /// The drawings whose bounds are reported, at most
    /// [`MAX_TRACKED_DRAWINGS`].
    pub tracked: Option<Vec<i32>>,
    /// The things that take the wheel, at most [`MAX_WHEEL_THINGS`].
    pub wheel: Option<Vec<i32>>,
}

/// The bits that say which of [`Watch`]'s lists a section holds, in its
/// fields' order from bit 0.
const WATCH_LISTS: usize = 3;

/// A label for a thing whose record carries none: a teletext sprite or
/// control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    /// The thing's handle.
    pub handle: i32,
    /// Plain text of at most [`MAX_LABEL_BYTES`].
    pub label: String,
}

/// A line of the developer log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// Its level, 0 to [`MAX_LOG_LEVEL`].
    pub level: u8,
    /// The context that wrote it.
    pub context: ContextId,
    /// Plain text of at most `MAX_LOG_CHARS` characters.
    pub text: String,
}

/// How a context stands after the step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextState {
    /// Which context.
    pub context: ContextId,
    /// Why it stopped; `None` while it runs.
    pub stopped: Option<FailReason>,
    /// How many callbacks have failed one after another.
    pub failures: u16,
    /// The tick of its earliest timer, so that the decoder wakes for it
    /// and never polls; `None` with no timer.
    pub next_due: Option<u64>,
}

/// The code of a context that runs, where a stopped one has its reason's.
const RUNNING: u8 = 0;

/// One step's whole output. A section that is `None` is absent: the step
/// changed nothing of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Batch {
    /// The number of the step this answers.
    pub sequence: u32,
    /// A verdict for each event of the step, in the events' order.
    pub verdicts: Option<Vec<Verdict>>,
    /// The rows that changed, in rising order.
    pub teletext_rows: Option<Vec<TeletextRow>>,
    /// The open page's local enhancement data, whole.
    pub teletext_enhancement: Option<Vec<Triplet>>,
    /// The page's settings, whole.
    pub teletext_settings: Option<TeletextSettings>,
    /// The passive objects sprites' frames are.
    pub teletext_objects: Option<Vec<TeletextObject>>,
    /// The page's sprites, whole.
    pub sprites: Option<Vec<Sprite>>,
    /// Animation operations: `show`, `hide`, `look`, `move`, `order`,
    /// `frame`, `cycle` and `stop_cycle`, and never `reset`.
    pub animate: Option<Vec<Operation>>,
    /// ReGIS text to append to the open page's one stream.
    pub regis: Option<Vec<u8>>,
    /// NAPLPS bytes to append.
    pub naplps: Option<NaplpsAppend>,
    /// The page's NAPLPS objects.
    pub naplps_objects: Option<NaplpsObjects>,
    /// Sound operations, in order.
    pub sound: Option<Vec<SoundOperation>>,
    /// The watch lists that changed.
    pub watch: Option<Watch>,
    /// Labels.
    pub labels: Option<Vec<Label>>,
    /// The page's title: plain text of at most [`MAX_TITLE_BYTES`].
    pub title: Option<String>,
    /// Where to navigate: plain text, not empty, of at most
    /// [`MAX_URL_BYTES`], which the decoder reads as a target.
    pub navigate: Option<String>,
    /// Lines for the developer log.
    pub log: Option<Vec<LogLine>>,
    /// How each context stands, in the contexts' order.
    pub state: Option<Vec<ContextState>>,
}

fn bad<T>(what: &'static str) -> Result<T, ProtocolError> {
    Err(ProtocolError::BadValue(what))
}

/// Holds `condition`, or refuses by `what`.
fn hold(condition: bool, what: &'static str) -> Result<(), ProtocolError> {
    if condition { Ok(()) } else { bad(what) }
}

/// Whether each of `numbers` is greater than the one before.
fn rising<T: PartialOrd + Copy>(numbers: impl Iterator<Item = T>) -> bool {
    let mut last = None;
    numbers.into_iter().all(|number| {
        let risen = last.is_none_or(|last| number > last);
        last = Some(number);
        risen
    })
}

/// Whether no two of `numbers` are the same. The lists it is given are a
/// few dozen long at most.
fn distinct(numbers: &[u16]) -> bool {
    numbers.iter().enumerate().all(|(place, number)| !numbers.iter().take(place).any(|earlier| earlier == number))
}

fn push_flag_byte(body: &mut Vec<u8>, flags: [bool; 2]) {
    body.push(pack(flags));
}

fn write_verdicts(verdicts: &[Verdict], body: &mut Vec<u8>) -> Result<(), ProtocolError> {
    hold(verdicts.len() <= MAX_STEP_EVENTS, "too many verdicts")?;
    body.extend(verdicts.iter().map(|verdict| pack([verdict.cancelled, verdict.handled])));
    Ok(())
}

fn read_verdicts(section: &mut Fields<'_>) -> Result<Vec<Verdict>, ProtocolError> {
    let bytes = section.rest();
    hold(bytes.len() <= MAX_STEP_EVENTS, "too many verdicts")?;
    bytes
        .iter()
        .map(|&byte| {
            let [cancelled, handled] = unpack(byte, "verdict")?;
            Ok(Verdict { cancelled, handled })
        })
        .collect()
}

fn check_rows(rows: &[TeletextRow]) -> Result<(), ProtocolError> {
    hold(rows.len() <= MAX_SCRIPT_ROWS, "too many rows")?;
    hold(rows.iter().all(|row| (FIRST_SCRIPT_ROW..=LAST_SCRIPT_ROW).contains(&row.row)), "row")?;
    hold(rising(rows.iter().map(|row| row.row)), "rows out of order")?;
    hold(rows.iter().all(|row| row.codes.iter().all(|&code| code <= MAX_ROW_CODE)), "row code")
}

fn write_rows(rows: &[TeletextRow], body: &mut Vec<u8>) -> Result<(), ProtocolError> {
    check_rows(rows)?;
    push_count(body, rows.len(), MAX_SCRIPT_ROWS, "too many rows")?;
    for row in rows {
        body.push(row.row);
        body.extend_from_slice(&row.codes);
    }
    Ok(())
}

fn read_rows(section: &mut Fields<'_>) -> Result<Vec<TeletextRow>, ProtocolError> {
    let count = section.count_u16(MAX_SCRIPT_ROWS, "too many rows")?;
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        rows.push(TeletextRow { row: section.u8()?, codes: section.array()? });
    }
    check_rows(&rows)?;
    Ok(rows)
}

fn check_triplets(triplets: &[Triplet], most: usize) -> Result<(), ProtocolError> {
    hold(triplets.len() <= most, "too many triplets")?;
    let within = |triplet: &Triplet| triplet.address <= MAX_TRIPLET_ADDRESS && triplet.mode <= MAX_TRIPLET_MODE && triplet.data <= MAX_TRIPLET_DATA;
    hold(triplets.iter().all(within), "triplet")
}

fn write_triplets(triplets: &[Triplet], most: usize, body: &mut Vec<u8>) -> Result<(), ProtocolError> {
    check_triplets(triplets, most)?;
    push_count(body, triplets.len(), most, "too many triplets")?;
    for triplet in triplets {
        body.extend_from_slice(&[triplet.address, triplet.mode, triplet.data]);
    }
    Ok(())
}

fn read_triplets(section: &mut Fields<'_>, most: usize) -> Result<Vec<Triplet>, ProtocolError> {
    let count = section.count_u16(most, "too many triplets")?;
    let mut triplets = Vec::with_capacity(count);
    for _ in 0..count {
        let [address, mode, data] = section.array()?;
        triplets.push(Triplet { address, mode, data });
    }
    check_triplets(&triplets, most)?;
    Ok(triplets)
}

impl TeletextSettings {
    fn check(&self) -> Result<(), ProtocolError> {
        hold(self.screen_colour <= LAST_COLOUR_ENTRY && self.row_colour <= LAST_COLOUR_ENTRY, "default colour")?;
        hold(self.colours.len() <= MAX_DEFINABLE_COLOURS, "too many colours")?;
        hold(self.colours.iter().all(|&(entry, colour)| (FIRST_DEFINABLE_ENTRY..=LAST_COLOUR_ENTRY).contains(&entry) && colour <= MAX_RGB12), "colour")?;
        hold(rising(self.colours.iter().map(|&(entry, _)| entry)), "colours out of order")
    }

    fn write(&self, body: &mut Vec<u8>) -> Result<(), ProtocolError> {
        self.check()?;
        body.extend_from_slice(&[self.screen_colour, self.row_colour, u8::from(self.substitution)]);
        push_count(body, self.colours.len(), MAX_DEFINABLE_COLOURS, "too many colours")?;
        for &(entry, colour) in &self.colours {
            body.push(entry);
            body.extend_from_slice(&colour.to_be_bytes());
        }
        Ok(())
    }

    fn read(section: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let (screen_colour, row_colour, substitution) = (section.u8()?, section.u8()?, section.flag("substitution")?);
        let count = section.count_u16(MAX_DEFINABLE_COLOURS, "too many colours")?;
        let mut colours = Vec::with_capacity(count);
        for _ in 0..count {
            colours.push((section.u8()?, section.u16()?));
        }
        let settings = Self { screen_colour, row_colour, substitution, colours };
        settings.check()?;
        Ok(settings)
    }
}

fn check_objects(objects: &[TeletextObject]) -> Result<(), ProtocolError> {
    hold(objects.len() <= MAX_TELETEXT_OBJECTS, "too many objects")?;
    hold(objects.iter().all(|object| object.number <= MAX_OBJECT_NUMBER), "object number")?;
    hold(rising(objects.iter().map(|object| object.number)), "objects out of order")
}

fn write_objects(objects: &[TeletextObject], body: &mut Vec<u8>) -> Result<(), ProtocolError> {
    check_objects(objects)?;
    push_count(body, objects.len(), MAX_TELETEXT_OBJECTS, "too many objects")?;
    for object in objects {
        body.extend_from_slice(&object.number.to_be_bytes());
        write_triplets(&object.triplets, MAX_OBJECT_TRIPLETS, body)?;
    }
    Ok(())
}

fn read_objects(section: &mut Fields<'_>) -> Result<Vec<TeletextObject>, ProtocolError> {
    let count = section.count_u16(MAX_TELETEXT_OBJECTS, "too many objects")?;
    // Room is taken as objects are read, not for the count a hostile
    // sender claims.
    let mut objects = Vec::new();
    for _ in 0..count {
        objects.push(TeletextObject { number: section.u16()?, triplets: read_triplets(section, MAX_OBJECT_TRIPLETS)? });
    }
    check_objects(&objects)?;
    Ok(objects)
}

impl Sprite {
    fn check(&self) -> Result<(), ProtocolError> {
        hold(self.number >= 1, "sprite number")?;
        hold((1..=LAST_LAYER).contains(&self.layer), "sprite layer")?;
        hold(self.column.unsigned_abs() <= MAX_SPRITE_COLUMN && self.row.unsigned_abs() <= MAX_SPRITE_ROW, "sprite origin")?;
        hold((1..=MAX_SPRITE_WIDTH).contains(&self.width) && (1..=MAX_SPRITE_HEIGHT).contains(&self.height), "sprite size")?;
        hold((1..=MAX_OBJECT_FRAMES).contains(&self.objects.len()), "sprite frames")?;
        hold(self.objects.iter().all(|&object| object <= MAX_OBJECT_NUMBER), "object number")
    }
}

fn check_sprites(sprites: &[Sprite]) -> Result<(), ProtocolError> {
    hold(sprites.len() <= MAX_SPRITES, "too many sprites")?;
    sprites.iter().try_for_each(Sprite::check)?;
    let numbers: Vec<u16> = sprites.iter().map(|sprite| sprite.number).collect();
    hold(distinct(&numbers), "sprite numbers")
}

fn write_sprites(sprites: &[Sprite], body: &mut Vec<u8>) -> Result<(), ProtocolError> {
    check_sprites(sprites)?;
    push_count(body, sprites.len(), MAX_SPRITES, "too many sprites")?;
    for sprite in sprites {
        body.extend_from_slice(&sprite.number.to_be_bytes());
        push_flag_byte(body, [sprite.shown, sprite.detectable]);
        body.push(sprite.layer);
        for signed in [sprite.order, sprite.column, sprite.row] {
            body.extend_from_slice(&signed.to_be_bytes());
        }
        body.extend_from_slice(&[sprite.width, sprite.height]);
        push_count(body, sprite.objects.len(), MAX_OBJECT_FRAMES, "sprite frames")?;
        for object in &sprite.objects {
            body.extend_from_slice(&object.to_be_bytes());
        }
    }
    Ok(())
}

fn read_sprites(section: &mut Fields<'_>) -> Result<Vec<Sprite>, ProtocolError> {
    let count = section.count_u16(MAX_SPRITES, "too many sprites")?;
    let mut sprites = Vec::with_capacity(count);
    for _ in 0..count {
        let number = section.u16()?;
        let [shown, detectable] = unpack(section.u8()?, "sprite flags")?;
        let (layer, order, column, row, width, height) = (section.u8()?, section.i8()?, section.i8()?, section.i8()?, section.u8()?, section.u8()?);
        let frames = section.count_u16(MAX_OBJECT_FRAMES, "sprite frames")?;
        let objects = (0..frames).map(|_| section.u16()).collect::<Result<_, _>>()?;
        sprites.push(Sprite { number, shown, detectable, layer, order, column, row, width, height, objects });
    }
    check_sprites(&sprites)?;
    Ok(sprites)
}

/// An operation a script may make: any but `reset`, which is the
/// decoder's own when a thing is replaced.
fn check_operation(operation: &Operation) -> Result<(), ProtocolError> {
    hold(operation.code() != animate::code::RESET && operation.check().is_ok(), "animate operation")
}

fn write_animate(operations: &[Operation], body: &mut Vec<u8>) -> Result<(), ProtocolError> {
    hold((1..=MAX_ANIMATE_OPERATIONS).contains(&operations.len()), "animate operations")?;
    push_count(body, operations.len(), MAX_ANIMATE_OPERATIONS, "animate operations")?;
    for operation in operations {
        check_operation(operation)?;
        let mut bytes = [0_u8; MAX_OPERATION_BYTES];
        let used = animate::write(operation, &mut bytes).map_err(|_| ProtocolError::BadValue("animate operation"))?;
        body.extend_from_slice(bytes.get(..used).ok_or(ProtocolError::BadValue("animate operation"))?);
    }
    Ok(())
}

fn read_animate(section: &mut Fields<'_>) -> Result<Vec<Operation>, ProtocolError> {
    let count = section.count_u16(MAX_ANIMATE_OPERATIONS, "animate operations")?;
    hold(count >= 1, "animate operations")?;
    let mut rest = section.rest();
    let mut operations = Vec::with_capacity(count);
    for _ in 0..count {
        // The engine's own reader of an operation's bytes (ANIM-6).
        let (operation, used) = animate::read(rest).map_err(|_| ProtocolError::BadValue("animate operation"))?;
        check_operation(&operation)?;
        rest = rest.get(used..).ok_or(ProtocolError::BadValue("animate operation"))?;
        operations.push(operation);
    }
    hold(rest.is_empty(), "payload longer than its fields")?;
    Ok(operations)
}

impl NaplpsAppend {
    fn write(&self, body: &mut Vec<u8>) {
        body.push(u8::from(self.new_generation));
        body.extend_from_slice(&self.bytes);
    }

    fn read(section: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        Ok(Self { new_generation: section.flag("new generation")?, bytes: section.rest().to_vec() })
    }
}

impl NaplpsObject {
    fn check(&self) -> Result<(), ProtocolError> {
        let is_macro = |code: u8| (FIRST_MACRO..=LAST_MACRO).contains(&code);
        hold(self.number >= 1, "object number")?;
        hold((1..=LAST_LAYER).contains(&self.layer), "object layer")?;
        hold((1..=RASTER_WIDTH).contains(&self.width) && (1..=RASTER_HEIGHT).contains(&self.height), "object size")?;
        hold(self.looks.iter().all(|&look| look == 0 || is_macro(look)), "object look")?;
        let frames = match self.kind {
            NaplpsObjectKind::Macro => (1..=MAX_OBJECT_FRAMES).contains(&self.frames.len()) && self.frames.iter().all(|&frame| is_macro(frame)),
            NaplpsObjectKind::Region => self.frames.is_empty(),
        };
        hold(frames, "object frames")?;
        check_plain(&self.label, MAX_LABEL_BYTES, "label")
    }
}

impl NaplpsObjects {
    fn check(&self) -> Result<(), ProtocolError> {
        hold(self.objects.len() <= MAX_NAPLPS_OBJECTS, "too many objects")?;
        hold(!self.one || (self.objects.len() == 1 && self.title.is_empty()), "one object")?;
        check_plain(&self.title, MAX_TITLE_BYTES, "title")?;
        self.objects.iter().try_for_each(NaplpsObject::check)?;
        let numbers: Vec<u16> = self.objects.iter().map(|object| object.number).collect();
        hold(distinct(&numbers), "object numbers")
    }

    fn write(&self, body: &mut Vec<u8>) -> Result<(), ProtocolError> {
        self.check()?;
        body.push(u8::from(self.one));
        push_bytes(body, self.title.as_bytes())?;
        push_count(body, self.objects.len(), MAX_NAPLPS_OBJECTS, "too many objects")?;
        for object in &self.objects {
            body.extend_from_slice(&object.number.to_be_bytes());
            body.push(match object.kind {
                NaplpsObjectKind::Macro => KIND_MACRO,
                NaplpsObjectKind::Region => KIND_REGION,
            });
            push_flag_byte(body, [object.shown, object.detectable]);
            body.push(object.layer);
            body.extend_from_slice(&object.order.to_be_bytes());
            for signed in [object.x, object.y] {
                body.extend_from_slice(&signed.to_be_bytes());
            }
            for size in [object.width, object.height] {
                body.extend_from_slice(&size.to_be_bytes());
            }
            for signed in [object.origin.0, object.origin.1] {
                body.extend_from_slice(&signed.to_be_bytes());
            }
            body.push(object.cover);
            body.extend_from_slice(&object.looks);
            push_count(body, object.frames.len(), MAX_OBJECT_FRAMES, "object frames")?;
            body.extend_from_slice(&object.frames);
            push_bytes(body, object.label.as_bytes())?;
        }
        Ok(())
    }

    fn read(section: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let one = section.flag("one object")?;
        let title = section.string(MAX_TITLE_BYTES, "title")?;
        let count = section.count_u16(MAX_NAPLPS_OBJECTS, "too many objects")?;
        let mut objects = Vec::with_capacity(count);
        for _ in 0..count {
            let number = section.u16()?;
            let kind = match section.u8()? {
                KIND_MACRO => NaplpsObjectKind::Macro,
                KIND_REGION => NaplpsObjectKind::Region,
                _ => return bad("object kind"),
            };
            let [shown, detectable] = unpack(section.u8()?, "object flags")?;
            let (layer, order) = (section.u8()?, section.i8()?);
            let (x, y, width, height) = (section.i16()?, section.i16()?, section.u16()?, section.u16()?);
            let origin = (section.i16()?, section.i16()?);
            let (cover, looks) = (section.u8()?, section.array()?);
            let frames = section.count_u16(MAX_OBJECT_FRAMES, "object frames")?;
            let frames = section.take(frames)?.to_vec();
            let label = section.string(MAX_LABEL_BYTES, "label")?;
            objects.push(NaplpsObject { number, kind, shown, detectable, layer, order, x, y, width, height, origin, cover, looks, frames, label });
        }
        let all = Self { one, title, objects };
        all.check()?;
        Ok(all)
    }
}

impl SoundOperation {
    fn check(self) -> Result<(), ProtocolError> {
        let voiced = |voice: u8| usize::from(voice) < AUDIO_CHANNELS;
        let held = match self {
            Self::Play { voice, gain, .. } => voiced(voice) && gain <= MAX_GAIN,
            Self::Stop { voice } => voiced(voice),
            Self::SynthStart { part, family, variant, gain } => {
                part <= MAX_PART && (FIRST_FAMILY..=LAST_FAMILY).contains(&family) && variant <= MAX_VARIANT && gain <= MAX_GAIN
            }
            Self::Midi { part, status, data1, data2 } => {
                part <= MAX_PART && MIDI_STATUSES.contains(&status) && data1 <= MAX_MIDI_DATA && data2 <= MAX_MIDI_DATA
            }
            Self::SynthStop { part } => part <= MAX_PART,
        };
        hold(held, "sound operation")
    }
}

fn write_sound(operations: &[SoundOperation], body: &mut Vec<u8>) -> Result<(), ProtocolError> {
    push_count(body, operations.len(), MAX_SOUND_OPERATIONS, "too many sound operations")?;
    for operation in operations {
        operation.check()?;
        match *operation {
            SoundOperation::Play { voice, sound, gain } => body.extend_from_slice(&[SOUND_PLAY, voice, sound, gain]),
            SoundOperation::Stop { voice } => body.extend_from_slice(&[SOUND_STOP, voice]),
            SoundOperation::SynthStart { part, family, variant, gain } => body.extend_from_slice(&[SOUND_SYNTH_START, part, family, variant, gain]),
            SoundOperation::Midi { part, status, data1, data2 } => body.extend_from_slice(&[SOUND_MIDI, part, status, data1, data2]),
            SoundOperation::SynthStop { part } => body.extend_from_slice(&[SOUND_SYNTH_STOP, part]),
        }
    }
    Ok(())
}

fn read_sound(section: &mut Fields<'_>) -> Result<Vec<SoundOperation>, ProtocolError> {
    let count = section.count_u16(MAX_SOUND_OPERATIONS, "too many sound operations")?;
    let mut operations = Vec::with_capacity(count);
    for _ in 0..count {
        let operation = match section.u8()? {
            SOUND_PLAY => SoundOperation::Play { voice: section.u8()?, sound: section.u8()?, gain: section.u8()? },
            SOUND_STOP => SoundOperation::Stop { voice: section.u8()? },
            SOUND_SYNTH_START => SoundOperation::SynthStart { part: section.u8()?, family: section.u8()?, variant: section.u8()?, gain: section.u8()? },
            SOUND_MIDI => SoundOperation::Midi { part: section.u8()?, status: section.u8()?, data1: section.u8()?, data2: section.u8()? },
            SOUND_SYNTH_STOP => SoundOperation::SynthStop { part: section.u8()? },
            _ => return bad("sound operation"),
        };
        operation.check()?;
        operations.push(operation);
    }
    Ok(operations)
}

impl Watch {
    /// The lists with their bounds, in the order of the bits that say
    /// which are held.
    fn lists(&self) -> [(&Option<Vec<i32>>, usize); WATCH_LISTS] {
        [(&self.painted, MAX_WATCHED_PAINTINGS), (&self.tracked, MAX_TRACKED_DRAWINGS), (&self.wheel, MAX_WHEEL_THINGS)]
    }

    fn write(&self, body: &mut Vec<u8>) -> Result<(), ProtocolError> {
        let lists = self.lists();
        // A section that held no list would say nothing.
        hold(lists.iter().any(|(list, _)| list.is_some()), "watch lists")?;
        body.push(pack(lists.map(|(list, _)| list.is_some())));
        for (list, most) in lists {
            if let Some(handles) = list {
                push_handles(body, handles, most, "too many watched")?;
            }
        }
        Ok(())
    }

    fn read(section: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let held: [bool; WATCH_LISTS] = unpack(section.u8()?, "watch lists")?;
        hold(held.contains(&true), "watch lists")?;
        let mut watch = Self::default();
        let bounds = watch.lists().map(|(_, most)| most);
        for ((list, held), most) in [&mut watch.painted, &mut watch.tracked, &mut watch.wheel].into_iter().zip(held).zip(bounds) {
            if held {
                *list = Some(take_handles(section, most, "too many watched")?);
            }
        }
        Ok(watch)
    }
}

fn write_labels(labels: &[Label], body: &mut Vec<u8>) -> Result<(), ProtocolError> {
    push_count(body, labels.len(), MAX_LABELS, "too many labels")?;
    for label in labels {
        check_plain(&label.label, MAX_LABEL_BYTES, "label")?;
        body.extend_from_slice(&label.handle.to_be_bytes());
        push_bytes(body, label.label.as_bytes())?;
    }
    Ok(())
}

fn read_labels(section: &mut Fields<'_>) -> Result<Vec<Label>, ProtocolError> {
    let count = section.count_u16(MAX_LABELS, "too many labels")?;
    let mut labels = Vec::with_capacity(count);
    for _ in 0..count {
        let label = Label { handle: section.i32()?, label: section.string(MAX_LABEL_BYTES, "label")? };
        check_plain(&label.label, MAX_LABEL_BYTES, "label")?;
        labels.push(label);
    }
    Ok(labels)
}

fn check_target(target: &str) -> Result<(), ProtocolError> {
    hold(!target.is_empty(), "navigation target")?;
    check_plain(target, MAX_URL_BYTES, "navigation target")
}

fn write_log(lines: &[LogLine], body: &mut Vec<u8>) -> Result<(), ProtocolError> {
    push_count(body, lines.len(), MAX_LOG_LINES, "too many log lines")?;
    for line in lines {
        hold(line.level <= MAX_LOG_LEVEL, "log level")?;
        check_log(&line.text)?;
        body.extend_from_slice(&[line.level, line.context.code()]);
        push_bytes(body, line.text.as_bytes())?;
    }
    Ok(())
}

fn read_log(section: &mut Fields<'_>) -> Result<Vec<LogLine>, ProtocolError> {
    let count = section.count_u16(MAX_LOG_LINES, "too many log lines")?;
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        let level = section.u8()?;
        hold(level <= MAX_LOG_LEVEL, "log level")?;
        let line = LogLine { level, context: ContextId::of(section.u8()?)?, text: section.string(MAX_LOG_BYTES, "log line")? };
        check_log(&line.text)?;
        lines.push(line);
    }
    Ok(lines)
}

fn check_states(states: &[ContextState]) -> Result<(), ProtocolError> {
    hold(states.len() <= CONTEXTS, "too many contexts")?;
    hold(rising(states.iter().map(|state| state.context)), "contexts out of order")
}

fn write_states(states: &[ContextState], body: &mut Vec<u8>) -> Result<(), ProtocolError> {
    check_states(states)?;
    push_count(body, states.len(), CONTEXTS, "too many contexts")?;
    for state in states {
        body.extend_from_slice(&[state.context.code(), state.stopped.map_or(RUNNING, FailReason::code)]);
        body.extend_from_slice(&state.failures.to_be_bytes());
        body.push(u8::from(state.next_due.is_some()));
        body.extend_from_slice(&state.next_due.unwrap_or_default().to_be_bytes());
    }
    Ok(())
}

fn read_states(section: &mut Fields<'_>) -> Result<Vec<ContextState>, ProtocolError> {
    let count = section.count_u16(CONTEXTS, "too many contexts")?;
    let mut states = Vec::with_capacity(count);
    for _ in 0..count {
        let context = ContextId::of(section.u8()?)?;
        let stopped = match section.u8()? {
            RUNNING => None,
            reason => Some(FailReason::of(reason)?),
        };
        let failures = section.u16()?;
        let (timed, due) = (section.flag("next due")?, section.u64()?);
        // One way to say "no timer": the tick is then 0.
        hold(timed || due == 0, "next due")?;
        states.push(ContextState { context, stopped, failures, next_due: timed.then_some(due) });
    }
    check_states(&states)?;
    Ok(states)
}

/// Appends a section to `payload`: its tag, its body's length and its
/// body, which `write` fills.
fn section(payload: &mut Vec<u8>, tag: u8, write: impl FnOnce(&mut Vec<u8>) -> Result<(), ProtocolError>) -> Result<(), ProtocolError> {
    let mut body = Vec::new();
    write(&mut body)?;
    payload.push(tag);
    push_bytes(payload, &body)
}

impl Batch {
    /// A batch for the step `sequence` that changes nothing.
    #[must_use]
    pub fn new(sequence: u32) -> Self {
        Self { sequence, ..Self::default() }
    }

    /// The batch as a payload.
    ///
    /// # Errors
    /// [`ProtocolError::BadValue`] naming the first field out of its
    /// bounds, and [`ProtocolError::TooLong`] for a batch of more than
    /// [`MAX_BATCH_BYTES`].
    pub(crate) fn write(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut payload = self.sequence.to_be_bytes().to_vec();
        let out = &mut payload;
        if let Some(verdicts) = &self.verdicts {
            section(out, VERDICTS, |body| write_verdicts(verdicts, body))?;
        }
        if let Some(rows) = &self.teletext_rows {
            section(out, TELETEXT_ROWS, |body| write_rows(rows, body))?;
        }
        if let Some(triplets) = &self.teletext_enhancement {
            section(out, TELETEXT_ENHANCEMENT, |body| write_triplets(triplets, MAX_LOCAL_TRIPLETS, body))?;
        }
        if let Some(settings) = &self.teletext_settings {
            section(out, TELETEXT_SETTINGS, |body| settings.write(body))?;
        }
        if let Some(objects) = &self.teletext_objects {
            section(out, TELETEXT_OBJECTS, |body| write_objects(objects, body))?;
        }
        if let Some(sprites) = &self.sprites {
            section(out, SPRITES, |body| write_sprites(sprites, body))?;
        }
        if let Some(operations) = &self.animate {
            section(out, ANIMATE, |body| write_animate(operations, body))?;
        }
        if let Some(text) = &self.regis {
            section(out, REGIS, |body| {
                body.extend_from_slice(text);
                Ok(())
            })?;
        }
        if let Some(naplps) = &self.naplps {
            section(out, NAPLPS, |body| {
                naplps.write(body);
                Ok(())
            })?;
        }
        if let Some(objects) = &self.naplps_objects {
            section(out, NAPLPS_OBJECTS, |body| objects.write(body))?;
        }
        if let Some(operations) = &self.sound {
            section(out, SOUND, |body| write_sound(operations, body))?;
        }
        if let Some(watch) = &self.watch {
            section(out, WATCH, |body| watch.write(body))?;
        }
        if let Some(labels) = &self.labels {
            section(out, LABELS, |body| write_labels(labels, body))?;
        }
        if let Some(title) = &self.title {
            check_plain(title, MAX_TITLE_BYTES, "title")?;
            section(out, TITLE, |body| {
                body.extend_from_slice(title.as_bytes());
                Ok(())
            })?;
        }
        if let Some(target) = &self.navigate {
            check_target(target)?;
            section(out, NAVIGATE, |body| {
                body.extend_from_slice(target.as_bytes());
                Ok(())
            })?;
        }
        if let Some(lines) = &self.log {
            section(out, LOG, |body| write_log(lines, body))?;
        }
        if let Some(states) = &self.state {
            section(out, STATE, |body| write_states(states, body))?;
        }
        if payload.len() > MAX_BATCH_BYTES { Err(ProtocolError::TooLong(payload.len())) } else { Ok(payload) }
    }

    /// Reads a batch's payload: its sections each at most once and in
    /// their order, each read to its exact length.
    pub(crate) fn read(fields: &mut Fields<'_>) -> Result<Self, ProtocolError> {
        let mut batch = Self::new(fields.u32()?);
        let mut last = 0_u8;
        while !fields.is_empty() {
            let tag = fields.u8()?;
            // Rising tags: none twice, none out of its place.
            hold(tag > last, "section order")?;
            last = tag;
            let mut body = Fields::new(fields.bytes(MAX_BATCH_BYTES, "section length")?);
            let text = |body: &mut Fields<'_>, what| core::str::from_utf8(body.rest()).map(str::to_owned).map_err(|_| ProtocolError::BadValue(what));
            match tag {
                VERDICTS => batch.verdicts = Some(read_verdicts(&mut body)?),
                TELETEXT_ROWS => batch.teletext_rows = Some(read_rows(&mut body)?),
                TELETEXT_ENHANCEMENT => batch.teletext_enhancement = Some(read_triplets(&mut body, MAX_LOCAL_TRIPLETS)?),
                TELETEXT_SETTINGS => batch.teletext_settings = Some(TeletextSettings::read(&mut body)?),
                TELETEXT_OBJECTS => batch.teletext_objects = Some(read_objects(&mut body)?),
                SPRITES => batch.sprites = Some(read_sprites(&mut body)?),
                ANIMATE => batch.animate = Some(read_animate(&mut body)?),
                REGIS => batch.regis = Some(body.rest().to_vec()),
                NAPLPS => batch.naplps = Some(NaplpsAppend::read(&mut body)?),
                NAPLPS_OBJECTS => batch.naplps_objects = Some(NaplpsObjects::read(&mut body)?),
                SOUND => batch.sound = Some(read_sound(&mut body)?),
                WATCH => batch.watch = Some(Watch::read(&mut body)?),
                LABELS => batch.labels = Some(read_labels(&mut body)?),
                TITLE => {
                    let title = text(&mut body, "title")?;
                    check_plain(&title, MAX_TITLE_BYTES, "title")?;
                    batch.title = Some(title);
                }
                NAVIGATE => {
                    let target = text(&mut body, "navigation target")?;
                    check_target(&target)?;
                    batch.navigate = Some(target);
                }
                LOG => batch.log = Some(read_log(&mut body)?),
                STATE => batch.state = Some(read_states(&mut body)?),
                _ => return bad("section"),
            }
            body.end()?;
        }
        Ok(batch)
    }
}

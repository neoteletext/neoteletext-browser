//! A sprite's frames as teletext holds them (script-api.md section 5.3):
//! each frame a Passive object (EN 300 706 clause 13.5), and the objects
//! laid out on the Object page the decoder writes for the script.
//!
//! A Passive object is drawn only where it defines a character, and with
//! its own attributes whatever lies beneath: which is what a frame is,
//! the cells its block set and no others. Its positions are from its own
//! origin: its Active Position is row 0, column 0 as it starts (clause
//! 13.7), a Set Active Position triplet's address is 40 plus the row from
//! the origin, and a column triplet's address is the column. Its triplets
//! are in screen order (Annex A.4): row by row from the top, and by
//! column along each row, each cell's attributes before its character.
//!
//! A Passive object starts in white on black with contiguous mosaics, and
//! clause 13.5 has its attributes hold over a change of row, where Table
//! 29 has a colour hold to the end of its row. So that a frame is drawn
//! the same by a receiver that follows either, each row's first cell is
//! written with its colours whatever the row before left in force.

use neoteletext_core::charset::modified_designation;
use neoteletext_core::consts::{
    ATTRIBUTE_UNDERLINE, MODE_BACKGROUND_COLOUR, MODE_DISPLAY_ATTRIBUTES, MODE_FOREGROUND_COLOUR, MODE_G0_LEVEL_25, MODE_G0_NO_DIACRITIC, MODE_G1, MODE_G2,
    MODE_MODIFIED_DESIGNATION, MODE_SET_ACTIVE_POSITION, MOSAIC_BIT5, MOSAIC_BIT6, MOSAIC_LOW_BITS, ROW_ADDRESS_BASE, SPACE,
};
use protocol::script::ContextId;
use protocol::script::batch::{MAX_ROW_CODE, MAX_TRIPLET_DATA, MAX_TRIPLET_MODE, TeletextObject, Triplet};
use protocol::script::sprites::{object_number, object_place, pointer_triplets};

use super::super::limits::LAST_ENTRY;
use super::super::{range, type_error};
use super::limits::{
    CELL_BYTES, LAST_SIXELS, MAX_OBJECT_BODY, NO_SETS, OBJECT_BODY_TRIPLETS, OBJECT_FRAMING_TRIPLETS, OBJECT_SUBPAGES, OBJECTS_PER_SUBPAGE,
    PAGE_SHARE_OBJECT_SUBPAGES, SUBPAGE_TRIPLETS,
};
use crate::host::Reply;

/// A cell's kind, the first of its [`CELL_BYTES`]: never set, so
/// transparent.
pub const CELL_NONE: u8 = 0;
/// A contiguous mosaic; its code is the six sixels.
pub const CELL_CONTIGUOUS: u8 = 1;
/// A separated mosaic.
pub const CELL_SEPARATED: u8 = 2;
/// A character; its code, mode and sets are the triplet the engine's
/// encoder gave for it (operation `TT_BLOCK_TEXT`).
pub const CELL_CHARACTER: u8 = 3;

/// The Display Attributes data a separated mosaic is drawn with: data bit
/// D5, "underline or separated mosaics" (Table 29, mode 01100).
const DISPLAY_SEPARATED: u8 = ATTRIBUTE_UNDERLINE;
/// The Display Attributes data of every other cell.
const DISPLAY_PLAIN: u8 = 0;

/// What one cell of a frame shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Shown {
    /// The triplet that writes its character: its mode and data.
    mode: u8,
    data: u8,
    /// Its foreground and background, Colour Map entries.
    ink: u8,
    paper: u8,
    /// Its Display Attributes.
    display: u8,
    /// The designation of the sets its character is read in, when one is
    /// to be written.
    sets: Option<u8>,
}

/// The G1 code of the mosaic whose six sixels are `sixels`, bit 0 the top
/// left and bit 5 the bottom right (EN 300 706 clause 15.6.1, Table 47):
/// the first five are the code's bits 0 to 4, the sixth its bit 6, and
/// bit 5 is set in every mosaic.
const fn mosaic_code(sixels: u8) -> u8 {
    MOSAIC_BIT5 | (sixels & MOSAIC_LOW_BITS) | if sixels & !MOSAIC_LOW_BITS == 0 { 0 } else { MOSAIC_BIT6 }
}

/// Whether `mode` is a column triplet that writes one character of a G0
/// or G2 set: what the engine's encoder writes for text (Table 29 modes
/// 01001, 01111 and 10000 to 11111).
const fn writes_character(mode: u8) -> bool {
    mode == MODE_G0_LEVEL_25 || mode == MODE_G2 || (mode >= MODE_G0_NO_DIACRITIC && mode <= MAX_TRIPLET_MODE)
}

/// One cell's six bytes, checked: what it shows, or none for a cell never
/// set. A script's bytes are outside input: nothing of them reaches a
/// triplet unchecked, so that no frame can hold an invocation, a
/// definition, a full screen or full row colour, a DRCS character or a
/// Termination Marker.
fn cell_of(bytes: &[u8]) -> Result<Option<Shown>, Reply> {
    let [kind, code, ink, paper, mode, sets] = <[u8; CELL_BYTES]>::try_from(bytes).map_err(|_| type_error("a block's cell is six bytes"))?;
    if kind == CELL_NONE {
        return Ok(None);
    }
    for (what, entry) in [("ink", ink), ("paper", paper)] {
        if i32::from(entry) > LAST_ENTRY {
            return Err(range("COLOUR_MAP_ENTRIES", &format!("a cell's {what} is a Colour Map entry, 0 to {LAST_ENTRY}, not {entry}")));
        }
    }
    match kind {
        CELL_CONTIGUOUS | CELL_SEPARATED => {
            if code > LAST_SIXELS {
                return Err(range("MOSAIC_PATTERNS", &format!("a mosaic is six sixels, 0 to {LAST_SIXELS}, not {code}")));
            }
            let display = if kind == CELL_SEPARATED { DISPLAY_SEPARATED } else { DISPLAY_PLAIN };
            Ok(Some(Shown { mode: MODE_G1, data: mosaic_code(code), ink, paper, display, sets: None }))
        }
        CELL_CHARACTER => {
            if !writes_character(mode) || !(SPACE..=MAX_ROW_CODE).contains(&code) {
                return Err(type_error("a block's character is one nt.teletext gave for its text"));
            }
            let sets = if sets == NO_SETS {
                None
            } else if sets <= MAX_TRIPLET_DATA && modified_designation(sets).is_some() {
                Some(sets)
            } else {
                return Err(type_error("a block's character is one nt.teletext gave for its text"));
            };
            Ok(Some(Shown { mode, data: code, ink, paper, display: DISPLAY_PLAIN, sets }))
        }
        _ => Err(type_error("a block's cell is transparent, a mosaic or a character")),
    }
}

/// The body of the Passive object that draws one frame: `cells` is the
/// frame's `rows` by `columns` cells, row by row, [`CELL_BYTES`] each
/// (the caller has held the length). The body is what lies between the
/// object's Object Definition triplet and its Termination Marker, which
/// the decoder writes; with them the object is at most
/// [`OBJECT_BODY_TRIPLETS`] long, or the frame is refused by that name.
///
/// # Errors
/// The `RangeError` or `TypeError` the script is thrown; nothing has
/// then changed.
pub fn compile(columns: u8, rows: u8, cells: &[u8]) -> Result<Vec<Triplet>, Reply> {
    let mut shown: Vec<Option<Shown>> = Vec::with_capacity(cells.len().checked_div(CELL_BYTES).unwrap_or(0));
    for bytes in cells.chunks(CELL_BYTES) {
        shown.push(cell_of(bytes)?);
    }
    // An object that uses Display Attributes anywhere writes them at each
    // row's first cell, as it writes the colours.
    let uses_display = shown.iter().flatten().any(|cell| cell.display != DISPLAY_PLAIN);
    let mut body: Vec<Triplet> = Vec::new();
    for (row, line) in (0..rows).zip(shown.chunks(usize::from(columns).max(1))) {
        // What the row's triplets have left in force: nothing is taken
        // from the row above.
        let mut in_force: Option<Shown> = None;
        let mut sets_in_force: Option<u8> = None;
        for (column, cell) in (0..columns).zip(line) {
            let Some(cell) = cell else { continue };
            if in_force.is_none() && row != 0 {
                // The object starts on its origin's row: only a row below
                // it is addressed.
                body.push(Triplet { address: ROW_ADDRESS_BASE.saturating_add(row), mode: MODE_SET_ACTIVE_POSITION, data: column });
            }
            if in_force.is_none_or(|before| before.ink != cell.ink) {
                body.push(Triplet { address: column, mode: MODE_FOREGROUND_COLOUR, data: cell.ink });
            }
            if in_force.is_none_or(|before| before.paper != cell.paper) {
                body.push(Triplet { address: column, mode: MODE_BACKGROUND_COLOUR, data: cell.paper });
            }
            if in_force.map_or(uses_display, |before| before.display != cell.display) {
                body.push(Triplet { address: column, mode: MODE_DISPLAY_ATTRIBUTES, data: cell.display });
            }
            if let Some(sets) = cell.sets
                && sets_in_force != Some(sets)
            {
                body.push(Triplet { address: column, mode: MODE_MODIFIED_DESIGNATION, data: sets });
                sets_in_force = Some(sets);
            }
            body.push(Triplet { address: column, mode: cell.mode, data: cell.data });
            in_force = Some(*cell);
        }
        if body.len() > MAX_OBJECT_BODY {
            return Err(range(
                "OBJECT_BODY_TRIPLETS",
                &format!(
                    "a frame's object is at most {OBJECT_BODY_TRIPLETS} triplets with its Object Definition triplet and its Termination Marker, so {MAX_OBJECT_BODY} of its own; this frame needs more by its row {row}"
                ),
            ));
        }
    }
    Ok(body)
}

/// One object of the Object page.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Stored {
    /// Its number, which says where it lies (Table 28).
    number: u16,
    /// Its body.
    body: Vec<Triplet>,
    /// The frames of sprites that are this object: frames that are the
    /// same are one object.
    users: u16,
}

/// The Object page the script's sprites take their frames from, as the
/// host lays it out: the page context's objects on its first sub-pages
/// and the channel context's on its last, so that neither can leave the
/// other no room (section 3.1).
#[derive(Default)]
pub struct ObjectPage {
    /// The objects, in rising order of number.
    objects: Vec<Stored>,
    /// The triplets of every body together.
    triplets: usize,
    /// Whether the decoder is yet to be told of the objects as they are.
    changed: bool,
}

/// The sub-pages that hold `owner`'s objects.
const fn share(owner: ContextId) -> core::ops::Range<usize> {
    match owner {
        ContextId::Page => 0..PAGE_SHARE_OBJECT_SUBPAGES,
        ContextId::Channel => PAGE_SHARE_OBJECT_SUBPAGES..OBJECT_SUBPAGES,
    }
}

impl ObjectPage {
    /// The objects held.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether no object is held.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// The triplets of the Object page's definitions that are taken: each
    /// object's body and its framing.
    #[must_use]
    pub const fn triplets_used(&self) -> usize {
        self.triplets.saturating_add(self.objects.len().saturating_mul(OBJECT_FRAMING_TRIPLETS))
    }

    /// The bytes the objects' section takes in the step's output, when
    /// the step carries it.
    #[must_use]
    pub fn batch_bytes(&self) -> usize {
        if self.changed {
            self.objects.len().saturating_mul(size_of::<TeletextObject>()).saturating_add(self.triplets.saturating_mul(size_of::<Triplet>()))
        } else {
            0
        }
    }

    /// The object of `number`'s place among those held.
    fn find(&self, number: u16) -> Option<usize> {
        self.objects.iter().position(|object| object.number == number)
    }

    /// Where a new object of `length` body triplets fits on sub-page
    /// `subpage`: its number, when the sub-page has a pointer left for it
    /// and its definitions still fit the sub-page's triplets (clause
    /// 10.5.1.2).
    fn place(&self, subpage: usize, length: usize) -> Option<u16> {
        let mut taken = [false; OBJECTS_PER_SUBPAGE];
        let mut used = 0_usize;
        let mut highest = 0_usize;
        for object in &self.objects {
            let Some((at, index)) = object_place(object.number) else { continue };
            if at != subpage {
                continue;
            }
            if let Some(slot) = taken.get_mut(index) {
                *slot = true;
            }
            used = used.saturating_add(object.body.len()).saturating_add(OBJECT_FRAMING_TRIPLETS);
            highest = highest.max(index);
        }
        let index = taken.iter().position(|slot| !slot)?;
        let needed = pointer_triplets(highest.max(index)).saturating_add(used).saturating_add(length).saturating_add(OBJECT_FRAMING_TRIPLETS);
        if needed > SUBPAGE_TRIPLETS {
            return None;
        }
        object_number(subpage, index)
    }

    /// Takes an object with `body` for a frame of `owner`'s: the object
    /// one of the owner's frames already is, when the bodies are the
    /// same, or a new one on the first of the owner's sub-pages that has
    /// room. None when none has.
    pub fn take(&mut self, owner: ContextId, body: &[Triplet]) -> Option<u16> {
        let own = |number: u16| object_place(number).is_some_and(|(subpage, _)| share(owner).contains(&subpage));
        if let Some(same) = self.objects.iter_mut().find(|object| own(object.number) && object.body == body) {
            same.users = same.users.saturating_add(1);
            return Some(same.number);
        }
        let number = share(owner).find_map(|subpage| self.place(subpage, body.len()))?;
        let at = self.objects.partition_point(|object| object.number < number);
        self.objects.insert(at, Stored { number, body: body.to_vec(), users: 1 });
        self.triplets = self.triplets.saturating_add(body.len());
        self.changed = true;
        Some(number)
    }

    /// A frame no longer is the object `number`; an object no frame is
    /// goes from the page.
    pub fn release(&mut self, number: u16) {
        let Some(at) = self.find(number) else { return };
        let Some(object) = self.objects.get_mut(at) else { return };
        object.users = object.users.saturating_sub(1);
        if object.users == 0 {
            self.triplets = self.triplets.saturating_sub(object.body.len());
            self.objects.remove(at);
            self.changed = true;
        }
    }

    /// The objects, whole, when the decoder has not been told of them as
    /// they are; it then has.
    pub fn section(&mut self) -> Option<Vec<TeletextObject>> {
        if !core::mem::take(&mut self.changed) {
            return None;
        }
        Some(self.objects.iter().map(|object| TeletextObject { number: object.number, triplets: object.body.clone() }).collect())
    }
}

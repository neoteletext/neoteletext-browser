//! The Colour Map's entries a script defines (script-api.md section 5.5;
//! EN 300 706 clause 9.4.2.2 and Table 30): CLUTs 2 and 3, entries 16 to
//! 31, each a 12-bit colour written as X/28/0 format 1. Redefining an
//! entry recolours every cell drawn in it for a few bytes, so each entry
//! is held to [`MAX_COLOUR_SETS_PER_SECOND`] changes a second (section
//! 10.5; the security review's finding S-10, TODO-7276): a change past
//! the rate waits, the latest value winning, and is applied at the first
//! step the rate allows. Nothing throws for it.

use protocol::script::batch::FIRST_DEFINABLE_ENTRY;

use super::limits::{FIRST_DEFINABLE, LAST_ENTRY, LAST_RGB12, MAX_COLOUR_SETS_PER_SECOND, TELETEXT_DEFINABLE_COLOURS};
use crate::host::Reply;
use crate::limits::TICKS_PER_SECOND;

/// One defined entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    /// The colour it shows.
    shown: u16,
    /// A colour asked for past the rate, waiting.
    waiting: Option<u16>,
    /// The ticks of its last changes, the oldest overwritten first; none
    /// for a change not yet made.
    changes: [Option<u64>; MAX_COLOUR_SETS_PER_SECOND],
    /// Where the next change's tick goes.
    next: usize,
}

impl Entry {
    /// The first tick at or after `tick` at which the entry may change.
    fn allowed_at(&self, tick: u64) -> u64 {
        // The slot the next change overwrites holds the oldest of the last
        // `MAX_COLOUR_SETS_PER_SECOND` changes: a second after it, one
        // more is within the rate.
        match self.changes.get(self.next).copied().flatten() {
            Some(oldest) => tick.max(oldest.saturating_add(TICKS_PER_SECOND)),
            None => tick,
        }
    }

    /// Shows `colour` from `tick`.
    fn change(&mut self, colour: u16, tick: u64) {
        self.shown = colour;
        self.waiting = None;
        if let Some(slot) = self.changes.get_mut(self.next) {
            *slot = Some(tick);
        }
        self.next = self.next.saturating_add(1).checked_rem(MAX_COLOUR_SETS_PER_SECOND).unwrap_or(0);
    }
}

/// The entries a script has defined.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColourMap {
    entries: [Option<Entry>; TELETEXT_DEFINABLE_COLOURS],
}

/// A 12-bit colour, or the `RangeError`.
fn rgb12(colour: i32) -> Result<u16, Reply> {
    u16::try_from(colour)
        .ok()
        .filter(|_| colour <= LAST_RGB12)
        .ok_or_else(|| Reply::Range(format!("MAX_RGB12: a colour is twelve bits, 0 to {LAST_RGB12}, not {colour}")))
}

impl ColourMap {
    /// `colours.define`: the next free entry, showing `colour` at once.
    ///
    /// # Errors
    /// A `RangeError` for a colour past twelve bits or a seventeenth
    /// colour.
    pub fn define(&mut self, colour: i32, tick: u64) -> Result<i32, Reply> {
        let colour = rgb12(colour)?;
        let Some((at, slot)) = self.entries.iter_mut().enumerate().find(|(_, slot)| slot.is_none()) else {
            return Err(Reply::Range(format!("TELETEXT_DEFINABLE_COLOURS: a page defines at most {TELETEXT_DEFINABLE_COLOURS} colours")));
        };
        let mut entry = Entry { shown: colour, waiting: None, changes: [None; MAX_COLOUR_SETS_PER_SECOND], next: 0 };
        entry.change(colour, tick);
        *slot = Some(entry);
        Ok(FIRST_DEFINABLE.saturating_add(i32::try_from(at).unwrap_or(0)))
    }

    /// `colours.set`: redefines a defined entry, now or when its rate
    /// allows.
    ///
    /// # Errors
    /// A `RangeError` for an entry outside 16 to 31 or a colour past
    /// twelve bits; a `TypeError` for an entry never defined.
    pub fn set(&mut self, entry: i32, colour: i32, tick: u64) -> Result<(), Reply> {
        if !(FIRST_DEFINABLE..=LAST_ENTRY).contains(&entry) {
            return Err(Reply::Range(format!("TELETEXT_DEFINABLE_COLOURS: a script sets the entries {FIRST_DEFINABLE} to {LAST_ENTRY}, not {entry}")));
        }
        let colour = rgb12(colour)?;
        let at = usize::try_from(entry.saturating_sub(FIRST_DEFINABLE)).unwrap_or(usize::MAX);
        let Some(held) = self.entries.get_mut(at).and_then(Option::as_mut) else {
            return Err(Reply::Type(format!("the entry {entry} is not one colours.define gave")));
        };
        if held.allowed_at(tick) <= tick {
            held.change(colour, tick);
        } else {
            held.waiting = Some(colour);
        }
        Ok(())
    }

    /// Applies each waiting colour whose entry may change at `tick`.
    pub fn begin_step(&mut self, tick: u64) {
        for held in self.entries.iter_mut().flatten() {
            if let Some(colour) = held.waiting
                && held.allowed_at(tick) <= tick
            {
                held.change(colour, tick);
            }
        }
    }

    /// The tick the earliest waiting colour can be applied at, so that
    /// the decoder wakes the script host for it.
    #[must_use]
    pub fn next_due(&self, tick: u64) -> Option<u64> {
        self.entries.iter().flatten().filter(|held| held.waiting.is_some()).map(|held| held.allowed_at(tick)).min()
    }

    /// How many entries are defined.
    #[must_use]
    pub fn defined(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    /// Whether `entry` is one a cell may be drawn in: every entry of the
    /// Colour Map is, defined by the script or not.
    #[must_use]
    pub fn drawable(entry: i32) -> Option<u8> {
        u8::try_from(entry).ok().filter(|_| entry <= LAST_ENTRY)
    }

    /// The defined entries in rising order, each with the colour it shows.
    #[must_use]
    pub fn shown(&self) -> Vec<(u8, u16)> {
        (FIRST_DEFINABLE_ENTRY..).zip(&self.entries).filter_map(|(entry, held)| held.as_ref().map(|held| (entry, held.shown))).collect()
    }
}

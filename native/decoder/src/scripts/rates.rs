//! The rates the decoder holds over a script host's batches, as the
//! authority (the script API's specification section 10.5; TODO-7293 step
//! 5): the script host holds each itself and tells the author at the
//! call, but "a host that has been subverted cannot be relied on to
//! count", so a batch that passes one here is never applied, and its
//! script host is failed.
//!
//! Each rate is counted over any [`SECOND_TICKS`] consecutive ticks of
//! the page's clock, the tick being the one the batch's step was asked
//! at. Ticks only rise, and a paused page's do not move, so a pause
//! neither spends nor restores a budget.

use std::collections::VecDeque;

use protocol::script::Batch;
use protocol::script::batch::{FIRST_DEFINABLE_ENTRY, MAX_DEFINABLE_COLOURS, TeletextSettings};

use super::limits::{
    MAX_COLOUR_SETS_PER_SECOND, MAX_NAVIGATIONS_PER_SECOND, MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND, MAX_SOUND_OPERATIONS_PER_SECOND, NAPLPS_MAX_BYTES_PER_SECOND,
    NAPLPS_REPAINT_TICKS, SECOND_TICKS,
};

/// The bytes a teletext row takes in a batch: its number and its codes.
const ROW_BYTES: usize = 1 + neoteletext_core::consts::COLUMNS;
/// The bytes of a triplet in a batch: address, mode and data.
const TRIPLET_BYTES: usize = 3;
/// The bytes of a colour in a batch: its entry and its 12 bits.
const COLOUR_BYTES: usize = 3;
/// The bytes of the settings outside their colours.
const SETTINGS_BYTES: usize = 3;
/// The bytes of a sprite outside its frames, and of each frame: the
/// engine's SPRITES record's.
const SPRITE_BYTES: usize = 10;
const FRAME_BYTES: usize = 2;
/// The bytes of a NAPLPS object outside its frames and its label: the
/// engine's `NAPLPS_OBJECTS` record's fixed fields.
const NAPLPS_OBJECT_BYTES: usize = 24;

/// An amount counted over the last second of ticks.
#[derive(Debug, Default)]
struct Window {
    /// The amounts added, by tick, oldest first; never more than a second
    /// of ticks.
    added: VecDeque<(u64, usize)>,
    total: usize,
}

impl Window {
    /// Forgets what was added a second or more before `tick`.
    fn forget(&mut self, tick: u64) {
        while let Some(&(at, amount)) = self.added.front() {
            if at.saturating_add(SECOND_TICKS) > tick {
                break;
            }
            self.total = self.total.saturating_sub(amount);
            self.added.pop_front();
        }
    }

    /// Adds `amount` at `tick` and gives the second's total. One entry a
    /// tick, so the window never holds more than a second's entries.
    fn add(&mut self, tick: u64, amount: usize) -> usize {
        self.forget(tick);
        if amount > 0 {
            match self.added.back_mut() {
                Some((at, held)) if *at == tick => *held = held.saturating_add(amount),
                _ => self.added.push_back((tick, amount)),
            }
            self.total = self.total.saturating_add(amount);
        }
        self.total
    }
}

/// Which rate a batch passed: the bound's name, as the specification's
/// section 10.6 gives it, for the developer log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Passed(pub &'static str);

/// The rates of one channel's scripts.
#[derive(Debug, Default)]
pub struct Rates {
    output: Window,
    naplps: Window,
    sound: Window,
    navigations: Window,
    /// The tick of the last new generation of the page's NAPLPS picture.
    generation: Option<u64>,
    /// Each definable colour's value as last set, and the ticks it was
    /// changed at within the last second.
    colours: [(Option<u16>, Window); MAX_DEFINABLE_COLOURS],
}

/// The bytes of renderer output `batch` carries, of every kind (section
/// 10.5: "renderer output of every kind"): what each drawing section
/// takes on the link, give or take its counts.
#[must_use]
pub fn output_bytes(batch: &Batch) -> usize {
    let rows = batch.teletext_rows.as_ref().map_or(0, |rows| rows.len().saturating_mul(ROW_BYTES));
    let enhancement = batch.teletext_enhancement.as_ref().map_or(0, |triplets| triplets.len().saturating_mul(TRIPLET_BYTES));
    let settings = batch.teletext_settings.as_ref().map_or(0, |settings| SETTINGS_BYTES.saturating_add(settings.colours.len().saturating_mul(COLOUR_BYTES)));
    let objects = batch.teletext_objects.iter().flatten().map(|object| object.triplets.len().saturating_mul(TRIPLET_BYTES).saturating_add(FRAME_BYTES));
    let sprites = batch.sprites.iter().flatten().map(|sprite| sprite.objects.len().saturating_mul(FRAME_BYTES).saturating_add(SPRITE_BYTES));
    let animate = batch.animate.as_ref().map_or(0, |operations| operations.len().saturating_mul(neoteletext_rules::animate::MAX_OPERATION_BYTES));
    let regis = batch.regis.as_ref().map_or(0, Vec::len);
    let naplps = batch.naplps.as_ref().map_or(0, |append| append.bytes.len());
    let cast = batch.naplps_objects.as_ref().map_or(0, |cast| {
        let objects = cast.objects.iter().map(|object| NAPLPS_OBJECT_BYTES.saturating_add(object.frames.len()).saturating_add(object.label.len()));
        objects.fold(cast.title.len(), usize::saturating_add)
    });
    [rows, enhancement, settings, animate, regis, naplps, cast].into_iter().chain(objects).chain(sprites).fold(0, usize::saturating_add)
}

impl Rates {
    /// Counts `batch`, the answer to a step asked at `tick`, against every
    /// rate.
    ///
    /// # Errors
    /// The first rate it passes. The batch is then not applied, and
    /// nothing more of this script host's is.
    pub fn count(&mut self, tick: u64, batch: &Batch) -> Result<(), Passed> {
        if self.output.add(tick, output_bytes(batch)) > MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND {
            return Err(Passed("MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND"));
        }
        if let Some(append) = &batch.naplps {
            if append.new_generation {
                if self.generation.is_some_and(|last| tick.saturating_sub(last) < NAPLPS_REPAINT_TICKS) {
                    return Err(Passed("NAPLPS_REPAINT_TICKS"));
                }
                self.generation = Some(tick);
            }
            if self.naplps.add(tick, append.bytes.len()) > NAPLPS_MAX_BYTES_PER_SECOND {
                return Err(Passed("NAPLPS_MAX_BYTES_PER_SECOND"));
            }
        }
        if self.sound.add(tick, batch.sound.as_ref().map_or(0, Vec::len)) > MAX_SOUND_OPERATIONS_PER_SECOND {
            return Err(Passed("MAX_SOUND_OPERATIONS_PER_SECOND"));
        }
        if self.navigations.add(tick, usize::from(batch.navigate.is_some())) > MAX_NAVIGATIONS_PER_SECOND {
            return Err(Passed("MAX_NAVIGATIONS_PER_SECOND"));
        }
        batch.teletext_settings.as_ref().map_or(Ok(()), |settings| self.colours(tick, settings))
    }

    /// Counts the redefinitions a settings section makes: an entry whose
    /// value differs from the one last set. The section is whole, so an
    /// entry it leaves out is back at its default, which is a value too.
    fn colours(&mut self, tick: u64, settings: &TeletextSettings) -> Result<(), Passed> {
        for (entry, (last, changes)) in (FIRST_DEFINABLE_ENTRY..).zip(&mut self.colours) {
            let now = settings.colours.iter().find(|(named, _)| *named == entry).map(|(_, colour)| *colour);
            if now == *last {
                continue;
            }
            *last = now;
            if changes.add(tick, 1) > MAX_COLOUR_SETS_PER_SECOND {
                return Err(Passed("MAX_COLOUR_SETS_PER_SECOND"));
            }
        }
        Ok(())
    }

    /// A page context ended: the next page's colours and picture start
    /// afresh, and what the channel's scripts spent of a second stays
    /// spent, so that changing page restores no budget.
    pub fn page_ended(&mut self) {
        self.generation = None;
        for (last, _) in &mut self.colours {
            *last = None;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;
    use protocol::script::batch::{MAX_BATCH_BYTES, NaplpsAppend, SoundOperation};

    fn regis(bytes: usize) -> Batch {
        Batch { regis: Some(vec![b' '; bytes]), ..Batch::new(0) }
    }

    /// Section 10.5: `MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND` is held over
    /// any second of ticks, to the byte, and a second later the budget is
    /// whole again.
    #[test]
    fn output_is_held_to_a_second_of_ticks_to_the_byte() {
        let piece = MAX_BATCH_BYTES / 2;
        let pieces = MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND / piece;
        let mut rates = Rates::default();
        for tick in 0..u64::try_from(pieces).unwrap() {
            assert_eq!(rates.count(tick, &regis(piece)), Ok(()), "tick {tick}");
        }
        let left = MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND - pieces * piece;
        assert_eq!(rates.count(20, &regis(left)), Ok(()));
        assert_eq!(rates.count(SECOND_TICKS - 1, &regis(1)), Err(Passed("MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND")));

        let mut rates = Rates::default();
        assert_eq!(rates.count(0, &regis(piece)), Ok(()));
        for tick in 1..u64::try_from(pieces).unwrap() {
            assert_eq!(rates.count(tick, &regis(piece)), Ok(()));
        }
        // Tick 0's bytes leave the second at tick 60, and no sooner.
        assert_eq!(rates.count(SECOND_TICKS, &regis(piece)), Ok(()));
        assert!(rates.count(SECOND_TICKS, &regis(piece)).is_err());
    }

    /// Section 10.5: one NAPLPS generation each `NAPLPS_REPAINT_TICKS`,
    /// and `NAPLPS_MAX_BYTES_PER_SECOND` appended.
    #[test]
    fn naplps_generations_and_bytes_are_held() {
        let generation =
            |bytes: usize, new_generation: bool| Batch { naplps: Some(NaplpsAppend { new_generation, bytes: vec![0x20; bytes] }), ..Batch::new(0) };
        let mut rates = Rates::default();
        assert_eq!(rates.count(5, &generation(10, true)), Ok(()));
        assert_eq!(rates.count(5 + NAPLPS_REPAINT_TICKS - 1, &generation(10, false)), Ok(()), "an append is no generation");
        assert_eq!(rates.count(5 + NAPLPS_REPAINT_TICKS - 1, &generation(10, true)), Err(Passed("NAPLPS_REPAINT_TICKS")));
        let mut rates = Rates::default();
        assert_eq!(rates.count(0, &generation(10, true)), Ok(()));
        assert_eq!(rates.count(NAPLPS_REPAINT_TICKS, &generation(10, true)), Ok(()));
        rates.page_ended();
        assert_eq!(rates.count(NAPLPS_REPAINT_TICKS + 1, &generation(10, true)), Ok(()), "a new page's first generation");

        let mut rates = Rates::default();
        let piece = NAPLPS_MAX_BYTES_PER_SECOND / 2;
        assert_eq!(rates.count(0, &generation(piece, false)), Ok(()));
        assert_eq!(rates.count(1, &generation(piece, false)), Ok(()));
        assert_eq!(rates.count(2, &generation(1, false)), Err(Passed("NAPLPS_MAX_BYTES_PER_SECOND")));
    }

    /// Section 10.5 and the security review's finding S-10: one colour
    /// entry is redefined at most `MAX_COLOUR_SETS_PER_SECOND` times in a
    /// second of ticks; a section that repeats a value redefines nothing.
    #[test]
    fn a_colour_entry_is_redefined_at_most_six_times_a_second() {
        let set =
            |colour: u16| Batch { teletext_settings: Some(TeletextSettings { colours: vec![(17, colour)], ..TeletextSettings::default() }), ..Batch::new(0) };
        let mut rates = Rates::default();
        for change in 0..MAX_COLOUR_SETS_PER_SECOND {
            let tick = u64::try_from(change).unwrap();
            assert_eq!(rates.count(tick, &set(u16::try_from(change).unwrap() + 1)), Ok(()), "change {change}");
            assert_eq!(rates.count(tick, &set(u16::try_from(change).unwrap() + 1)), Ok(()), "the same value again");
        }
        assert_eq!(rates.count(30, &set(0xFFF)), Err(Passed("MAX_COLOUR_SETS_PER_SECOND")));
        // Another entry has its own count; leaving an entry out is a
        // change of it, back to its default.
        let mut rates = Rates::default();
        assert_eq!(rates.count(0, &set(1)), Ok(()));
        let other = Batch { teletext_settings: Some(TeletextSettings { colours: vec![(18, 5)], ..TeletextSettings::default() }), ..Batch::new(0) };
        assert_eq!(rates.count(1, &other), Ok(()));
        assert_eq!(rates.colours[1].1.total, 2, "entry 17 was set and then left out");
        assert_eq!(rates.colours[2].1.total, 1);
    }

    /// Section 10.5: sound operations and navigations a second.
    #[test]
    fn sound_operations_and_navigations_are_held() {
        let sounds = |count: usize| Batch { sound: Some(vec![SoundOperation::Stop { voice: 0 }; count]), ..Batch::new(0) };
        let mut rates = Rates::default();
        let step = protocol::script::batch::MAX_SOUND_OPERATIONS;
        for tick in 0..u64::try_from(MAX_SOUND_OPERATIONS_PER_SECOND / step).unwrap() {
            assert_eq!(rates.count(tick, &sounds(step)), Ok(()));
        }
        assert_eq!(rates.count(10, &sounds(1)), Err(Passed("MAX_SOUND_OPERATIONS_PER_SECOND")));
        let go = Batch { navigate: Some("100".into()), ..Batch::new(0) };
        let mut rates = Rates::default();
        for tick in 0..u64::try_from(MAX_NAVIGATIONS_PER_SECOND).unwrap() {
            assert_eq!(rates.count(tick, &go), Ok(()));
        }
        assert_eq!(rates.count(SECOND_TICKS - 1, &go), Err(Passed("MAX_NAVIGATIONS_PER_SECOND")));
        assert_eq!(rates.count(SECOND_TICKS + 1, &go), Ok(()), "a second after the first two");
    }
}

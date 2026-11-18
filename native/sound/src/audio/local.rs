//! Local sounds (neoteletext-common's `docs/page-rules.md` RULE-60 and
//! RULE-62): a synthesizer sound with no blocks from the server, whose
//! instrument is held here and rendered just in time, on the audio thread,
//! as the mixer asks for its pulled voice's samples, in blocks of
//! `LOCAL_BLOCK_SAMPLES`. A rule's MIDI message waits in a small queue and
//! is applied before the next block, so it waits at most one block.
//!
//! The instruments are the engine's: integer-only, never allocating, with
//! no unsafe code. Everything given to one here is untrusted: its bank
//! images, its setup block and every MIDI message come from a page's
//! channel through its decoder. The trusted core bounded each field; the
//! instruments' own readers bound what they do with them.

use neoteletext_audio::{CHANNELS, MOST_BANKS};
use neoteletext_audio_instruments::local::Local;
use neoteletext_audio_instruments::midi::{Banks, Instrument};
use neoteletext_audio_mix::{Mixer, Pull, SoundKey, meter_level};
use neoteletext_rules::table::{WATCH_ENVELOPE_LEVEL, WATCH_ENVELOPE_STAGE, WATCH_LEVEL, WATCH_VOICES};
use protocol::audio::{LocalStart, MAX_WATCH_SLOTS, WatchSlot};

/// The MIDI messages that wait for a local sound's next block (RULE-62):
/// one arriving at a full queue is dropped and counted.
pub const LOCAL_MIDI_QUEUE: usize = 256;
/// The buffers running whose local sounds took longer to render than the
/// time they fill, after which a view's local sounds are stopped (RULE-62).
pub const LOCAL_OVERRUN_BUFFERS: u32 = 8;
/// The audio channels of a view, as a count of voices.
pub(super) const VOICES: usize = CHANNELS as usize;

/// One MIDI channel voice message of a rule: its status and two data bytes.
type Midi = (u8, u8, u8);

/// One local sound: its instrument, its banks and the messages waiting.
pub(super) struct LocalVoice {
    /// The mixer's key of its pulled sound.
    pub(super) key: SoundKey,
    instrument: Local,
    /// The bank images, bank index 0 first.
    banks: Vec<Vec<u8>>,
    /// The messages waiting, oldest first, in a ring that never grows.
    queue: [Midi; LOCAL_MIDI_QUEUE],
    queue_head: usize,
    queue_length: usize,
    /// Messages that arrived at a full queue.
    pub(super) dropped: u32,
    /// Every note is to be released before the next block: the sound was
    /// stopped or replaced.
    release: bool,
    /// It was stopped, or replaced on its audio channel: its notes ring
    /// out, and it is no longer an open local sound (RULE-97).
    pub(super) ended: bool,
}

/// The sound's banks, as its instrument reads them: no allocation.
fn banks_of(banks: &[Vec<u8>]) -> Banks<'_> {
    let mut held = Banks::none();
    for image in banks.iter().take(MOST_BANKS) {
        held.push(image);
    }
    held
}

impl LocalVoice {
    /// The instrument a start names, made and given its setup, with the
    /// banks given for it; `None` for a family, a variant or an option
    /// this build has no instrument for. This allocates, and is called
    /// before the mixers' lock is taken.
    pub(super) fn new(key: SoundKey, start: &LocalStart, banks: Vec<Vec<u8>>) -> Option<Box<Self>> {
        let instrument = Local::new(start.family, start.variant, start.option, start.split)?;
        let mut voice = Box::new(Self {
            key,
            instrument,
            banks,
            queue: [(0, 0, 0); LOCAL_MIDI_QUEUE],
            queue_head: 0,
            queue_length: 0,
            dropped: 0,
            release: false,
            ended: false,
        });
        let Self { instrument, banks, .. } = &mut *voice;
        instrument.start_with(&banks_of(banks), &start.setup);
        Some(voice)
    }

    /// Queues a rule's MIDI message for the next block (RULE-62).
    pub(super) fn queue(&mut self, message: Midi) {
        let place = self.queue_head.saturating_add(self.queue_length).checked_rem(LOCAL_MIDI_QUEUE).unwrap_or(0);
        match self.queue.get_mut(place).filter(|_| self.queue_length < LOCAL_MIDI_QUEUE) {
            Some(slot) => {
                *slot = message;
                self.queue_length = self.queue_length.saturating_add(1);
            }
            None => self.dropped = self.dropped.saturating_add(1),
        }
    }

    /// The sound was stopped or replaced: every note is released before
    /// the next block, and what waited is forgotten (RULE-60).
    pub(super) fn release(&mut self) {
        self.queue_length = 0;
        self.release = true;
    }

    /// Applies every message waiting, in arrival order, then renders the
    /// next block (RULE-62). On the audio thread: no allocation, no wait.
    fn render(&mut self, block: &mut [i16]) {
        let Self { instrument, banks, queue, queue_head, queue_length, release, .. } = self;
        let banks = banks_of(banks);
        while *queue_length != 0 {
            if let Some(&(status, data1, data2)) = queue.get(*queue_head) {
                instrument.channel_message(status, data1, data2, &banks);
            }
            *queue_head = queue_head.saturating_add(1).checked_rem(LOCAL_MIDI_QUEUE).unwrap_or(0);
            *queue_length = queue_length.saturating_sub(1);
        }
        if core::mem::take(release) {
            instrument.release_all();
        }
        instrument.render(block);
    }

    /// One metered value of the instrument (RULE-31, RULE-33).
    fn meter(&self, kind: u8, part: u8) -> u8 {
        let meter = self.instrument.meter(part);
        match kind {
            WATCH_ENVELOPE_STAGE => meter.stage.code(),
            WATCH_ENVELOPE_LEVEL => meter.level,
            WATCH_VOICES => meter.voices,
            _ => 0,
        }
    }
}

/// The local sounds of one view, by audio channel, and the banks given for
/// the one about to start on each.
#[derive(Default)]
pub(super) struct Locals {
    pub(super) voices: [Option<Box<LocalVoice>>; VOICES],
    pub(super) pending_banks: [Vec<Vec<u8>>; VOICES],
    /// Buffers running whose local sounds overran their time.
    overruns: u32,
    /// The page the local sounds belong to is paused (RULE-111 case 4,
    /// RULE-62): nothing is rendered for them, their notes released and
    /// their instruments kept.
    pub(super) paused: bool,
}

impl Locals {
    /// Pauses every local sound with its notes released, or lets them
    /// render again (RULE-111 case 4). A note held when the page was
    /// paused does not sound on its return.
    pub(super) fn pause(&mut self, paused: bool) {
        if paused && !self.paused {
            for local in self.voices.iter_mut().flatten() {
                local.release();
            }
        }
        self.paused = paused;
    }

    /// Whether any local sound is held.
    pub(super) fn any(&self) -> bool {
        self.voices.iter().any(Option::is_some)
    }

    /// Notes how long rendering one buffer took against the time it
    /// fills, when this view has local sounds; true once it overran for
    /// [`LOCAL_OVERRUN_BUFFERS`] buffers running (RULE-62).
    pub(super) fn overran(&mut self, took_us: u64, fills_us: u64) -> bool {
        if !self.any() || took_us <= fills_us {
            self.overruns = 0;
            return false;
        }
        self.overruns = self.overruns.saturating_add(1);
        if self.overruns < LOCAL_OVERRUN_BUFFERS {
            return false;
        }
        self.overruns = 0;
        true
    }

    /// The values of a watch list, in its order (RULE-33 item 3): a level
    /// from the mixer's peak of that audio channel's voice since the last
    /// report, the envelope's stage and level and the voice count from the
    /// local sound's instrument. The count written.
    pub(super) fn metered(&self, mixer: &mut Mixer, watch: &[WatchSlot], values: &mut [u8; MAX_WATCH_SLOTS]) -> usize {
        // One peak for each audio channel, taken once however many slots
        // read it.
        let mut peaks = [None; VOICES];
        let mut count = 0_usize;
        for (slot, value) in watch.iter().zip(values.iter_mut()) {
            let voice = usize::from(slot.channel);
            *value = if slot.kind == WATCH_LEVEL {
                match peaks.get_mut(voice) {
                    Some(peak) => *peak.get_or_insert_with(|| meter_level(mixer.take_peak(voice))),
                    None => 0,
                }
            } else {
                self.voices.get(voice).and_then(Option::as_ref).map_or(0, |local| local.meter(slot.kind, slot.part))
            };
            count = count.saturating_add(1);
        }
        count
    }
}

impl Pull for Locals {
    fn pull(&mut self, voice: usize, key: SoundKey, block: &mut [i16]) {
        match self.voices.get_mut(voice).and_then(Option::as_mut).filter(|local| local.key == key && !self.paused) {
            Some(local) => local.render(block),
            None => block.fill(0),
        }
    }
}

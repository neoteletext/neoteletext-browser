//! The interface's sound (the engine's area AUD, decisions 37, 38 and 40): the
//! voice operations the trusted core passes on are applied to one engine
//! mixer for each channel, so every tab is heard and each can be muted, and
//! the operating system's output device plays their sum, through `cpal`.
//!
//! The mixers are shared between the thread that reads the trusted core and
//! the device's audio thread behind a mutex. The reader holds it only to
//! apply one operation; the audio thread holds it only to render one
//! buffer, which never allocates or frees. The reader allocates a sound's
//! samples, or a live sound's ring, before it takes the lock and gives the
//! buffer to the mixer whole (finding F-55): allocating the largest sound,
//! two minutes at 48 kHz, under the lock held the audio thread up for as
//! long as the allocator took: at worst 2.0 ms of 20 tries, against
//! 25 µs to apply the definition given its buffer (release build, macOS,
//! 24 October 2026), and a device buffer is about 10 ms. Under the lock
//! the mixer may still grow its small table of sounds, and frees a sound
//! that one of the same key replaces or that was forgotten. The audio
//! thread waits for the lock rather than skipping a buffer: a skipped
//! buffer is a click. What the reader frees itself (a channel's whole
//! mixer, a ring the mixer refused) it frees after letting go.
//!
//! The device runs only while a channel sounds. Whichever thread finds
//! that the channels went from silent to sounding or back wakes a watcher
//! through a channel (decision 46: a program waits on what it needs, never
//! by looking again on a timer); the watcher tells the window, which
//! starts or stops the device. Idle, the browser then costs the audio
//! thread nothing: before this, the device ran all the time and woke the
//! process about a hundred times a second (measured 21 October 2026).
//!
//! The settings' sound and volume (requirement 19.4) are each mixer's
//! master: off silences every channel, and the volume scales them all.
//!
//! A page's local sounds (page-rules.md RULE-60, RULE-62) are instruments
//! held here and rendered on the audio thread, as each mixer asks for its
//! pulled voices' samples (`local.rs`); while one plays, the device runs,
//! so that a note never waits for it to start. What a page's rules watch
//! (RULE-32, RULE-33) is metered after each buffer and told the trusted
//! core once half a tick of frames was rendered: each watched
//! audio channel's peak before the master volume and the mute, and a local
//! sound's envelope and voice count. Neither a condition nor a rule is
//! known here.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};

use neoteletext_audio_mix::{Mixer, Start};
use neoteletext_rules::table::WATCH_LEVEL;
use protocol::audio::{Audio, MAX_WATCH_SLOTS, Sound, WatchSlot};

use local::{LocalVoice, Locals, VOICES};
use names::{Name, Names};

pub mod buffer;
mod device;
mod local;
#[cfg(test)]
mod local_tests;
mod names;
#[cfg(test)]
mod tests;

pub use device::{OutputSample, Speaker, render};
pub use local::{LOCAL_MIDI_QUEUE, LOCAL_OVERRUN_BUFFERS};

/// The reports of the audio thread that wait for the helper's loop: a few
/// buffers' worth for every view; one that finds no room is dropped, and
/// the next takes its place.
pub const TOLD_QUEUE: usize = 64;
/// The parts of a second after which what is watched is reported: half a
/// tick of a page's rules. RULE-33 item 3 says a sixtieth of a second, by
/// which a device buffer of 10 milliseconds is reported only every second
/// time and RULE-65's 45 milliseconds from a key to its level is missed
/// by a buffer; RULE-65 counts on "the level of the device buffer in
/// which the note began [being] reported at that buffer's end". With half
/// a tick every buffer of 400 frames or more at 48 kHz is reported at its
/// end, and smaller buffers are still held to this many reports a second
/// (DEC-6573, TODO-6575).
const REPORTS_PER_SECOND: u32 = HALF_TICKS_PER_TICK * neoteletext_rules::limits::TICKS_PER_SECOND;
/// The halves of a tick.
const HALF_TICKS_PER_TICK: u32 = 2;
/// The parts of a second of silence after which a view whose local sound
/// plays nothing no longer shows as sounding (RULE-62: a view's speaker
/// mark follows what is heard).
const UNHEARD_PER_SECOND: u32 = 2;
/// Microseconds in a second.
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;

/// What the audio thread tells the helper's loop, for the trusted core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Told {
    /// The values of one view's watch list, in its order (RULE-33 item 3).
    Metered {
        /// The view's channel.
        channel: u32,
        /// The values, of which the first `count` are the list's.
        values: [u8; MAX_WATCH_SLOTS],
        /// How many.
        count: usize,
    },
    /// One view's local sounds overran their time and were stopped
    /// (RULE-62).
    Overrun {
        /// The view's channel.
        channel: u32,
    },
    /// A cue of one of a view's sounds was heard (RULE-109): the device
    /// buffer holding its sample has been rendered.
    Cue {
        /// The view's channel.
        channel: u32,
        /// The audio channel.
        voice: u8,
        /// The cue's number.
        number: u8,
    },
    /// The device reported an underrun, or a buffer took longer to fill
    /// than its own time (page-rules.md RULE-97 item 3).
    DeviceTrouble,
    /// The time spent rendering one view's sounds since it was last told
    /// (REQ-6626 item 1 b).
    Spent {
        /// The view's channel.
        channel: u32,
        /// Microseconds.
        microseconds: u32,
    },
}

/// The most frames rendered at once: buffers larger than this are rendered
/// in pieces, so the scratch buffer is allocated once, before playing.
const SCRATCH_FRAMES: usize = 8192;

/// The mixers the reader and the audio thread share.
pub type Shared = Arc<Mutex<Mixers>>;

/// The loudest volume: the engine mixer's master is a percentage.
const MAX_VOLUME: u8 = 100;
const _: () = assert!(MAX_VOLUME == protocol::settings::MAX_VOLUME);

/// One channel's mixer, the keys it knows the decoder's names by, and its
/// local sounds.
struct Channel {
    mixer: Mixer,
    names: Names,
    locals: Locals,
    /// Whether its output was other than silence lately, and for how many
    /// frames it has been silent.
    heard: bool,
    silent_frames: usize,
    /// Microseconds spent rendering it since last told (REQ-6626).
    spent_us: u64,
}

impl Channel {
    fn new(rate_hz: u32) -> Self {
        Self { mixer: Mixer::new(rate_hz), names: Names::default(), locals: Locals::default(), heard: false, silent_frames: 0, spent_us: 0 }
    }

    /// Whether the view's tab shows it sounds: something plays or is about
    /// to, and where a local sound is held, something of it was heard
    /// lately (RULE-62).
    fn sounding(&self) -> bool {
        self.mixer.active() && (!self.locals.any() || self.heard)
    }
}

/// One mixer for each channel with sound, at the output's rate.
pub struct Mixers {
    rate_hz: u32,
    channels: BTreeMap<u32, Channel>,
    /// The settings: page sounds play, and their volume.
    sound: bool,
    volume: u8,
    /// The channels whose tabs are muted.
    muted: BTreeSet<u32>,
    /// Whether any channel sounded when last looked, and how many of the
    /// views with a local sound showed as sounding.
    active: bool,
    shown: usize,
    /// Each view's watch list (RULE-33 item 2), kept apart from its mixer,
    /// so a list given before the view's first sound holds when its mixer
    /// is made.
    watches: BTreeMap<u32, Vec<WatchSlot>>,
    /// The output frames rendered since what is watched was last reported.
    since_report: usize,
    /// Where the audio thread tells what it metered and what it stopped.
    told: Option<SyncSender<Told>>,
    /// Woken when that changes. It holds one wake, and a wake that finds
    /// one waiting is not needed: the watcher looks afresh each time.
    wake: Option<SyncSender<()>>,
}

impl Mixers {
    /// No channel has sound yet; sound is on at full volume until the
    /// settings say otherwise.
    #[must_use]
    pub fn new(rate_hz: u32) -> Self {
        Self {
            rate_hz,
            channels: BTreeMap::new(),
            sound: true,
            volume: MAX_VOLUME,
            muted: BTreeSet::new(),
            active: false,
            shown: 0,
            watches: BTreeMap::new(),
            since_report: 0,
            told: None,
            wake: None,
        }
    }

    /// The views with a local sound open, paused or not (RULE-97: a paused
    /// page's kept instrument counts as open).
    #[must_use]
    pub fn views_with_locals(&self) -> Vec<u32> {
        self.channels.iter().filter(|(_, channel)| channel.locals.voices.iter().flatten().any(|voice| !voice.ended)).map(|(&number, _)| number).collect()
    }

    /// The local sounds open in every view.
    #[must_use]
    pub fn locals_open(&self) -> usize {
        self.channels.values().map(|channel| channel.locals.voices.iter().flatten().filter(|voice| !voice.ended).count()).sum()
    }

    /// Gives the audio thread where to tell what it metered and stopped.
    pub fn tell(&mut self, told: SyncSender<Told>) {
        self.told = Some(told);
    }

    /// Whether the output device is to run: a channel sounds or is about
    /// to. A local sound keeps it running while it plays, heard or not, so
    /// that a note never waits for the device to start (RULE-62).
    #[must_use]
    pub fn device_wanted(&self) -> bool {
        self.channels.values().any(|channel| channel.mixer.active())
    }

    /// Wakes the watcher when the channels went from silent to sounding or
    /// back. Called with the lock held, on the audio thread too: it never
    /// blocks or allocates.
    fn note_activity(&mut self) {
        let active = self.channels.values().any(|channel| channel.mixer.active());
        // A view with a local sound shows as sounding only while it is
        // heard: its tab's mark changes while the device runs on.
        let shown = self.channels.values().filter(|channel| channel.locals.any() && channel.sounding()).count();
        if active == self.active && shown == self.shown {
            return;
        }
        self.active = active;
        self.shown = shown;
        if let Some(wake) = &self.wake
            && let Err(TrySendError::Disconnected(())) = wake.try_send(())
        {
            self.wake = None;
        }
    }

    /// Each channel's master: the volume, silent when sound is off or the
    /// tab is muted.
    fn master(&mut self, channel: u32) {
        let silent = !self.sound || self.muted.contains(&channel);
        if let Some(Channel { mixer, .. }) = self.channels.get_mut(&channel) {
            mixer.set_master(self.volume, silent);
        }
    }

    /// The channels sounding or about to, muted or not: their tabs show it.
    #[must_use]
    pub fn sounding(&self) -> BTreeSet<u32> {
        self.channels.iter().filter(|(_, channel)| channel.sounding()).map(|(&number, _)| number).collect()
    }

    /// Keeps the peak of exactly the audio channels of `channel` whose
    /// level its watch list reads (RULE-32: nothing is metered for a
    /// channel no rule watches).
    fn meter(&mut self, channel: u32) {
        let Some(Channel { mixer, .. }) = self.channels.get_mut(&channel) else { return };
        let watch = self.watches.get(&channel).map_or(&[][..], Vec::as_slice);
        for (voice, number) in (0..VOICES).zip(0_u8..) {
            let watched = watch.iter().any(|slot| slot.kind == WATCH_LEVEL && slot.channel == number);
            if mixer.is_metered(voice) != watched {
                mixer.set_metered(voice, watched);
            }
        }
    }

    /// Tells the helper's loop, without waiting: a report that finds no
    /// room is dropped.
    fn tell_loop(&mut self, told: Told) {
        if let Some(sender) = &self.told
            && let Err(TrySendError::Disconnected(_)) = sender.try_send(told)
        {
            self.told = None;
        }
    }

    /// One report of zeros for each view with a watch list: the output
    /// device stopped, and nothing more is reported until it runs again
    /// (RULE-33 item 3).
    pub fn report_silence(&mut self) {
        let silent: Vec<(u32, usize)> = self.watches.iter().map(|(&channel, watch)| (channel, watch.len())).collect();
        for (channel, count) in silent {
            self.tell_loop(Told::Metered { channel, values: [0; MAX_WATCH_SLOTS], count });
        }
    }

    /// Reports what each view's watch list reads, once half a tick of
    /// output frames was rendered since the last report (RULE-33 item 3,
    /// as [`REPORTS_PER_SECOND`] reads it). On the audio thread: it never allocates or waits.
    fn report_watched(&mut self, frames: usize) {
        self.since_report = self.since_report.saturating_add(frames);
        let due = usize::try_from(self.rate_hz.checked_div(REPORTS_PER_SECOND).unwrap_or(0)).unwrap_or(usize::MAX);
        if self.since_report < due {
            return;
        }
        self.since_report = 0;
        let Self { watches, channels, told, .. } = self;
        let Some(sender) = told else { return };
        // What each view's sounds cost, told as often as what is watched
        // (REQ-6626 item 1 b).
        for (&channel, held) in channels.iter_mut().filter(|(_, held)| held.spent_us != 0) {
            let microseconds = u32::try_from(core::mem::take(&mut held.spent_us)).unwrap_or(u32::MAX);
            let _ = sender.try_send(Told::Spent { channel, microseconds });
        }
        for (&channel, watch) in watches.iter() {
            let mut values = [0; MAX_WATCH_SLOTS];
            let count = match channels.get_mut(&channel) {
                Some(Channel { mixer, locals, .. }) => locals.metered(mixer, watch, &mut values),
                None => watch.len().min(MAX_WATCH_SLOTS),
            };
            let _ = sender.try_send(Told::Metered { channel, values, count });
        }
    }

    /// Renders the sum of every channel into `out`, using `scratch`, as long
    /// as `out`, for each. It never allocates or frees. A channel with
    /// nothing playing or about to is silent and is not rendered: a mixer
    /// costs its work for every sample whether or not it sounds, so a few
    /// hundred tabs that once played a sound would otherwise take a whole
    /// processor core on the audio thread.
    fn render(&mut self, out: &mut [i16], scratch: &mut [i16]) {
        out.fill(0);
        let Some(scratch) = scratch.get_mut(..out.len()) else { return };
        let frames = u64::try_from(out.len()).unwrap_or(u64::MAX);
        let fills_us = frames.saturating_mul(MICROSECONDS_PER_SECOND).checked_div(u64::from(self.rate_hz)).unwrap_or(0);
        let unheard_after = usize::try_from(self.rate_hz.checked_div(UNHEARD_PER_SECOND).unwrap_or(0)).unwrap_or(usize::MAX);
        let mut overran = None;
        for (&number, channel) in self.channels.iter_mut().filter(|(_, channel)| channel.mixer.active()) {
            let Channel { mixer, locals, heard, silent_frames, spent_us, .. } = channel;
            // Every view is timed (REQ-6626); one with local sounds is held
            // to the time of the buffer its instruments fill (RULE-62).
            let started = std::time::Instant::now();
            mixer.render_pulling(scratch, locals);
            for (sum, &sample) in out.iter_mut().zip(scratch.iter()) {
                *sum = sum.saturating_add(sample);
            }
            // The cues whose samples this buffer holds were heard
            // (RULE-109): told after the buffer, before the next.
            if let Some(sender) = &self.told {
                mixer.take_heard_cues(&mut |cue| {
                    let voice = u8::try_from(cue.voice).unwrap_or(u8::MAX);
                    let _ = sender.try_send(Told::Cue { channel: number, voice, number: cue.number });
                });
            }
            *spent_us = spent_us.saturating_add(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
            if !locals.any() {
                continue;
            }
            if scratch.iter().any(|&sample| sample != 0) {
                *heard = true;
                *silent_frames = 0;
            } else {
                *silent_frames = silent_frames.saturating_add(scratch.len());
                *heard &= *silent_frames < unheard_after;
            }
            let took_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
            if locals.overran(took_us, fills_us) {
                // Stopped with all notes released, and reported (RULE-62).
                for (voice, local) in locals.voices.iter_mut().enumerate() {
                    if let Some(local) = local {
                        local.release();
                        mixer.stop(voice, neoteletext_audio::MIN_FADE_MS);
                    }
                }
                overran = Some(number);
            }
        }
        if let Some(channel) = overran {
            self.tell_loop(Told::Overrun { channel });
        }
        self.report_watched(out.len());
        // A sound ends here, on the audio thread.
        self.note_activity();
    }
}

/// Waits for the channels to go from silent to sounding or back, and tells
/// `changed` which channels sound, muted or not, each time that set
/// changes. Returns when the mixers are gone or `changed` says to stop.
pub fn watch(mixers: &Mutex<Mixers>, wakes: &Receiver<()>, mut changed: impl FnMut(BTreeSet<u32>) -> bool) {
    let mut last = (BTreeSet::new(), false);
    while wakes.recv().is_ok() {
        let now = {
            let mixers = mixers.lock().unwrap_or_else(PoisonError::into_inner);
            (mixers.sounding(), mixers.device_wanted())
        };
        // Told too when only the device is wanted or no longer is: a local
        // sound that plays nothing yet keeps it running (RULE-62).
        if now != last {
            last.clone_from(&now);
            if !changed(now.0) {
                return;
            }
        }
    }
}

/// The settings' sound and volume, for every channel (requirement 19.4).
pub fn set_sound(mixers: &Mutex<Mixers>, sound: bool, volume: u8) {
    let mut mixers = mixers.lock().unwrap_or_else(PoisonError::into_inner);
    mixers.sound = sound;
    mixers.volume = volume.min(MAX_VOLUME);
    let channels: Vec<u32> = mixers.channels.keys().copied().collect();
    for channel in channels {
        mixers.master(channel);
    }
}

/// Applies one voice operation of `channel` from the trusted core: its
/// buffer is allocated before the lock is taken, and what it leaves to
/// free is freed after the lock is let go (finding F-55).
pub fn apply(mixers: &Mutex<Mixers>, channel: u32, audio: Audio) {
    let operation = match audio {
        // A local sound's instrument is made and given its setup before
        // the lock is taken for it, with the banks given for it, which
        // only this thread writes.
        Audio::Local { channel: voice, start } => {
            let banks = mixers.lock().unwrap_or_else(PoisonError::into_inner).take_banks(channel, voice);
            let local = LocalVoice::new((0, 0), &start, banks);
            Operation::Local { voice, start, ring: vec![0; Mixer::pulled_ring()], local }
        }
        other => Operation::prepare(other),
    };
    let mut locked = mixers.lock().unwrap_or_else(PoisonError::into_inner);
    let Leftover { mixer, buffer, local } = locked.apply(channel, operation);
    drop(locked);
    drop(mixer);
    drop(buffer);
    drop(local);
}

/// A voice operation from the trusted core with the buffer it gives the
/// mixer, allocated before the lock is taken.
enum Operation {
    /// [`Audio::Define`], with its silent samples.
    Define { sound: Sound, rate_hz: u32, samples: Vec<i16> },
    /// [`Audio::Stream`], with its ring.
    Stream { channel: u8, sound: Sound, rate_hz: u32, ring: Vec<i16> },
    /// [`Audio::Local`], with its pulled sound's ring and its instrument,
    /// made and set up; none for a family this build has no instrument of.
    Local { voice: u8, start: protocol::audio::LocalStart, ring: Vec<i16>, local: Option<Box<LocalVoice>> },
    /// Every other operation, which needs no buffer.
    Other(Audio),
}

impl Operation {
    /// Allocates the buffer `audio` needs: a definition's length is
    /// bounded by the protocol (`MAX_SOUND_SAMPLES`), and a ring is the
    /// length the mixer takes.
    fn prepare(audio: Audio) -> Self {
        match audio {
            Audio::Define { sound, rate_hz, samples } => Self::Define { sound, rate_hz, samples: vec![0; usize::try_from(samples).unwrap_or(0)] },
            Audio::Stream { channel, sound, rate_hz } => Self::Stream { channel, sound, rate_hz, ring: vec![0; Mixer::stream_ring(rate_hz)] },
            other => Self::Other(other),
        }
    }
}

/// What an operation leaves to be freed once the lock is let go.
#[derive(Default)]
struct Leftover {
    /// A channel's mixer, with every sound it held.
    mixer: Option<Channel>,
    /// A ring the mixer refused, or a buffer for a name beyond the
    /// channel's bound.
    buffer: Option<Vec<i16>>,
    /// A local sound's instrument that another replaced, or that could
    /// not be started.
    local: Option<Box<LocalVoice>>,
}

impl Mixers {
    /// The banks given for the local sound about to start on audio channel
    /// `voice` of `channel`, taken: it allocates nothing.
    fn take_banks(&mut self, channel: u32, voice: u8) -> Vec<Vec<u8>> {
        self.channels.get_mut(&channel).and_then(|held| held.locals.pending_banks.get_mut(usize::from(voice))).map(core::mem::take).unwrap_or_default()
    }

    /// Applies one voice operation of `channel`, with the lock held. Only
    /// a definition or a live sound makes the channel's mixer (finding
    /// F-55): any other operation on a channel without one does nothing,
    /// so the trusted core's stream cannot grow the mixers, which the
    /// audio thread walks for every buffer, with channels that never
    /// sound.
    #[must_use]
    fn apply(&mut self, channel: u32, operation: Operation) -> Leftover {
        match operation {
            // The channel is gone. Its mixer, with every sound it held, is
            // freed once the audio thread can have the lock again.
            Operation::Other(Audio::Reset) => {
                let mixer = self.channels.remove(&channel);
                self.muted.remove(&channel);
                self.watches.remove(&channel);
                self.note_activity();
                return Leftover { mixer, buffer: None, local: None };
            }
            // Kept apart from the mixer, so a mute given before the
            // channel's first sound holds when its mixer is made.
            Operation::Other(Audio::Mute(muted)) => {
                if muted {
                    self.muted.insert(channel);
                } else {
                    self.muted.remove(&channel);
                }
                self.master(channel);
                return Leftover::default();
            }
            // The page the view's local sounds belong to is paused, or
            // open again (RULE-111 case 4): its notes released and its
            // cue marks forgotten (RULE-109).
            Operation::Other(Audio::Pause(paused)) => {
                if let Some(held) = self.channels.get_mut(&channel) {
                    held.locals.pause(paused);
                    if paused {
                        let keys: Vec<_> = held.names.keys().collect();
                        for key in keys {
                            held.mixer.forget_cues(key);
                        }
                    }
                }
                return Leftover::default();
            }
            // The view's watch list, replacing the one before (RULE-33
            // item 2): kept apart from the mixer, as a mute is.
            Operation::Other(Audio::Watch(slots)) => {
                if slots.is_empty() {
                    self.watches.remove(&channel);
                } else {
                    self.watches.insert(channel, slots);
                }
                // A tick of frames from now, the new list's first report.
                self.since_report = 0;
                self.meter(channel);
                return Leftover::default();
            }
            Operation::Define { .. } | Operation::Stream { .. } | Operation::Local { .. } | Operation::Other(Audio::LocalBank { .. }) => {
                if let std::collections::btree_map::Entry::Vacant(vacant) = self.channels.entry(channel) {
                    // A new mixer holds no sound yet, so it allocates nothing.
                    vacant.insert(Channel::new(self.rate_hz));
                    self.master(channel);
                    self.meter(channel);
                }
            }
            Operation::Other(_) => {}
        }
        let leftover = match self.channels.get_mut(&channel) {
            Some(held) => held.apply(operation),
            None => Leftover::default(),
        };
        // A sound starts here, on the reader's thread, while the device
        // may be stopped.
        self.note_activity();
        leftover
    }
}

impl Channel {
    /// Applies one voice operation to this channel's mixer.
    #[must_use]
    fn apply(&mut self, operation: Operation) -> Leftover {
        let Self { mixer, names, locals, .. } = self;
        let mut leftover = Leftover::default();
        match operation {
            Operation::Define { sound, rate_hz, samples } => match names.keep(Name::Stored(sound)) {
                Some(key) => mixer.define_with(key, rate_hz, samples),
                None => leftover.buffer = Some(samples),
            },
            // A stream under a name still held replaces it (the engine's
            // INT-37): the old one's voice plays out what it holds.
            Operation::Stream { channel: voice, sound, rate_hz, ring } => match names.renew(Name::Stream(voice, sound)) {
                Some((key, old)) => {
                    if let Some(old) = old {
                        mixer.forget(old);
                    }
                    if let Err(ring) = mixer.stream_with(key, rate_hz, ring) {
                        names.release(Name::Stream(voice, sound));
                        leftover.buffer = Some(ring);
                    }
                }
                None => leftover.buffer = Some(ring),
            },
            Operation::Local { voice, start, ring, local } => return self.start_local(voice, &start, ring, local),
            Operation::Other(audio) => match audio {
                // One bank of the local sound about to start on an audio
                // channel: the banks of a start come in index order.
                Audio::LocalBank { channel: voice, index, image, .. } => {
                    if let Some(banks) = locals.pending_banks.get_mut(usize::from(voice)) {
                        banks.truncate(usize::from(index));
                        banks.push(image);
                    }
                }
                // A rule's MIDI message, for the local sound's next block
                // (RULE-62).
                Audio::LocalMidi { channel: voice, status, data1, data2 } => {
                    if let Some(local) = locals.voices.get_mut(usize::from(voice)).and_then(Option::as_mut) {
                        local.queue((status, data1, data2));
                    }
                }
                Audio::Samples { sound, first, pcm } => {
                    if let Some(key) = names.get(Name::Stored(sound)) {
                        mixer.samples(key, usize::try_from(first).unwrap_or(usize::MAX), &pcm);
                    }
                }
                // The sound and every stream of it end; what plays them
                // plays out what it holds.
                Audio::Forget(sound) => {
                    for key in names.release_sound(sound) {
                        mixer.forget(key);
                    }
                }
                // A live sound forgotten plays what it holds, then ends,
                // and the sound queued after it on the voice starts.
                Audio::EndStream { channel: voice, sound } => {
                    if let Some(key) = names.release(Name::Stream(voice, sound)) {
                        mixer.forget(key);
                    }
                }
                Audio::Start { channel: voice, sound, gain, fade_ms, looped, duck_percent, after_current } => {
                    if let Some(key) = names.started(voice, sound) {
                        // What replaces a local sound ends it, its notes
                        // released as it ramps out.
                        if let Some(local) = locals.voices.get_mut(usize::from(voice)).and_then(Option::as_mut).filter(|local| local.key != key) {
                            local.release();
                            local.ended = true;
                        }
                        mixer.start(usize::from(voice), Start { key, gain, fade_ms, looped, duck_percent, after_current });
                    }
                }
                // A stop releases every note of a local sound (RULE-60).
                Audio::Stop { channel: voice, fade_ms } => {
                    if let Some(local) = locals.voices.get_mut(usize::from(voice)).and_then(Option::as_mut) {
                        local.release();
                        local.ended = true;
                    }
                    mixer.stop(usize::from(voice), fade_ms);
                }
                Audio::Gain { channel: voice, gain, fade_ms } => mixer.gain(usize::from(voice), gain, fade_ms),
                // A cue mark of the sound streaming on an audio channel
                // (RULE-109): a mark past the mixer's bound is dropped and
                // counted by the mixer.
                Audio::Cue { channel: voice, sound, position, number } => {
                    if let Some(key) = names.get(Name::Stream(voice, sound)) {
                        let _ = mixer.stream_cue(key, position, number);
                    }
                }
                Audio::StreamSamples { channel: voice, sound, pcm } => {
                    if let Some(key) = names.get(Name::Stream(voice, sound)) {
                        mixer.stream_samples(key, &pcm);
                    }
                }
                // Prepared with their buffers, or applied above.
                Audio::Define { .. } | Audio::Stream { .. } | Audio::Mute(_) | Audio::Reset | Audio::Local { .. } | Audio::Watch(_) | Audio::Pause(_) => {}
            },
        }
        leftover
    }

    /// A local sound starts (RULE-60): its pulled voice replaces what the
    /// audio channel played, and the instrument it had.
    #[must_use]
    fn start_local(&mut self, voice: u8, start: &protocol::audio::LocalStart, ring: Vec<i16>, local: Option<Box<LocalVoice>>) -> Leftover {
        let Self { mixer, names, locals, .. } = self;
        let mut leftover = Leftover::default();
        let place = locals.voices.get_mut(usize::from(voice));
        match (local, place, names.renew(Name::Stream(voice, start.sound))) {
            (Some(mut local), Some(place), Some((key, old))) => {
                if let Some(old) = old {
                    mixer.forget(old);
                }
                match mixer.pulled_with(key, ring) {
                    Ok(()) => {
                        local.key = key;
                        leftover.local = place.replace(local);
                        let begin =
                            Start { key, gain: start.gain, fade_ms: start.fade_ms, looped: false, duck_percent: start.duck_percent, after_current: false };
                        mixer.start(usize::from(voice), begin);
                    }
                    Err(ring) => {
                        names.release(Name::Stream(voice, start.sound));
                        leftover.buffer = Some(ring);
                        leftover.local = Some(local);
                    }
                }
            }
            (local, _, _) => {
                leftover.buffer = Some(ring);
                leftover.local = local;
            }
        }
        leftover
    }
}

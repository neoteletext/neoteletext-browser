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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};

use neoteletext_audio_mix::{Mixer, SoundKey, Start};
use protocol::audio::{Audio, Sound};

mod device;
#[cfg(test)]
mod tests;

pub use device::{OutputSample, Speaker, render};

/// The most frames rendered at once: buffers larger than this are rendered
/// in pieces, so the scratch buffer is allocated once, before playing.
const SCRATCH_FRAMES: usize = 8192;

/// The mixers the reader and the audio thread share.
pub type Shared = Arc<Mutex<Mixers>>;

fn key(sound: Sound) -> SoundKey {
    (sound.handle, sound.generation)
}

/// The loudest volume: the engine mixer's master is a percentage.
const MAX_VOLUME: u8 = 100;
const _: () = assert!(MAX_VOLUME == protocol::settings::MAX_VOLUME);

/// One mixer for each channel with sound, at the output's rate.
pub struct Mixers {
    rate_hz: u32,
    channels: BTreeMap<u32, Mixer>,
    /// The settings: page sounds play, and their volume.
    sound: bool,
    volume: u8,
    /// The channels whose tabs are muted.
    muted: BTreeSet<u32>,
    /// Whether any channel sounded when last looked.
    active: bool,
    /// Woken when that changes. It holds one wake, and a wake that finds
    /// one waiting is not needed: the watcher looks afresh each time.
    wake: Option<SyncSender<()>>,
}

impl Mixers {
    /// No channel has sound yet; sound is on at full volume until the
    /// settings say otherwise.
    #[must_use]
    pub fn new(rate_hz: u32) -> Self {
        Self { rate_hz, channels: BTreeMap::new(), sound: true, volume: MAX_VOLUME, muted: BTreeSet::new(), active: false, wake: None }
    }

    /// Wakes the watcher when the channels went from silent to sounding or
    /// back. Called with the lock held, on the audio thread too: it never
    /// blocks or allocates.
    fn note_activity(&mut self) {
        let active = self.channels.values().any(Mixer::active);
        if active == self.active {
            return;
        }
        self.active = active;
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
        if let Some(mixer) = self.channels.get_mut(&channel) {
            mixer.set_master(self.volume, silent);
        }
    }

    /// The channels sounding or about to, muted or not: their tabs show it.
    #[must_use]
    pub fn sounding(&self) -> BTreeSet<u32> {
        self.channels.iter().filter(|(_, mixer)| mixer.active()).map(|(&channel, _)| channel).collect()
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
        for mixer in self.channels.values_mut().filter(|mixer| mixer.active()) {
            mixer.render(scratch);
            for (sum, &sample) in out.iter_mut().zip(scratch.iter()) {
                *sum = sum.saturating_add(sample);
            }
        }
        // A sound ends here, on the audio thread.
        self.note_activity();
    }
}

/// Waits for the channels to go from silent to sounding or back, and tells
/// `changed` which channels sound, muted or not, each time that set
/// changes. Returns when the mixers are gone or `changed` says to stop.
pub fn watch(mixers: &Mutex<Mixers>, wakes: &Receiver<()>, mut changed: impl FnMut(BTreeSet<u32>) -> bool) {
    let mut last = BTreeSet::new();
    while wakes.recv().is_ok() {
        let sounding = mixers.lock().unwrap_or_else(PoisonError::into_inner).sounding();
        if sounding != last {
            last.clone_from(&sounding);
            if !changed(sounding) {
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
    let operation = Operation::prepare(audio);
    let mut locked = mixers.lock().unwrap_or_else(PoisonError::into_inner);
    let Leftover { mixer, buffer } = locked.apply(channel, operation);
    drop(locked);
    drop(mixer);
    drop(buffer);
}

/// A voice operation from the trusted core with the buffer it gives the
/// mixer, allocated before the lock is taken.
enum Operation {
    /// [`Audio::Define`], with its silent samples.
    Define { sound: Sound, rate_hz: u32, samples: Vec<i16> },
    /// [`Audio::Stream`], with its ring.
    Stream { sound: Sound, rate_hz: u32, ring: Vec<i16> },
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
            Audio::Stream { sound, rate_hz } => Self::Stream { sound, rate_hz, ring: vec![0; Mixer::stream_ring(rate_hz)] },
            other => Self::Other(other),
        }
    }
}

/// What an operation leaves to be freed once the lock is let go.
#[derive(Default)]
struct Leftover {
    /// A channel's mixer, with every sound it held.
    mixer: Option<Mixer>,
    /// A ring the mixer refused.
    buffer: Option<Vec<i16>>,
}

impl Mixers {
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
                self.note_activity();
                return Leftover { mixer, buffer: None };
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
            Operation::Define { .. } | Operation::Stream { .. } => {
                if let std::collections::btree_map::Entry::Vacant(vacant) = self.channels.entry(channel) {
                    // A new mixer holds no sound yet, so it allocates nothing.
                    vacant.insert(Mixer::new(self.rate_hz));
                    self.master(channel);
                }
            }
            Operation::Other(_) => {}
        }
        let Some(mixer) = self.channels.get_mut(&channel) else { return Leftover::default() };
        let mut leftover = Leftover::default();
        match operation {
            Operation::Define { sound, rate_hz, samples } => mixer.define_with(key(sound), rate_hz, samples),
            Operation::Stream { sound, rate_hz, ring } => leftover.buffer = mixer.stream_with(key(sound), rate_hz, ring).err(),
            Operation::Other(audio) => match audio {
                Audio::Samples { sound, first, pcm } => mixer.samples(key(sound), usize::try_from(first).unwrap_or(usize::MAX), &pcm),
                Audio::Forget(sound) => mixer.forget(key(sound)),
                Audio::Start { channel, sound, gain, fade_ms, looped, duck_percent, after_current } => {
                    mixer.start(usize::from(channel), Start { key: key(sound), gain, fade_ms, looped, duck_percent, after_current });
                }
                Audio::Stop { channel, fade_ms } => mixer.stop(usize::from(channel), fade_ms),
                Audio::Gain { channel, gain, fade_ms } => mixer.gain(usize::from(channel), gain, fade_ms),
                Audio::StreamSamples { sound, pcm } => mixer.stream_samples(key(sound), &pcm),
                // Prepared with their buffers, or applied above.
                Audio::Define { .. } | Audio::Stream { .. } | Audio::Mute(_) | Audio::Reset => {}
            },
        }
        // A sound starts here, on the reader's thread, while the device
        // may be stopped.
        self.note_activity();
        leftover
    }
}

//! The sound of a run: the sound helper's own mixer
//! (`sound::audio::Mixers`), given the voice operations the page's own
//! records cause and those a script's batch asks for, and rendered one
//! tick's samples at a time into a WAV file (script-api.md section 16).
//! No device is opened and no clock read: a tick is always
//! [`SAMPLES_PER_TICK`] samples.
//!
//! Turning a batch's sound operations into voice operations is the
//! decoder's in the browser (work package F4): the mapping here is the
//! runner's own until that exists, and is the plain one, each operation
//! to the voice operation of the same meaning.

use std::sync::Mutex;

use protocol::audio::{Audio, LocalStart, Sound};
use protocol::script::batch::SoundOperation;
use sound::audio::{Mixers, apply, render};

use crate::limits::{BYTES_PER_SAMPLE, SAMPLE_RATE_HZ, SAMPLES_PER_TICK, WAV_HEADER_BYTES};

/// The view a run's sounds play in: the runner has one.
const VIEW: u32 = 1;
/// The fade a script's start or stop is given, in milliseconds: the audio
/// profile's shortest, 5 ms.
const FADE_MS: u16 = 5;
/// The level the other voices keep while one plays: all of it.
const NO_DUCKING: u8 = 100;
/// The frames the mixer's scratch holds: one tick's.
const SCRATCH_FRAMES: usize = SAMPLES_PER_TICK;
/// WAV's format code for linear PCM, its channel count here and its bits
/// a sample.
const WAV_PCM: u16 = 1;
/// See [`WAV_PCM`].
const WAV_CHANNELS: u16 = 1;
/// See [`WAV_PCM`].
const WAV_BITS: u16 = 16;
const _: () = assert!(WAV_BITS as usize == BYTES_PER_SAMPLE * 8);
/// The bytes of the format chunk's body.
const WAV_FORMAT_BYTES: u32 = 16;

/// The mixer and everything it has rendered.
pub struct Speaker {
    mixers: Mutex<Mixers>,
    scratch: Vec<i32>,
    samples: Vec<i16>,
    /// The channel's sounds as the page's records defined them, so that a
    /// script's `play` names the generation the mixer holds.
    defined: Vec<Sound>,
    /// The loudest sample so far.
    pub peak: u16,
    /// The voice operations given to the mixer.
    pub operations: u64,
}

impl Default for Speaker {
    fn default() -> Self {
        Self::new()
    }
}

impl Speaker {
    /// A silent mixer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            mixers: Mutex::new(Mixers::new(SAMPLE_RATE_HZ)),
            scratch: vec![0; SCRATCH_FRAMES.saturating_mul(2)],
            samples: Vec::new(),
            defined: Vec::new(),
            peak: 0,
            operations: 0,
        }
    }

    /// A voice operation from the page's own records.
    pub fn page(&mut self, audio: Audio) {
        match &audio {
            Audio::Define { sound, .. } => {
                self.defined.retain(|held| held.handle != sound.handle);
                self.defined.push(*sound);
            }
            Audio::Forget(sound) => self.defined.retain(|held| held != sound),
            _ => {}
        }
        self.operations = self.operations.saturating_add(1);
        apply(&self.mixers, VIEW, audio);
    }

    /// The handles of the sounds the channel has announced, rising: what
    /// a context's opening lists.
    #[must_use]
    pub fn handles(&self) -> Vec<u8> {
        let mut handles: Vec<u8> = self.defined.iter().map(|sound| sound.handle).collect();
        handles.sort_unstable();
        handles.dedup();
        handles
    }

    /// A script's sound operation.
    pub fn script(&mut self, operation: SoundOperation) {
        let audio = match operation {
            SoundOperation::Play { voice, sound, gain } => {
                // A sound the channel never announced plays nothing.
                let Some(sound) = self.defined.iter().find(|held| held.handle == sound).copied() else { return };
                Audio::Start { channel: voice, sound, gain, fade_ms: FADE_MS, looped: false, duck_percent: NO_DUCKING, after_current: false }
            }
            SoundOperation::Stop { voice } => Audio::Stop { channel: voice, fade_ms: FADE_MS },
            SoundOperation::SynthStart { part, family, variant, gain } => Audio::Local {
                channel: part,
                start: LocalStart {
                    sound: Sound { handle: part, generation: 0 },
                    family,
                    variant,
                    option: 0,
                    split: 0,
                    gain,
                    fade_ms: FADE_MS,
                    duck_percent: NO_DUCKING,
                    setup: Vec::new(),
                },
            },
            SoundOperation::Midi { part, status, data1, data2 } => Audio::LocalMidi { channel: part, status, data1, data2 },
            SoundOperation::SynthStop { part } => Audio::Stop { channel: part, fade_ms: FADE_MS },
        };
        self.operations = self.operations.saturating_add(1);
        apply(&self.mixers, VIEW, audio);
    }

    /// Renders one tick's samples.
    pub fn tick(&mut self) {
        let start = self.samples.len();
        self.samples.resize(start.saturating_add(SAMPLES_PER_TICK), 0);
        if let Some(block) = self.samples.get_mut(start..) {
            self.peak = self.peak.max(render(&self.mixers, block, usize::from(WAV_CHANNELS), &mut self.scratch));
        }
    }

    /// Everything rendered, as a WAV file: mono, 16-bit, 48 kHz.
    #[must_use]
    pub fn wav(&self) -> Vec<u8> {
        let data_bytes = u32::try_from(self.samples.len().saturating_mul(BYTES_PER_SAMPLE)).unwrap_or(u32::MAX);
        let frame_bytes = WAV_CHANNELS.saturating_mul(WAV_BITS / 8);
        let mut out = Vec::with_capacity(self.samples.len().saturating_mul(BYTES_PER_SAMPLE).saturating_add(WAV_HEADER_BYTES));
        out.extend_from_slice(b"RIFF");
        // The RIFF chunk holds everything after its own eight bytes.
        out.extend_from_slice(&data_bytes.saturating_add(u32::try_from(WAV_HEADER_BYTES.saturating_sub(8)).unwrap_or(0)).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&WAV_FORMAT_BYTES.to_le_bytes());
        out.extend_from_slice(&WAV_PCM.to_le_bytes());
        out.extend_from_slice(&WAV_CHANNELS.to_le_bytes());
        out.extend_from_slice(&SAMPLE_RATE_HZ.to_le_bytes());
        out.extend_from_slice(&SAMPLE_RATE_HZ.saturating_mul(u32::from(frame_bytes)).to_le_bytes());
        out.extend_from_slice(&frame_bytes.to_le_bytes());
        out.extend_from_slice(&WAV_BITS.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_bytes.to_le_bytes());
        debug_assert_eq!(out.len(), WAV_HEADER_BYTES);
        for sample in &self.samples {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }
}

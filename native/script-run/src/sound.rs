//! The sound of a run: the sound helper's own mixer
//! (`sound::audio::Mixers`), given the voice operations the decoder's
//! engine gives, those of the page's own records and those a script's
//! batch asks for as the decoder carries them out, and rendered one
//! tick's samples at a time into a WAV file (script-api.md section 16).
//! No device is opened and no clock read: a tick is always
//! [`SAMPLES_PER_TICK`] samples.

use std::sync::Mutex;

use protocol::audio::Audio;
use sound::audio::{Mixers, apply, render};

use crate::limits::{BYTES_PER_SAMPLE, SAMPLE_RATE_HZ, SAMPLES_PER_TICK, WAV_HEADER_BYTES};

/// The view a run's sounds play in: the runner has one.
const VIEW: u32 = 1;
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
            peak: 0,
            operations: 0,
        }
    }

    /// A voice operation the decoder's engine gave: from the page's own
    /// records, or from a script's sound operation as the decoder
    /// carried it out (`Engine::script_sound`).
    pub fn voice(&mut self, audio: Audio) {
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

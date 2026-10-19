//! Sound on the pipes (`architecture.md` section 6.4, the engine's area AUD):
//! the voice operations a decoder's audio receiver produces, carried to the
//! trusted core and, if its channel is the one heard, on to the interface,
//! which mixes and plays them. One payload form serves both pipes; the
//! first byte names the operation, and each side accepts only the
//! operations meant for it.

use neoteletext_audio::{CHANNELS, DUCK_PERCENT, MAX_FADE_MS, MAX_GAIN, MIN_FADE_MS, SAMPLE_RATES};

use super::{Fields, ProtocolError};

/// The longest sound the browser plays: two minutes at the highest rate.
pub const MAX_SOUND_SAMPLES: u32 = 48_000 * 120;
/// The most samples in one message: 64 KB of 16-bit samples.
pub const MAX_MESSAGE_SAMPLES: usize = 32 * 1024;

const OP_DEFINE: u8 = 1;
const OP_SAMPLES: u8 = 2;
const OP_FORGET: u8 = 3;
const OP_START: u8 = 4;
const OP_STOP: u8 = 5;
const OP_GAIN: u8 = 6;
/// The trusted core's alone: the interface forgets every sound.
const OP_RESET: u8 = 7;

const FLAG_LOOP: u8 = 1;
const FLAG_AFTER_CURRENT: u8 = 2;
const ALL_FLAGS: u8 = FLAG_LOOP | FLAG_AFTER_CURRENT;

/// A sound: the audio receiver's handle and generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Sound {
    /// The OBJECT record's handle.
    pub handle: u8,
    /// Its generation.
    pub generation: u16,
}

/// One voice operation (the engine's REQ-AUD-018).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Audio {
    /// A sound exists: make room for it, silent until its samples arrive.
    Define {
        /// The sound.
        sound: Sound,
        /// One of the profile's sample rates.
        rate_hz: u32,
        /// Its length, at most [`MAX_SOUND_SAMPLES`].
        samples: u32,
    },
    /// Samples of a sound from sample `first`.
    Samples {
        /// The sound.
        sound: Sound,
        /// Where they start.
        first: u32,
        /// At most [`MAX_MESSAGE_SAMPLES`], ending within the sound's bound.
        pcm: Vec<i16>,
    },
    /// The sound is gone; a voice already playing it plays to its end.
    Forget(Sound),
    /// Start a sound on a channel.
    Start {
        /// 0 to 3.
        channel: u8,
        /// The sound.
        sound: Sound,
        /// 0 to 100 per cent.
        gain: u8,
        /// 5 to 10 000 milliseconds.
        fade_ms: u16,
        /// Repeat until stopped.
        looped: bool,
        /// The other channels' level while this plays: a Table 19 step.
        duck_percent: u8,
        /// Wait for the channel's current sound to end.
        after_current: bool,
    },
    /// Ramp a channel to silence and stop it.
    Stop {
        /// 0 to 3.
        channel: u8,
        /// 5 to 10 000 milliseconds.
        fade_ms: u16,
    },
    /// Ramp a channel to a new gain.
    Gain {
        /// 0 to 3.
        channel: u8,
        /// 0 to 100 per cent.
        gain: u8,
        /// 5 to 10 000 milliseconds.
        fade_ms: u16,
    },
    /// Forget every sound and stop every channel at once: another channel
    /// is now the one heard. Only the trusted core sends it.
    Reset,
}

/// Who sends a payload, which decides what it may carry.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sender {
    Decoder,
    Core,
}

fn push_sound(payload: &mut Vec<u8>, sound: Sound) {
    payload.push(sound.handle);
    payload.extend_from_slice(&sound.generation.to_be_bytes());
}

/// The payload of `audio`, checked against the same bounds the reader
/// applies, so a faulty sender is caught at once.
pub(crate) fn push(payload: &mut Vec<u8>, audio: &Audio, sender: Sender) -> Result<(), ProtocolError> {
    match audio {
        Audio::Define { sound, rate_hz, samples } => {
            payload.push(OP_DEFINE);
            push_sound(payload, *sound);
            payload.extend_from_slice(&rate_hz.to_be_bytes());
            payload.extend_from_slice(&samples.to_be_bytes());
        }
        Audio::Samples { sound, first, pcm } => {
            payload.push(OP_SAMPLES);
            push_sound(payload, *sound);
            payload.extend_from_slice(&first.to_be_bytes());
            for sample in pcm {
                payload.extend_from_slice(&sample.to_le_bytes());
            }
        }
        Audio::Forget(sound) => {
            payload.push(OP_FORGET);
            push_sound(payload, *sound);
        }
        Audio::Start { channel, sound, gain, fade_ms, looped, duck_percent, after_current } => {
            payload.extend_from_slice(&[OP_START, *channel]);
            push_sound(payload, *sound);
            payload.push(*gain);
            payload.extend_from_slice(&fade_ms.to_be_bytes());
            payload.push(*duck_percent);
            payload.push(if *looped { FLAG_LOOP } else { 0 } | if *after_current { FLAG_AFTER_CURRENT } else { 0 });
        }
        Audio::Stop { channel, fade_ms } => {
            payload.extend_from_slice(&[OP_STOP, *channel]);
            payload.extend_from_slice(&fade_ms.to_be_bytes());
        }
        Audio::Gain { channel, gain, fade_ms } => {
            payload.extend_from_slice(&[OP_GAIN, *channel, *gain]);
            payload.extend_from_slice(&fade_ms.to_be_bytes());
        }
        Audio::Reset => payload.push(OP_RESET),
    }
    // What would be refused on arrival is refused on sending.
    read(&mut Fields { bytes: payload }, sender).map(|_| ())
}

/// Reads an audio payload from `sender`, checking every bound.
pub(crate) fn read(fields: &mut Fields<'_>, sender: Sender) -> Result<Audio, ProtocolError> {
    let audio = match fields.u8()? {
        OP_DEFINE => {
            let sound = sound(fields)?;
            let rate_hz = fields.u32()?;
            if !SAMPLE_RATES.contains(&rate_hz) {
                return Err(ProtocolError::BadValue("sample rate"));
            }
            let samples = fields.u32()?;
            if samples == 0 || samples > MAX_SOUND_SAMPLES {
                return Err(ProtocolError::BadValue("sound length"));
            }
            Audio::Define { sound, rate_hz, samples }
        }
        OP_SAMPLES => {
            let sound = sound(fields)?;
            let first = fields.u32()?;
            let bytes = fields.take(fields.bytes.len())?;
            let (pairs, odd) = bytes.as_chunks::<2>();
            if !odd.is_empty() || pairs.is_empty() || pairs.len() > MAX_MESSAGE_SAMPLES {
                return Err(ProtocolError::BadValue("samples"));
            }
            let end = u32::try_from(pairs.len()).ok().and_then(|count| first.checked_add(count));
            if end.is_none_or(|end| end > MAX_SOUND_SAMPLES) {
                return Err(ProtocolError::BadValue("samples beyond the sound"));
            }
            Audio::Samples { sound, first, pcm: pairs.iter().map(|pair| i16::from_le_bytes(*pair)).collect() }
        }
        OP_FORGET => Audio::Forget(sound(fields)?),
        OP_START => {
            let channel = channel(fields)?;
            let sound = sound(fields)?;
            let gain = gain(fields)?;
            let fade_ms = fade(fields)?;
            let duck_percent = fields.u8()?;
            if !DUCK_PERCENT.contains(&duck_percent) {
                return Err(ProtocolError::BadValue("duck"));
            }
            let flags = fields.u8()?;
            if flags & !ALL_FLAGS != 0 {
                return Err(ProtocolError::BadValue("audio flags"));
            }
            Audio::Start { channel, sound, gain, fade_ms, looped: flags & FLAG_LOOP != 0, duck_percent, after_current: flags & FLAG_AFTER_CURRENT != 0 }
        }
        OP_STOP => Audio::Stop { channel: channel(fields)?, fade_ms: fade(fields)? },
        OP_GAIN => Audio::Gain { channel: channel(fields)?, gain: gain(fields)?, fade_ms: fade(fields)? },
        OP_RESET if sender == Sender::Core => Audio::Reset,
        _ => return Err(ProtocolError::BadValue("audio operation")),
    };
    fields.end()?;
    Ok(audio)
}

fn sound(fields: &mut Fields<'_>) -> Result<Sound, ProtocolError> {
    Ok(Sound { handle: fields.u8()?, generation: fields.u16()? })
}

fn channel(fields: &mut Fields<'_>) -> Result<u8, ProtocolError> {
    let channel = fields.u8()?;
    if channel >= CHANNELS {
        return Err(ProtocolError::BadValue("audio channel"));
    }
    Ok(channel)
}

fn gain(fields: &mut Fields<'_>) -> Result<u8, ProtocolError> {
    let gain = fields.u8()?;
    if gain > MAX_GAIN {
        return Err(ProtocolError::BadValue("gain"));
    }
    Ok(gain)
}

fn fade(fields: &mut Fields<'_>) -> Result<u16, ProtocolError> {
    let fade = fields.u16()?;
    if !(MIN_FADE_MS..=MAX_FADE_MS).contains(&fade) {
        return Err(ProtocolError::BadValue("fade"));
    }
    Ok(fade)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{FromDecoder, ToDecoder, read_from_core as decoder_reads, read_from_decoder, write_to_core as decoder_writes, write_to_decoder};
    use crate::{ToCore, ToInterface, read_from_core, read_from_interface, write_to_core, write_to_interface};
    use interface_core::canvas::PageKey;

    const SOUND: Sound = Sound { handle: 3, generation: 9 };

    fn every_operation() -> Vec<Audio> {
        vec![
            Audio::Define { sound: SOUND, rate_hz: 22_050, samples: 44_100 },
            Audio::Samples { sound: SOUND, first: 1000, pcm: vec![-32_768, 0, 32_767] },
            Audio::Forget(SOUND),
            Audio::Start { channel: 3, sound: SOUND, gain: 80, fade_ms: 5, looped: true, duck_percent: 25, after_current: true },
            Audio::Stop { channel: 0, fade_ms: 10_000 },
            Audio::Gain { channel: 1, gain: 0, fade_ms: 50 },
        ]
    }

    #[test]
    fn every_operation_round_trips_on_both_pipes() {
        for audio in every_operation() {
            let mut bytes = Vec::new();
            decoder_writes(&mut bytes, &FromDecoder::Audio(audio.clone())).unwrap();
            assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap(), Some(FromDecoder::Audio(audio.clone())));
            let mut bytes = Vec::new();
            write_to_interface(&mut bytes, &ToInterface::Audio(audio.clone())).unwrap();
            assert_eq!(read_from_core(&mut bytes.as_slice()).unwrap(), Some(ToInterface::Audio(audio)));
        }
        let mut bytes = Vec::new();
        write_to_interface(&mut bytes, &ToInterface::Audio(Audio::Reset)).unwrap();
        assert_eq!(read_from_core(&mut bytes.as_slice()).unwrap(), Some(ToInterface::Audio(Audio::Reset)));
        for message in [ToDecoder::Opened(Some(0x123)), ToDecoder::Opened(None), ToDecoder::Audible(true), ToDecoder::Audible(false)] {
            let mut bytes = Vec::new();
            write_to_decoder(&mut bytes, &message).unwrap();
            assert_eq!(decoder_reads(&mut bytes.as_slice()).unwrap(), Some(message));
        }
        for message in [ToCore::Opened(Some(PageKey { channel: 4, page: 0x100 })), ToCore::Opened(None)] {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, &message).unwrap();
            assert_eq!(read_from_interface(&mut bytes.as_slice()).unwrap(), Some(message));
        }
    }

    #[test]
    fn a_decoder_cannot_reset_the_interface() {
        assert!(matches!(decoder_writes(&mut Vec::new(), &FromDecoder::Audio(Audio::Reset)), Err(ProtocolError::BadValue("audio operation"))));
        // Written by hand, as a hostile decoder would, it is refused too.
        let bytes = [67, 0, 0, 0, 1, OP_RESET];
        assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::BadValue("audio operation"))));
    }

    #[test]
    fn every_bound_is_kept() {
        let refused = |audio: Audio, why: &str| {
            let result = decoder_writes(&mut Vec::new(), &FromDecoder::Audio(audio.clone()));
            assert!(matches!(result, Err(ProtocolError::BadValue(reason)) if reason == why), "{audio:?}: {result:?}");
        };
        refused(Audio::Define { sound: SOUND, rate_hz: 12_345, samples: 10 }, "sample rate");
        refused(Audio::Define { sound: SOUND, rate_hz: 8_000, samples: 0 }, "sound length");
        refused(Audio::Define { sound: SOUND, rate_hz: 8_000, samples: MAX_SOUND_SAMPLES + 1 }, "sound length");
        refused(Audio::Samples { sound: SOUND, first: 0, pcm: vec![] }, "samples");
        refused(Audio::Samples { sound: SOUND, first: 0, pcm: vec![0; MAX_MESSAGE_SAMPLES + 1] }, "samples");
        refused(Audio::Samples { sound: SOUND, first: MAX_SOUND_SAMPLES, pcm: vec![0] }, "samples beyond the sound");
        refused(Audio::Samples { sound: SOUND, first: u32::MAX, pcm: vec![0] }, "samples beyond the sound");
        let start =
            |channel, gain, fade_ms, duck_percent| Audio::Start { channel, sound: SOUND, gain, fade_ms, looped: false, duck_percent, after_current: false };
        refused(start(4, 100, 5, 100), "audio channel");
        refused(start(0, 101, 5, 100), "gain");
        refused(start(0, 100, 4, 100), "fade");
        refused(start(0, 100, 10_001, 100), "fade");
        refused(start(0, 100, 5, 50), "duck");
        // An odd byte of samples, unknown flags and an unknown operation.
        for (bytes, why) in [
            (vec![67, 0, 0, 0, 9, OP_SAMPLES, 0, 0, 0, 0, 0, 0, 0, 1], "samples"),
            (vec![67, 0, 0, 0, 10, OP_START, 0, 0, 0, 0, 100, 0, 5, 100, 4], "audio flags"),
            (vec![67, 0, 0, 0, 1, 99], "audio operation"),
            (vec![54, 0, 0, 0, 1, 2], "audible"),
        ] {
            let result = if bytes[0] == 54 { decoder_reads(&mut bytes.as_slice()).map(|_| ()) } else { read_from_decoder(&mut bytes.as_slice()).map(|_| ()) };
            assert!(matches!(result, Err(ProtocolError::BadValue(reason)) if reason == why), "{bytes:?}: {result:?}");
        }
    }
}

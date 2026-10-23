//! Sound on the pipes (`architecture.md` section 6.4, the engine's area AUD):
//! the voice operations a decoder's audio receiver produces, carried to the
//! trusted core and on to the interface, tagged with their channel, which
//! mixes and plays them. One payload form serves both pipes; the
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
/// The trusted core's alone: the interface forgets every sound of a channel.
const OP_RESET: u8 = 7;
const OP_STREAM: u8 = 8;
const OP_STREAM_SAMPLES: u8 = 9;
/// The trusted core's alone: the user muted or unmuted a channel.
const OP_MUTE: u8 = 10;

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
    /// A live sound exists (the engine's REQ-AUD-014): its samples follow
    /// without end, in order.
    Stream {
        /// The sound.
        sound: Sound,
        /// One of the profile's sample rates.
        rate_hz: u32,
    },
    /// The next samples of a live sound.
    StreamSamples {
        /// The sound.
        sound: Sound,
        /// At most [`MAX_MESSAGE_SAMPLES`].
        pcm: Vec<i16>,
    },
    /// Forget every sound of the channel and stop its voices at once: its
    /// decoder ended. Only the trusted core sends it.
    Reset,
    /// The user muted the channel's tab, or unmuted it. Only the trusted
    /// core sends it.
    Mute(bool),
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
        Audio::Stream { sound, rate_hz } => {
            payload.push(OP_STREAM);
            push_sound(payload, *sound);
            payload.extend_from_slice(&rate_hz.to_be_bytes());
        }
        Audio::StreamSamples { sound, pcm } => {
            payload.push(OP_STREAM_SAMPLES);
            push_sound(payload, *sound);
            for sample in pcm {
                payload.extend_from_slice(&sample.to_le_bytes());
            }
        }
        Audio::Reset => payload.push(OP_RESET),
        Audio::Mute(muted) => payload.extend_from_slice(&[OP_MUTE, u8::from(*muted)]),
    }
    // What would be refused on arrival is refused on sending.
    read(&mut Fields::new(payload), sender).map(|_| ())
}

/// Reads an audio payload from `sender`, checking every bound.
pub(crate) fn read(fields: &mut Fields<'_>, sender: Sender) -> Result<Audio, ProtocolError> {
    let audio = match fields.u8()? {
        OP_DEFINE => {
            let sound = sound(fields)?;
            let rate_hz = rate(fields)?;
            let samples = fields.u32()?;
            if samples == 0 || samples > MAX_SOUND_SAMPLES {
                return Err(ProtocolError::BadValue("sound length"));
            }
            Audio::Define { sound, rate_hz, samples }
        }
        OP_SAMPLES => {
            let sound = sound(fields)?;
            let first = fields.u32()?;
            let bytes = fields.take(fields.remaining())?;
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
        OP_STREAM => Audio::Stream { sound: sound(fields)?, rate_hz: rate(fields)? },
        OP_STREAM_SAMPLES => {
            let sound = sound(fields)?;
            let bytes = fields.take(fields.remaining())?;
            let (pairs, odd) = bytes.as_chunks::<2>();
            if !odd.is_empty() || pairs.is_empty() || pairs.len() > MAX_MESSAGE_SAMPLES {
                return Err(ProtocolError::BadValue("samples"));
            }
            Audio::StreamSamples { sound, pcm: pairs.iter().map(|pair| i16::from_le_bytes(*pair)).collect() }
        }
        OP_RESET if sender == Sender::Core => Audio::Reset,
        OP_MUTE if sender == Sender::Core => Audio::Mute(fields.flag("mute")?),
        _ => return Err(ProtocolError::BadValue("audio operation")),
    };
    fields.end()?;
    Ok(audio)
}

fn rate(fields: &mut Fields<'_>) -> Result<u32, ProtocolError> {
    let rate_hz = fields.u32()?;
    if !SAMPLE_RATES.contains(&rate_hz) {
        return Err(ProtocolError::BadValue("sample rate"));
    }
    Ok(rate_hz)
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
    use crate::sound::{ToSound, read_to_sound, write_to_sound};
    use crate::{ToCore, read_from_interface, write_to_core};
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
            Audio::Stream { sound: SOUND, rate_hz: 8_000 },
            Audio::StreamSamples { sound: SOUND, pcm: vec![1, -1] },
        ]
    }

    #[test]
    fn every_operation_round_trips_on_both_pipes() {
        for audio in every_operation() {
            let mut bytes = Vec::new();
            decoder_writes(&mut bytes, &FromDecoder::Audio(audio.clone())).unwrap();
            assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap(), Some(FromDecoder::Audio(audio.clone())));
            let mut bytes = Vec::new();
            write_to_sound(&mut bytes, &ToSound::Audio { channel: 7, audio: audio.clone() }).unwrap();
            assert_eq!(read_to_sound(&mut bytes.as_slice()).unwrap(), Some(ToSound::Audio { channel: 7, audio }));
        }
        for audio in [Audio::Reset, Audio::Mute(true), Audio::Mute(false)] {
            let mut bytes = Vec::new();
            write_to_sound(&mut bytes, &ToSound::Audio { channel: 1, audio: audio.clone() }).unwrap();
            assert_eq!(read_to_sound(&mut bytes.as_slice()).unwrap(), Some(ToSound::Audio { channel: 1, audio }));
        }
        for message in [ToDecoder::Opened(Some(0x123)), ToDecoder::Opened(None)] {
            let mut bytes = Vec::new();
            write_to_decoder(&mut bytes, &message).unwrap();
            assert_eq!(decoder_reads(&mut bytes.as_slice()).unwrap(), Some(message));
        }
        for message in
            [ToCore::Opened(Some(PageKey { channel: 4, page: 0x100 })), ToCore::Opened(None), ToCore::MuteTab(2), ToCore::MuteAll(true), ToCore::MuteAll(false)]
        {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, &message).unwrap();
            assert_eq!(read_from_interface(&mut bytes.as_slice()).unwrap(), Some(message));
        }
    }

    #[test]
    fn a_decoder_cannot_reset_or_mute_the_interface() {
        assert!(matches!(decoder_writes(&mut Vec::new(), &FromDecoder::Audio(Audio::Reset)), Err(ProtocolError::BadValue("audio operation"))));
        assert!(matches!(decoder_writes(&mut Vec::new(), &FromDecoder::Audio(Audio::Mute(false))), Err(ProtocolError::BadValue("audio operation"))));
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
        refused(Audio::Stream { sound: SOUND, rate_hz: 12_345 }, "sample rate");
        refused(Audio::StreamSamples { sound: SOUND, pcm: vec![] }, "samples");
        refused(Audio::StreamSamples { sound: SOUND, pcm: vec![0; MAX_MESSAGE_SAMPLES + 1] }, "samples");
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
        ] {
            let result = read_from_decoder(&mut bytes.as_slice()).map(|_| ());
            assert!(matches!(result, Err(ProtocolError::BadValue(reason)) if reason == why), "{bytes:?}: {result:?}");
        }
    }
}

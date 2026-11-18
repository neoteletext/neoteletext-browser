//! Sound on the pipes (`architecture.md` section 6.4, the engine's area AUD):
//! the voice operations a decoder's audio receiver produces, carried to the
//! trusted core and on to the interface, tagged with their channel, which
//! mixes and plays them. One payload form serves both pipes; the
//! first byte names the operation, and each side accepts only the
//! operations meant for it.

use neoteletext_audio::record::{SYNTH8_KEYS_DUAL, SYNTH8_KEYS_SPLIT, SYNTH8_KEYS_WHOLE};
use neoteletext_audio::{CHANNELS, Codec, DUCK_PERCENT, MAX_FADE_MS, MAX_GAIN, MIN_FADE_MS, MOST_BANK_BYTES, MOST_BANKS, MOST_EVENT_BLOCK_BYTES, SAMPLE_RATES};
use neoteletext_rules::limits::{MAX_MIDI_DATA, MAX_PART, MAX_VARIANT, MAX_WATCHED};
use neoteletext_rules::table::{WATCH_ENVELOPE_LEVEL, WATCH_ENVELOPE_STAGE, WATCH_LEVEL, WATCH_VOICES};

use super::{Fields, ProtocolError};

#[cfg(test)]
mod tests;

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
/// A stream ends (the engine's INT-36): a dedicated operation rather than
/// a channel on [`Audio::Forget`], which ends every stream of a sound.
const OP_END_STREAM: u8 = 11;

/// One bank of the local sound about to start (page-rules.md RULE-63).
const OP_LOCAL_BANK: u8 = 12;
/// A local sound starts: its instrument is made in the sound helper.
const OP_LOCAL: u8 = 13;
/// One MIDI message of a rule's `midi` action, for a local sound.
const OP_LOCAL_MIDI: u8 = 14;
/// The trusted core's alone: the watch list of a view (RULE-33 item 2).
const OP_WATCH: u8 = 15;
// A cue mark of a sound playing (page-rules.md RULE-63, RULE-109), from a
// decoder, and the pause of a view's local sounds (RULE-111 case 4), from
// the trusted core alone: the next operation numbers free.
const OP_CUE: u8 = 16;
const OP_PAUSE: u8 = 17;

/// The most bytes of a bank image a local sound is given: the audio
/// profile's (`neoteletext_audio::MOST_BANK_BYTES`, MIDI-55).
pub const MAX_BANK_IMAGE: usize = MOST_BANK_BYTES as usize;
/// The most bytes of a local sound's setup block: one event block of the
/// audio profile (`neoteletext_audio::MOST_EVENT_BLOCK_BYTES`, MIDI-30).
pub const MAX_LOCAL_SETUP: usize = MOST_EVENT_BLOCK_BYTES;
/// The most slots of a watch list (RULE-33, the rule table's
/// `MAX_WATCHED`), and so the most bytes of a metered report.
pub const MAX_WATCH_SLOTS: usize = MAX_WATCHED;
/// The MIDI status bytes a rule's `midi` action carries (RULE-21): note
/// off, note on, control change, program change and pitch bend, by their
/// high four bits (MIDI 1.0 Detailed Specification, "Channel Voice
/// Messages").
const MIDI_STATUS_KINDS: [u8; 5] = [0x80, 0x90, 0xB0, 0xC0, 0xE0];
/// The bits of a status byte that name its kind; the rest are its MIDI
/// channel.
const MIDI_KIND_MASK: u8 = 0xF0;

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

/// One metered value a view's table reads (RULE-33 item 1): a slot of the
/// watch list, and one byte of each metered report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchSlot {
    /// The audio channel, 0 to 3.
    pub channel: u8,
    /// 1 `level`, 2 `envelope_stage`, 3 `envelope_level`, 4 `voices`.
    pub kind: u8,
    /// The part, 0 to 15; 0 for a level.
    pub part: u8,
}

/// A local sound's start (RULE-60, RULE-63; the engine's
/// `neoteletext_audio::port::LocalStart`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalStart {
    /// The sound.
    pub sound: Sound,
    /// The family: its codec's code, 13 to 18.
    pub family: u8,
    /// The variant of the family, 0 to 2.
    pub variant: u8,
    /// The family's option byte: synth8's key mode, else 0.
    pub option: u8,
    /// synth8's split point, a MIDI key; 0 for every other family.
    pub split: u8,
    /// 0 to 100 per cent.
    pub gain: u8,
    /// 5 to 10 000 milliseconds.
    pub fade_ms: u16,
    /// The other channels' level while this plays: a Table 19 step.
    pub duck_percent: u8,
    /// The setup block, at most [`MAX_LOCAL_SETUP`] bytes: an event list
    /// the sound helper applies to the new instrument. Passed on unread by
    /// the trusted core.
    pub setup: Vec<u8>,
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
    /// The sound is gone; a voice already playing it plays to its end, and
    /// every stream of it, on any audio channel, ends as
    /// [`Audio::EndStream`] ends one.
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
    /// A stream begins on an audio channel (the engine's REQ-AUD-014,
    /// INT-33 and INT-37): samples of the sound follow in order, with no
    /// length known ahead, until [`Audio::EndStream`] or [`Audio::Forget`].
    /// A stream is named by its audio channel and its sound together, so
    /// one sound may stream on two audio channels at once, each its own
    /// playing; a stream begun under a name still held replaces it.
    Stream {
        /// 0 to 3.
        channel: u8,
        /// The sound.
        sound: Sound,
        /// One of the profile's sample rates.
        rate_hz: u32,
    },
    /// The next samples of the stream of a sound on an audio channel.
    StreamSamples {
        /// 0 to 3.
        channel: u8,
        /// The sound.
        sound: Sound,
        /// At most [`MAX_MESSAGE_SAMPLES`].
        pcm: Vec<i16>,
    },
    /// The stream of a sound on an audio channel ends (the engine's
    /// INT-36): no more samples come. Its voice plays what it holds, then
    /// ends, and a sound queued after it on that channel starts.
    EndStream {
        /// 0 to 3.
        channel: u8,
        /// The sound.
        sound: Sound,
    },
    /// One bank of the local sound about to start on an audio channel
    /// (RULE-63 operation 12): the banks of a start come in index order,
    /// each before its [`Audio::Local`]. The image is passed on unread by
    /// the trusted core.
    LocalBank {
        /// 0 to 3.
        channel: u8,
        /// The sound.
        sound: Sound,
        /// The bank's index, 0 to 9.
        index: u8,
        /// The bank image, at most [`MAX_BANK_IMAGE`] bytes.
        image: Vec<u8>,
    },
    /// A local sound starts on an audio channel (RULE-63 operation 13):
    /// the sound helper makes the family's instrument with the banks given
    /// for that channel and sound since the last start, and applies the
    /// setup.
    Local {
        /// 0 to 3.
        channel: u8,
        /// The start.
        start: LocalStart,
    },
    /// One MIDI channel voice message of a rule's `midi` action, for the
    /// local sound on an audio channel (RULE-21, RULE-63 operation 14).
    LocalMidi {
        /// 0 to 3.
        channel: u8,
        /// Note off, note on, control change, program change or pitch
        /// bend, with the part's MIDI channel in its low four bits.
        status: u8,
        /// 0 to 127.
        data1: u8,
        /// 0 to 127.
        data2: u8,
    },
    /// The watch list of the channel's view (RULE-33 item 2, RULE-63
    /// operation 15), replacing the one before: at most
    /// [`MAX_WATCH_SLOTS`] slots; empty for none. Only the trusted core
    /// sends it.
    Watch(Vec<WatchSlot>),
    /// A cue point of the sound streaming on an audio channel (RULE-109,
    /// RULE-63): the sound helper reports it heard when the device buffer
    /// holding sample `position` of the stream has been rendered.
    Cue {
        /// 0 to 3.
        channel: u8,
        /// The sound.
        sound: Sound,
        /// Samples from the stream's first sample.
        position: u64,
        /// The cue's number, 0 to `MAX_CUE`.
        number: u8,
    },
    /// The view's local sounds are silenced with every note released and
    /// their instruments kept (true), or rendered again (false): the page
    /// they belong to is paused (RULE-111 case 4). Only the trusted core
    /// sends it.
    Pause(bool),
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
        Audio::Stream { channel, sound, rate_hz } => {
            payload.extend_from_slice(&[OP_STREAM, *channel]);
            push_sound(payload, *sound);
            payload.extend_from_slice(&rate_hz.to_be_bytes());
        }
        Audio::StreamSamples { channel, sound, pcm } => {
            payload.extend_from_slice(&[OP_STREAM_SAMPLES, *channel]);
            push_sound(payload, *sound);
            for sample in pcm {
                payload.extend_from_slice(&sample.to_le_bytes());
            }
        }
        Audio::EndStream { channel, sound } => {
            payload.extend_from_slice(&[OP_END_STREAM, *channel]);
            push_sound(payload, *sound);
        }
        Audio::LocalBank { channel, sound, index, image } => {
            payload.extend_from_slice(&[OP_LOCAL_BANK, *channel]);
            push_sound(payload, *sound);
            payload.push(*index);
            payload.extend_from_slice(image);
        }
        Audio::Local { channel, start } => {
            payload.extend_from_slice(&[OP_LOCAL, *channel]);
            push_sound(payload, start.sound);
            payload.extend_from_slice(&[start.family, start.variant, start.option, start.split, start.gain]);
            payload.extend_from_slice(&start.fade_ms.to_be_bytes());
            payload.push(start.duck_percent);
            payload.extend_from_slice(&start.setup);
        }
        Audio::LocalMidi { channel, status, data1, data2 } => payload.extend_from_slice(&[OP_LOCAL_MIDI, *channel, *status, *data1, *data2]),
        Audio::Watch(slots) => {
            payload.push(OP_WATCH);
            crate::push_count_u8(payload, slots.len(), MAX_WATCH_SLOTS, "watch list")?;
            for slot in slots {
                payload.extend_from_slice(&[slot.channel, slot.kind, slot.part]);
            }
        }
        Audio::Cue { channel, sound, position, number } => {
            payload.extend_from_slice(&[OP_CUE, *channel]);
            push_sound(payload, *sound);
            payload.extend_from_slice(&position.to_be_bytes());
            payload.push(*number);
        }
        Audio::Pause(paused) => payload.extend_from_slice(&[OP_PAUSE, u8::from(*paused)]),
        Audio::Reset => payload.push(OP_RESET),
        Audio::Mute(muted) => payload.extend_from_slice(&[OP_MUTE, u8::from(*muted)]),
    }
    // What would be refused on arrival is refused on sending.
    read(&mut Fields::new(payload), sender).map(|_| ())
}

/// Reads an audio payload from `sender`, checking every bound.
pub(crate) fn read(fields: &mut Fields<'_>, sender: Sender) -> Result<Audio, ProtocolError> {
    let operation = fields.u8()?;
    let audio = match operation {
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
        OP_STREAM => Audio::Stream { channel: channel(fields)?, sound: sound(fields)?, rate_hz: rate(fields)? },
        OP_STREAM_SAMPLES => {
            let channel = channel(fields)?;
            let sound = sound(fields)?;
            let bytes = fields.take(fields.remaining())?;
            let (pairs, odd) = bytes.as_chunks::<2>();
            if !odd.is_empty() || pairs.is_empty() || pairs.len() > MAX_MESSAGE_SAMPLES {
                return Err(ProtocolError::BadValue("samples"));
            }
            Audio::StreamSamples { channel, sound, pcm: pairs.iter().map(|pair| i16::from_le_bytes(*pair)).collect() }
        }
        OP_END_STREAM => Audio::EndStream { channel: channel(fields)?, sound: sound(fields)? },
        OP_LOCAL_BANK | OP_LOCAL | OP_LOCAL_MIDI => read_local(operation, fields)?,
        OP_WATCH if sender == Sender::Core => {
            let count = fields.count_u8(MAX_WATCH_SLOTS, "watch list")?;
            let mut slots = Vec::with_capacity(count);
            for _ in 0..count {
                slots.push(watch_slot(fields)?);
            }
            Audio::Watch(slots)
        }
        OP_CUE => {
            let channel = channel(fields)?;
            let sound = sound(fields)?;
            let position = u64::from_be_bytes(fields.array()?);
            let number = fields.u8()?;
            if number > neoteletext_rules::limits::MAX_CUE {
                return Err(ProtocolError::BadValue("cue number"));
            }
            Audio::Cue { channel, sound, position, number }
        }
        OP_PAUSE if sender == Sender::Core => Audio::Pause(fields.flag("pause")?),
        OP_RESET if sender == Sender::Core => Audio::Reset,
        OP_MUTE if sender == Sender::Core => Audio::Mute(fields.flag("mute")?),
        _ => return Err(ProtocolError::BadValue("audio operation")),
    };
    fields.end()?;
    Ok(audio)
}

/// Reads a local sound's operation (page-rules.md RULE-63), each field
/// within its bound: the bank image and the setup block are bounded in
/// length and otherwise unread.
fn read_local(operation: u8, fields: &mut Fields<'_>) -> Result<Audio, ProtocolError> {
    Ok(match operation {
        OP_LOCAL_BANK => {
            let channel = channel(fields)?;
            let sound = sound(fields)?;
            let index = fields.u8()?;
            if usize::from(index) >= MOST_BANKS {
                return Err(ProtocolError::BadValue("bank index"));
            }
            let length = fields.remaining();
            if length == 0 || length > MAX_BANK_IMAGE {
                return Err(ProtocolError::BadValue("bank image"));
            }
            Audio::LocalBank { channel, sound, index, image: fields.take(length)?.to_vec() }
        }
        OP_LOCAL => {
            let channel = channel(fields)?;
            let sound = sound(fields)?;
            let (family, variant, option, split) = (fields.u8()?, fields.u8()?, fields.u8()?, fields.u8()?);
            // RULE-60: a family that plays notes, never the synth6581
            // family or a bank.
            if !Codec::is_family_code(family) {
                return Err(ProtocolError::BadValue("local family"));
            }
            if variant > MAX_VARIANT {
                return Err(ProtocolError::BadValue("local variant"));
            }
            if ![SYNTH8_KEYS_WHOLE, SYNTH8_KEYS_DUAL, SYNTH8_KEYS_SPLIT].contains(&option) {
                return Err(ProtocolError::BadValue("local option"));
            }
            if split > MAX_MIDI_DATA {
                return Err(ProtocolError::BadValue("local split"));
            }
            let gain = gain(fields)?;
            let fade_ms = fade(fields)?;
            let duck_percent = fields.u8()?;
            if !DUCK_PERCENT.contains(&duck_percent) {
                return Err(ProtocolError::BadValue("duck"));
            }
            let length = fields.remaining();
            if length > MAX_LOCAL_SETUP {
                return Err(ProtocolError::BadValue("local setup"));
            }
            Audio::Local {
                channel,
                start: LocalStart { sound, family, variant, option, split, gain, fade_ms, duck_percent, setup: fields.take(length)?.to_vec() },
            }
        }
        _ => {
            let channel = channel(fields)?;
            let (status, data1, data2) = (fields.u8()?, fields.u8()?, fields.u8()?);
            if !MIDI_STATUS_KINDS.contains(&(status & MIDI_KIND_MASK)) || data1 > MAX_MIDI_DATA || data2 > MAX_MIDI_DATA {
                return Err(ProtocolError::BadValue("midi message"));
            }
            Audio::LocalMidi { channel, status, data1, data2 }
        }
    })
}

/// One slot of a watch list, each field within its range (RULE-33 item
/// 1): a level has part 0.
pub(crate) fn watch_slot(fields: &mut Fields<'_>) -> Result<WatchSlot, ProtocolError> {
    let slot = WatchSlot { channel: channel(fields)?, kind: fields.u8()?, part: fields.u8()? };
    let kinds = [WATCH_LEVEL, WATCH_ENVELOPE_STAGE, WATCH_ENVELOPE_LEVEL, WATCH_VOICES];
    if !kinds.contains(&slot.kind) || slot.part > MAX_PART || (slot.kind == WATCH_LEVEL && slot.part != 0) {
        return Err(ProtocolError::BadValue("watch slot"));
    }
    Ok(slot)
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

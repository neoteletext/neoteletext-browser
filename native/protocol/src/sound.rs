//! Messages between the trusted core and the sound helper (decision 55).
//! The core passes on the voice operations it checked (requirement 16.4)
//! and the settings' sound and volume (requirement 19.4); the sound helper
//! says which channels sound, for the tabs' speakers and so a sounding
//! tab stays awake (decision 47).

use std::io::{Read, Write};

use super::audio::{self, Audio, Sender};
use super::interface::{push_channels, read_channels};
use super::settings::MAX_VOLUME;
use super::{Fields, ProtocolError, read_frame, write_frame};

const KIND_AUDIO: u8 = 1;
const KIND_SETTINGS: u8 = 2;
const KIND_CLOSE: u8 = 3;
const KIND_SOUNDING: u8 = 4;
/// The metered values of a view (page-rules.md RULE-33 item 3, RULE-63).
const KIND_METERED: u8 = 5;
/// A view's local sounds were stopped for overrunning their time (RULE-62).
const KIND_LOCAL_OVERRUN: u8 = 6;
/// A cue of a view's sound was heard (page-rules.md RULE-109, RULE-63).
const KIND_CUE_HEARD: u8 = 7;
/// The processor time spent rendering one view's sounds (REQ-6626).
const KIND_SPENT: u8 = 8;

/// From the trusted core to the sound helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToSound {
    /// A voice operation of a channel, checked by the trusted core.
    Audio {
        /// The channel whose sound it is.
        channel: u32,
        /// The operation.
        audio: Audio,
    },
    /// The settings: page sounds play, and their volume, 0 to 100.
    Settings {
        /// Page sounds play.
        sound: bool,
        /// Their volume.
        volume: u8,
    },
    /// Stop.
    Close,
}

/// From the sound helper to the trusted core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromSound {
    /// The channels sounding now, muted or not.
    Sounding(Vec<u32>),
    /// The metered values of one view's watch list (page-rules.md RULE-33
    /// item 3): one byte for each slot of the list the trusted core last
    /// passed for that channel, in its order. The trusted core checks the
    /// count against that list.
    Metered {
        /// The view's channel.
        channel: u32,
        /// The values, at most [`audio::MAX_WATCH_SLOTS`].
        values: Vec<u8>,
    },
    /// Rendering one view's local sounds took longer than the buffers it
    /// filled, for too many buffers running, and the sound helper stopped
    /// them with all notes released (page-rules.md RULE-62): fault 7 for
    /// that view's open page.
    LocalOverrun {
        /// The view's channel.
        channel: u32,
        /// How many times the view's local sounds were stopped since the
        /// last report; 0 when the small device buffer was given up for
        /// the usual one and the sounds go on (RULE-97 item 3).
        count: u16,
    },
    /// A cue of a sound on one of the view's audio channels was heard:
    /// the device buffer holding its sample was rendered (RULE-109).
    CueHeard {
        /// The view's channel.
        channel: u32,
        /// The audio channel, 0 to 3.
        audio_channel: u8,
        /// The cue's number, 0 to `MAX_CUE`.
        number: u8,
    },
    /// The processor time the helper spent rendering one view's sounds
    /// since it last said, measured around its render calls (REQ-6626
    /// item 1 b).
    Spent {
        /// The view's channel.
        channel: u32,
        /// Microseconds.
        microseconds: u32,
    },
}

/// # Errors
/// When writing fails or the message breaks a bound.
pub fn write_to_sound(out: &mut dyn Write, message: &ToSound) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        ToSound::Audio { channel, audio } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            // Checked on its own, as the reader reads it after the channel.
            let mut operation = Vec::new();
            audio::push(&mut operation, audio, Sender::Core)?;
            payload.extend_from_slice(&operation);
            KIND_AUDIO
        }
        ToSound::Settings { sound, volume } => {
            if *volume > MAX_VOLUME {
                return Err(ProtocolError::BadValue("volume"));
            }
            payload.push(u8::from(*sound));
            payload.push(*volume);
            KIND_SETTINGS
        }
        ToSound::Close => KIND_CLOSE,
    };
    write_frame(out, kind, &payload)
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors and any malformed or out-of-bounds message.
pub fn read_to_sound(input: &mut dyn Read) -> Result<Option<ToSound>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_AUDIO => ToSound::Audio { channel: fields.u32()?, audio: audio::read(&mut fields, Sender::Core)? },
        KIND_SETTINGS => {
            let sound = fields.flag("sound")?;
            let volume = fields.u8()?;
            if volume > MAX_VOLUME {
                return Err(ProtocolError::BadValue("volume"));
            }
            ToSound::Settings { sound, volume }
        }
        KIND_CLOSE => ToSound::Close,
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// # Errors
/// When writing fails or the message breaks a bound.
pub fn write_from_sound(out: &mut dyn Write, message: &FromSound) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        FromSound::Sounding(channels) => {
            push_channels(&mut payload, channels)?;
            KIND_SOUNDING
        }
        FromSound::Metered { channel, values } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            super::push_count_u8(&mut payload, values.len(), audio::MAX_WATCH_SLOTS, "metered values")?;
            payload.extend_from_slice(values);
            KIND_METERED
        }
        FromSound::LocalOverrun { channel, count } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            payload.extend_from_slice(&count.to_be_bytes());
            KIND_LOCAL_OVERRUN
        }
        FromSound::CueHeard { channel, audio_channel, number } => {
            crate::rules::check_cue(*audio_channel, *number)?;
            payload.extend_from_slice(&channel.to_be_bytes());
            payload.extend_from_slice(&[*audio_channel, *number]);
            KIND_CUE_HEARD
        }
        FromSound::Spent { channel, microseconds } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            payload.extend_from_slice(&microseconds.to_be_bytes());
            KIND_SPENT
        }
    };
    write_frame(out, kind, &payload)
}

/// Reads one message from the sound helper, which is untrusted, or `None`
/// at a clean end.
///
/// # Errors
/// On input errors and any malformed or out-of-bounds message.
pub fn read_from_sound(input: &mut dyn Read) -> Result<Option<FromSound>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_SOUNDING => FromSound::Sounding(read_channels(&mut fields)?),
        KIND_METERED => {
            let channel = fields.u32()?;
            let count = fields.count_u8(audio::MAX_WATCH_SLOTS, "metered values")?;
            FromSound::Metered { channel, values: fields.take(count)?.to_vec() }
        }
        KIND_LOCAL_OVERRUN => FromSound::LocalOverrun { channel: fields.u32()?, count: fields.u16()? },
        KIND_CUE_HEARD => {
            let (channel, audio_channel, number) = (fields.u32()?, fields.u8()?, fields.u8()?);
            crate::rules::check_cue(audio_channel, number)?;
            FromSound::CueHeard { channel, audio_channel, number }
        }
        KIND_SPENT => FromSound::Spent { channel: fields.u32()?, microseconds: fields.u32()? },
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use crate::MAX_TABS;
    use crate::audio::Sound;

    #[test]
    fn every_kind_round_trips() {
        for message in [
            ToSound::Audio { channel: 3, audio: Audio::Define { sound: Sound { handle: 1, generation: 2 }, rate_hz: 8_000, samples: 400 } },
            ToSound::Audio { channel: 3, audio: Audio::Mute(true) },
            ToSound::Settings { sound: false, volume: 40 },
            ToSound::Close,
        ] {
            let mut bytes = Vec::new();
            write_to_sound(&mut bytes, &message).unwrap();
            assert_eq!(read_to_sound(&mut bytes.as_slice()).unwrap(), Some(message));
        }
        let mut bytes = Vec::new();
        write_from_sound(&mut bytes, &FromSound::Sounding(vec![1, 7])).unwrap();
        assert_eq!(read_from_sound(&mut bytes.as_slice()).unwrap(), Some(FromSound::Sounding(vec![1, 7])));
    }

    #[test]
    fn out_of_bounds_is_refused() {
        assert!(write_to_sound(&mut Vec::new(), &ToSound::Settings { sound: true, volume: 101 }).is_err());
        let mut bytes = Vec::new();
        write_frame(&mut bytes, KIND_SETTINGS, &[2, 50]).unwrap();
        assert!(read_to_sound(&mut bytes.as_slice()).is_err(), "a flag that is not one");
        let mut bytes = Vec::new();
        let too_many: Vec<u8> = (0..=u32::try_from(MAX_TABS).unwrap()).flat_map(u32::to_be_bytes).collect();
        write_frame(&mut bytes, KIND_SOUNDING, &too_many).unwrap();
        assert!(read_from_sound(&mut bytes.as_slice()).is_err());
        let mut bytes = Vec::new();
        write_frame(&mut bytes, KIND_CLOSE, &[]).unwrap();
        assert!(read_from_sound(&mut bytes.as_slice()).is_err(), "the sound helper may not send what the core sends");
    }
}

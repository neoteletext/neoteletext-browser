//! Messages between the trusted core and the sound helper (decision 55).
//! The core passes on the voice operations it checked (requirement 16.4)
//! and the settings' sound and volume (requirement 19.4); the sound helper
//! says which channels sound, for the tabs' speakers and so a sounding
//! tab stays awake (decision 47).

use std::io::{Read, Write};

use super::audio::{self, Audio, Sender};
use super::settings::MAX_VOLUME;
use super::{Fields, MAX_TABS, ProtocolError, read_frame, write_frame};

const KIND_AUDIO: u8 = 1;
const KIND_SETTINGS: u8 = 2;
const KIND_CLOSE: u8 = 3;
const KIND_SOUNDING: u8 = 4;

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
            let sound = match fields.u8()? {
                0 => false,
                1 => true,
                _ => return Err(ProtocolError::BadValue("sound")),
            };
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
            if channels.len() > MAX_TABS {
                return Err(ProtocolError::BadValue("too many channels"));
            }
            for channel in channels {
                payload.extend_from_slice(&channel.to_be_bytes());
            }
            KIND_SOUNDING
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
        KIND_SOUNDING => {
            let mut channels = Vec::new();
            while !fields.is_empty() {
                if channels.len() == MAX_TABS {
                    return Err(ProtocolError::BadValue("too many channels"));
                }
                channels.push(fields.u32()?);
            }
            FromSound::Sounding(channels)
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
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

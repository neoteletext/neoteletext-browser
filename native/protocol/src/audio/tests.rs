//! The voice operations' tests: every operation round trips on both pipes,
//! a decoder cannot reset or mute, and every bound is kept.

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
    let start = |channel, gain, fade_ms, duck_percent| Audio::Start { channel, sound: SOUND, gain, fade_ms, looped: false, duck_percent, after_current: false };
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

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
        Audio::Stream { channel: 2, sound: SOUND, rate_hz: 8_000 },
        Audio::StreamSamples { channel: 2, sound: SOUND, pcm: vec![1, -1] },
        Audio::EndStream { channel: 3, sound: SOUND },
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
    refused(Audio::Stream { channel: 0, sound: SOUND, rate_hz: 12_345 }, "sample rate");
    refused(Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![] }, "samples");
    refused(Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![0; MAX_MESSAGE_SAMPLES + 1] }, "samples");
    // A stream names one of the profile's audio channels.
    refused(Audio::Stream { channel: CHANNELS, sound: SOUND, rate_hz: 8_000 }, "audio channel");
    refused(Audio::StreamSamples { channel: CHANNELS, sound: SOUND, pcm: vec![0] }, "audio channel");
    refused(Audio::EndStream { channel: CHANNELS, sound: SOUND }, "audio channel");
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

fn local_start() -> LocalStart {
    LocalStart { sound: SOUND, family: 14, variant: 1, option: 0, split: 0, gain: 100, fade_ms: 5, duck_percent: 100, setup: vec![0, 3, 0, 0xC0, 0] }
}

fn local_operations() -> Vec<Audio> {
    vec![
        Audio::LocalBank { channel: 0, sound: SOUND, index: 9, image: vec![7; MAX_BANK_IMAGE] },
        Audio::LocalBank { channel: 3, sound: SOUND, index: 0, image: vec![1] },
        Audio::Local { channel: 1, start: local_start() },
        Audio::Local { channel: 1, start: LocalStart { family: 17, option: 2, split: 60, setup: vec![0; MAX_LOCAL_SETUP], ..local_start() } },
        Audio::Local { channel: 1, start: LocalStart { setup: vec![], ..local_start() } },
        Audio::LocalMidi { channel: 2, status: 0x90, data1: 60, data2: 127 },
        Audio::LocalMidi { channel: 2, status: 0x8F, data1: 0, data2: 0 },
        Audio::LocalMidi { channel: 2, status: 0xB3, data1: 7, data2: 100 },
        Audio::LocalMidi { channel: 2, status: 0xC0, data1: 5, data2: 0 },
        Audio::LocalMidi { channel: 2, status: 0xE1, data1: 0, data2: 64 },
    ]
}

/// RULE-63 (REQ-6535): the local sound's operations round trip on both
/// pipes at their bounds, and the watch list on the trusted core's alone.
#[test]
fn the_local_operations_round_trip_on_both_pipes() {
    for audio in local_operations() {
        let mut bytes = Vec::new();
        decoder_writes(&mut bytes, &FromDecoder::Audio(audio.clone())).unwrap();
        assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap(), Some(FromDecoder::Audio(audio.clone())));
        let mut bytes = Vec::new();
        write_to_sound(&mut bytes, &ToSound::Audio { channel: 7, audio: audio.clone() }).unwrap();
        assert_eq!(read_to_sound(&mut bytes.as_slice()).unwrap(), Some(ToSound::Audio { channel: 7, audio }));
    }
    let slots = |count: usize| (0..count).map(|slot| WatchSlot { channel: 3, kind: 4, part: u8::try_from(slot % 16).unwrap() }).collect::<Vec<_>>();
    for watch in [Audio::Watch(vec![]), Audio::Watch(vec![WatchSlot { channel: 0, kind: 1, part: 0 }]), Audio::Watch(slots(MAX_WATCH_SLOTS))] {
        let mut bytes = Vec::new();
        write_to_sound(&mut bytes, &ToSound::Audio { channel: 1, audio: watch.clone() }).unwrap();
        assert_eq!(read_to_sound(&mut bytes.as_slice()).unwrap(), Some(ToSound::Audio { channel: 1, audio: watch }));
    }
    assert!(write_to_sound(&mut Vec::new(), &ToSound::Audio { channel: 1, audio: Audio::Watch(slots(MAX_WATCH_SLOTS + 1)) }).is_err());
}

/// RULE-63 (REQ-6535): the decoder may not send `Watch`, which is the
/// trusted core's alone.
#[test]
fn a_decoder_cannot_send_a_watch_list() {
    assert!(decoder_writes(&mut Vec::new(), &FromDecoder::Audio(Audio::Watch(vec![]))).is_err());
    let mut payload = Vec::new();
    push(&mut payload, &Audio::Watch(vec![WatchSlot { channel: 0, kind: 1, part: 0 }]), Sender::Core).unwrap();
    assert!(read(&mut Fields::new(&payload), Sender::Decoder).is_err());
}

/// RULE-63, RULE-21 (REQ-6535): each field of the local operations is
/// held to its bound.
#[test]
fn local_operations_out_of_bounds_are_refused() {
    let refused = |audio: Audio| decoder_writes(&mut Vec::new(), &FromDecoder::Audio(audio)).is_err();
    assert!(refused(Audio::LocalBank { channel: 4, sound: SOUND, index: 0, image: vec![1] }));
    assert!(refused(Audio::LocalBank { channel: 0, sound: SOUND, index: 10, image: vec![1] }), "ten banks, 0 to 9");
    assert!(refused(Audio::LocalBank { channel: 0, sound: SOUND, index: 0, image: vec![] }));
    assert!(refused(Audio::LocalBank { channel: 0, sound: SOUND, index: 0, image: vec![0; MAX_BANK_IMAGE + 1] }));
    let local = |change: &dyn Fn(&mut LocalStart)| {
        let mut start = local_start();
        change(&mut start);
        refused(Audio::Local { channel: 0, start })
    };
    assert!(!local(&|_| {}));
    assert!(local(&|start| start.family = 12), "the synth6581 family has no local sound (RULE-60)");
    assert!(local(&|start| start.family = 19), "a bank is no instrument");
    assert!(local(&|start| start.variant = 3));
    assert!(local(&|start| start.option = 3));
    assert!(local(&|start| start.split = 128));
    assert!(local(&|start| start.gain = 101));
    assert!(local(&|start| start.fade_ms = 4));
    assert!(local(&|start| start.duck_percent = 33));
    assert!(local(&|start| start.setup = vec![0; MAX_LOCAL_SETUP + 1]));
    assert!(refused(Audio::LocalMidi { channel: 4, status: 0x90, data1: 0, data2: 0 }));
    for status in [0x00, 0x7F, 0xA0, 0xD0, 0xF0, 0xF8] {
        assert!(refused(Audio::LocalMidi { channel: 0, status, data1: 0, data2: 0 }), "status {status:#x} is not a rule's (RULE-21)");
    }
    assert!(refused(Audio::LocalMidi { channel: 0, status: 0x90, data1: 128, data2: 0 }));
    assert!(refused(Audio::LocalMidi { channel: 0, status: 0x90, data1: 0, data2: 128 }));
}

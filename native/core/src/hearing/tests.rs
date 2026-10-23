//! Tests of what of each channel's sound is heard.

use super::*;

const SOUND: Sound = Sound { handle: 1, generation: 1 };

fn define(samples: u32) -> Audio {
    Audio::Define { sound: SOUND, rate_hz: 8_000, samples }
}

fn start() -> Audio {
    Audio::Start { channel: 0, sound: SOUND, gain: 50, fade_ms: 5, looped: false, duck_percent: 100, after_current: false }
}

#[test]
fn every_channel_is_heard_and_its_operations_are_tagged_with_it() {
    let mut hearing = Hearing::default();
    assert_eq!(hearing.decoded(1, define(10)), Some(ToSound::Audio { channel: 1, audio: define(10) }));
    assert_eq!(hearing.decoded(2, define(10)), Some(ToSound::Audio { channel: 2, audio: define(10) }), "a background tab is heard");
    assert!(hearing.decoded(2, start()).is_some());
    assert!(hearing.decoded(3, start()).is_none(), "each channel's sounds are its own");
}

#[test]
fn the_opened_page_goes_to_the_active_channels_decoder_and_others_keep_theirs() {
    let mut hearing = Hearing::default();
    let page = |channel, page| Some(PageKey { channel, page });
    assert_eq!(hearing.open(Some(4), page(4, 0x100)), [Send::Decoder(4, ToDecoder::Opened(Some(0x100)))]);
    assert_eq!(hearing.open(Some(4), page(4, 0x100)), []);
    assert_eq!(hearing.open(Some(4), page(9, 0x200)), [Send::Decoder(4, ToDecoder::Opened(None))], "another channel's page is no page");
    assert_eq!(hearing.open(Some(4), page(4, 0x100)), [Send::Decoder(4, ToDecoder::Opened(Some(0x100)))]);
    // Another tab becomes active: channel 4 keeps page 100 open.
    assert_eq!(hearing.open(Some(5), None), []);
    assert_eq!(hearing.open(Some(5), page(5, 0x300)), [Send::Decoder(5, ToDecoder::Opened(Some(0x300)))]);
    assert_eq!(hearing.open(None, None), []);
}

#[test]
fn muting_tells_the_interface_once_per_change() {
    let mut hearing = Hearing::default();
    assert_eq!(hearing.mute(3, false), None, "channels start unmuted");
    assert_eq!(hearing.mute(3, true), Some(Send::Sound(ToSound::Audio { channel: 3, audio: Audio::Mute(true) })));
    assert_eq!(hearing.mute(3, true), None);
    assert_eq!(hearing.mute(3, false), Some(Send::Sound(ToSound::Audio { channel: 3, audio: Audio::Mute(false) })));
}

#[test]
fn operations_are_checked_against_what_was_defined() {
    let mut hearing = Hearing::default();
    assert!(hearing.decoded(1, start()).is_none(), "a sound never defined");
    assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 0, pcm: vec![0; 4] }).is_none());
    hearing.decoded(1, define(10)).unwrap();
    assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 8, pcm: vec![0; 2] }).is_some());
    assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 9, pcm: vec![0; 2] }).is_none(), "beyond its length");
    assert!(hearing.decoded(1, Audio::StreamSamples { sound: SOUND, pcm: vec![0; 2] }).is_none(), "not a live sound");
    assert!(hearing.decoded(1, start()).is_some());
    assert!(hearing.decoded(1, Audio::Reset).is_none());
    assert!(hearing.decoded(1, Audio::Mute(true)).is_none());
    assert!(hearing.decoded(1, Audio::Forget(SOUND)).is_some());
    assert!(hearing.decoded(1, start()).is_none(), "forgotten");
}

#[test]
fn a_live_sound_takes_stream_samples_only() {
    let mut hearing = Hearing::default();
    assert!(hearing.decoded(1, Audio::StreamSamples { sound: SOUND, pcm: vec![0; 2] }).is_none(), "never announced");
    assert!(hearing.decoded(1, Audio::Stream { sound: SOUND, rate_hz: 8_000 }).is_some());
    assert!(hearing.decoded(1, Audio::StreamSamples { sound: SOUND, pcm: vec![0; 2] }).is_some());
    assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 0, pcm: vec![0; 2] }).is_none(), "a live sound has no positions");
    assert!(hearing.decoded(1, start()).is_some());
}

#[test]
fn the_ledger_bounds_what_the_interface_holds() {
    let mut hearing = Hearing::default();
    let big = u32::try_from(MAX_HELD_SAMPLES / 2).unwrap();
    for handle in 0..2 {
        let sound = Sound { handle, generation: 1 };
        assert!(hearing.decoded(1, Audio::Define { sound, rate_hz: 48_000, samples: big }).is_some());
    }
    let third = Sound { handle: 2, generation: 1 };
    assert!(hearing.decoded(1, Audio::Define { sound: third, rate_hz: 48_000, samples: 1 }).is_none(), "over the channel's ledger");
    // Redefining a sound counts it once; forgetting one frees its room.
    assert!(hearing.decoded(1, Audio::Define { sound: Sound { handle: 0, generation: 1 }, rate_hz: 48_000, samples: big }).is_some());
    hearing.decoded(1, Audio::Forget(Sound { handle: 0, generation: 1 })).unwrap();
    assert!(hearing.decoded(1, Audio::Define { sound: third, rate_hz: 48_000, samples: 1 }).is_some());
    // A second channel fills the rest of the total; a third has no room.
    for handle in 0..2 {
        assert!(hearing.decoded(2, Audio::Define { sound: Sound { handle, generation: 1 }, rate_hz: 48_000, samples: big }).is_some());
    }
    assert!(hearing.decoded(3, Audio::Define { sound: SOUND, rate_hz: 48_000, samples: big }).is_none(), "over the total");
    assert_eq!(hearing.channel_gone(1), [Send::Sound(ToSound::Audio { channel: 1, audio: Audio::Reset })]);
    assert_eq!(hearing.channel_gone(1), []);
    assert!(hearing.decoded(3, Audio::Define { sound: SOUND, rate_hz: 48_000, samples: big }).is_some(), "the gone channel's room is free");
}

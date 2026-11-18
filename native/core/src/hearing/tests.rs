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
    assert_eq!(hearing.decoded(1, define(10), 0), Some(ToSound::Audio { channel: 1, audio: define(10) }));
    assert_eq!(hearing.decoded(2, define(10), 0), Some(ToSound::Audio { channel: 2, audio: define(10) }), "a background tab is heard");
    assert!(hearing.decoded(2, start(), 0).is_some());
    assert!(hearing.decoded(3, start(), 0).is_none(), "each channel's sounds are its own");
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
    assert!(hearing.decoded(1, start(), 0).is_none(), "a sound never defined");
    assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 0, pcm: vec![0; 4] }, 0).is_none());
    hearing.decoded(1, define(10), 0).unwrap();
    assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 8, pcm: vec![0; 2] }, 0).is_some());
    assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 9, pcm: vec![0; 2] }, 0).is_none(), "beyond its length");
    assert!(hearing.decoded(1, Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![0; 2] }, 0).is_none(), "not a live sound");
    assert!(hearing.decoded(1, start(), 0).is_some());
    assert!(hearing.decoded(1, Audio::Reset, 0).is_none());
    assert!(hearing.decoded(1, Audio::Mute(true), 0).is_none());
    assert!(hearing.decoded(1, Audio::Forget(SOUND), 0).is_some());
    assert!(hearing.decoded(1, start(), 0).is_none(), "forgotten");
}

#[test]
fn a_live_sound_takes_stream_samples_only() {
    let mut hearing = Hearing::default();
    assert!(hearing.decoded(1, Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![0; 2] }, 0).is_none(), "never announced");
    assert!(hearing.decoded(1, Audio::Stream { channel: 0, sound: SOUND, rate_hz: 8_000 }, 0).is_some());
    assert!(hearing.decoded(1, Audio::StreamSamples { channel: 0, sound: SOUND, pcm: vec![0; 2] }, 0).is_some());
    assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 0, pcm: vec![0; 2] }, 0).is_none(), "a live sound has no positions");
    assert!(hearing.decoded(1, start(), 0).is_some());
}

#[test]
fn the_ledger_bounds_what_the_interface_holds() {
    let mut hearing = Hearing::default();
    let big = u32::try_from(MAX_HELD_SAMPLES / 2).unwrap();
    for handle in 0..2 {
        let sound = Sound { handle, generation: 1 };
        assert!(hearing.decoded(1, Audio::Define { sound, rate_hz: 48_000, samples: big }, 0).is_some());
    }
    let third = Sound { handle: 2, generation: 1 };
    assert!(hearing.decoded(1, Audio::Define { sound: third, rate_hz: 48_000, samples: 1 }, 0).is_none(), "over the channel's ledger");
    // Redefining a sound counts it once; forgetting one frees its room.
    assert!(hearing.decoded(1, Audio::Define { sound: Sound { handle: 0, generation: 1 }, rate_hz: 48_000, samples: big }, 0).is_some());
    hearing.decoded(1, Audio::Forget(Sound { handle: 0, generation: 1 }), 0).unwrap();
    assert!(hearing.decoded(1, Audio::Define { sound: third, rate_hz: 48_000, samples: 1 }, 0).is_some());
    // A second channel fills the rest of the total; a third has no room.
    for handle in 0..2 {
        assert!(hearing.decoded(2, Audio::Define { sound: Sound { handle, generation: 1 }, rate_hz: 48_000, samples: big }, 0).is_some());
    }
    assert!(hearing.decoded(3, Audio::Define { sound: SOUND, rate_hz: 48_000, samples: big }, 0).is_none(), "over the total");
    assert_eq!(hearing.channel_gone(1), [Send::Sound(ToSound::Audio { channel: 1, audio: Audio::Reset })]);
    assert_eq!(hearing.channel_gone(1), []);
    assert!(hearing.decoded(3, Audio::Define { sound: SOUND, rate_hz: 48_000, samples: big }, 0).is_some(), "the gone channel's room is free");
}

/// The engine's INT-36 and INT-37: a stream is named by its audio channel
/// and its sound, each charged on its own; its samples and its start go only
/// to the audio channel it streams on; its end lets it go, and a forget ends
/// every stream of the sound.
#[test]
fn each_audio_channels_stream_is_checked_and_charged_on_its_own() {
    let mut hearing = Hearing::default();
    let stream = |voice| Audio::Stream { channel: voice, sound: SOUND, rate_hz: 8_000 };
    let samples = |voice| Audio::StreamSamples { channel: voice, sound: SOUND, pcm: vec![0; 2] };
    let start_on = |voice| Audio::Start { channel: voice, sound: SOUND, gain: 50, fade_ms: 5, looped: false, duck_percent: 100, after_current: false };
    let end = |voice| Audio::EndStream { channel: voice, sound: SOUND };
    assert!(hearing.decoded(1, end(0), 0).is_none(), "no stream to end");
    assert!(hearing.decoded(1, stream(0), 0).is_some());
    assert!(hearing.decoded(1, stream(2), 0).is_some());
    assert_eq!(hearing.held, 2 * u64::from(STREAM_HELD_SAMPLES), "each stream charged");
    assert!(hearing.decoded(1, samples(2), 0).is_some());
    assert!(hearing.decoded(1, samples(1), 0).is_none(), "no stream on audio channel 1");
    assert!(hearing.decoded(1, start_on(1), 0).is_none(), "nothing to start on audio channel 1");
    assert!(hearing.decoded(1, start_on(2), 0).is_some());
    assert!(hearing.decoded(1, stream(2), 0).is_some(), "a stream under a held name replaces it");
    assert_eq!(hearing.held, 2 * u64::from(STREAM_HELD_SAMPLES), "charged once");
    assert!(hearing.decoded(1, end(0), 0).is_some());
    assert!(hearing.decoded(1, samples(0), 0).is_none(), "ended");
    assert!(hearing.decoded(1, end(0), 0).is_none(), "ended once");
    assert_eq!(hearing.held, u64::from(STREAM_HELD_SAMPLES));
    hearing.decoded(1, define(10), 0).unwrap();
    assert!(hearing.decoded(1, Audio::Forget(SOUND), 0).is_some());
    assert!(hearing.decoded(1, samples(2), 0).is_none(), "the forget ended the stream too");
    assert_eq!(hearing.held, 0);
    assert!(hearing.decoded(1, Audio::Forget(SOUND), 0).is_none(), "nothing left to forget");
}

/// RULE-21, RULE-60, RULE-63 (REQ-6535): a local sound's operations pass
/// the trusted core's check; at most two local sounds play at once in a
/// view and a third is refused; a rule's MIDI message goes only to an
/// audio channel a local sound was started on and not stopped or replaced
/// since; and a decoder cannot give a watch list.
#[test]
fn local_sounds_are_bounded_and_midi_goes_only_to_one() {
    use protocol::audio::{LocalStart, WatchSlot};
    let local = |channel: u8| Audio::Local {
        channel,
        start: LocalStart { sound: SOUND, family: 14, variant: 0, option: 0, split: 0, gain: 100, fade_ms: 5, duck_percent: 100, setup: vec![0, 0xC0, 0] },
    };
    let midi = |channel: u8| Audio::LocalMidi { channel, status: 0x90, data1: 60, data2: 100 };
    let mut hearing = Hearing::default();
    assert!(hearing.decoded(7, midi(0), 0).is_none(), "no local sound plays there");
    assert!(hearing.decoded(7, Audio::LocalBank { channel: 0, sound: SOUND, index: 0, image: vec![1; 64] }, 0).is_some());
    assert!(hearing.decoded(7, local(0), 0).is_some());
    assert!(hearing.decoded(7, midi(0), 0).is_some());
    assert!(hearing.decoded(7, midi(1), 0).is_none(), "another audio channel");
    assert!(hearing.decoded(8, midi(0), 0).is_none(), "another view");
    assert!(hearing.decoded(7, local(1), 0).is_some(), "a second");
    assert!(hearing.decoded(7, local(2), 0).is_none(), "a third is refused");
    assert!(hearing.decoded(7, local(1), 0).is_some(), "one started again on its own audio channel is no third");
    assert!(hearing.decoded(7, Audio::Stop { channel: 0, fade_ms: 5 }, 0).is_some());
    assert!(hearing.decoded(7, midi(0), 0).is_none(), "stopped: its notes are no longer its page's to play");
    assert!(hearing.decoded(7, local(2), 0).is_some(), "and there is room for another");
    assert!(hearing.decoded(7, Audio::Watch(vec![WatchSlot { channel: 0, kind: 1, part: 0 }]), 0).is_none(), "only the trusted core gives a watch list");
}

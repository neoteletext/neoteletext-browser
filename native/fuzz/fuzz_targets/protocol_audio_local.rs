//! The voice operations of a local sound and of metering (neoteletext-common's
//! page-rules.md RULE-63, RULE-83): whatever bytes a decoder or the
//! trusted core's pipe to the sound helper carries, an operation that is
//! read holds every bound, a decoder's is never a watch list or a pause,
//! and it writes back to the bytes that read as it; the second form's cue
//! points and the sound helper's cues heard and time spent hold theirs
//! (RULE-109, RULE-111, REQ-6626).
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::audio::{Audio, MAX_BANK_IMAGE, MAX_LOCAL_SETUP, MAX_WATCH_SLOTS};
use protocol::decoder::{FromDecoder, read_from_decoder, write_to_core};
use protocol::rules::MAX_CUE;
use protocol::sound::{FromSound, ToSound, read_from_sound, read_to_sound, write_from_sound, write_to_sound};

/// The audio channels of a view.
const CHANNELS: u8 = 4;
/// The codec codes of the families that have a local sound (RULE-60).
const FAMILIES: core::ops::RangeInclusive<u8> = 13..=18;
/// The bank indexes of a sound (MIDI-57).
const BANKS: u8 = 10;

/// Holds one operation to the bounds of RULE-63.
fn check(audio: &Audio, from_decoder: bool) {
    match audio {
        Audio::LocalBank { channel, index, image, .. } => {
            assert!(*channel < CHANNELS && *index < BANKS);
            assert!(!image.is_empty() && image.len() <= MAX_BANK_IMAGE);
        }
        Audio::Local { channel, start } => {
            assert!(*channel < CHANNELS);
            assert!(FAMILIES.contains(&start.family) && start.variant <= 2 && start.option <= 2 && start.split <= 127);
            assert!(start.gain <= 100 && (5..=10_000).contains(&start.fade_ms));
            assert!(start.setup.len() <= MAX_LOCAL_SETUP);
        }
        // RULE-21: note off, note on, control change, program change or
        // pitch bend, each data byte 0 to 127.
        Audio::LocalMidi { channel, status, data1, data2 } => {
            assert!(*channel < CHANNELS);
            assert!(matches!(status & 0xF0, 0x80 | 0x90 | 0xB0 | 0xC0 | 0xE0));
            assert!(*data1 <= 127 && *data2 <= 127);
        }
        Audio::Watch(slots) => {
            assert!(!from_decoder, "only the trusted core gives a watch list");
            assert!(slots.len() <= MAX_WATCH_SLOTS);
            for slot in slots {
                assert!(slot.channel < CHANNELS && (1..=4).contains(&slot.kind) && slot.part <= 15);
                assert!(slot.kind != 1 || slot.part == 0, "a level has no part");
            }
        }
        Audio::Cue { channel, number, .. } => assert!(*channel < CHANNELS && *number <= MAX_CUE),
        Audio::Pause(_) => assert!(!from_decoder, "only the trusted core pauses a view's local sounds"),
        _ => {}
    }
}

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = read_from_decoder(&mut input) {
        if let FromDecoder::Audio(audio) = &message {
            check(audio, true);
            let mut written = Vec::new();
            assert!(write_to_core(&mut written, &message).is_ok(), "what was read is written");
            assert_eq!(read_from_decoder(&mut written.as_slice()).ok().flatten().as_ref(), Some(&message));
        }
    }
    let mut input = data;
    while let Ok(Some(message)) = read_to_sound(&mut input) {
        if let ToSound::Audio { audio, .. } = &message {
            check(audio, false);
            let mut written = Vec::new();
            assert!(write_to_sound(&mut written, &message).is_ok(), "what was read is written");
            assert_eq!(read_to_sound(&mut written.as_slice()).ok().flatten().as_ref(), Some(&message));
        }
    }
    // The sound helper's own messages: a metered report within the watch
    // list's bound.
    let mut input = data;
    while let Ok(Some(message)) = read_from_sound(&mut input) {
        match &message {
            FromSound::Metered { values, .. } => assert!(values.len() <= MAX_WATCH_SLOTS),
            FromSound::CueHeard { audio_channel, number, .. } => assert!(*audio_channel < CHANNELS && *number <= MAX_CUE),
            _ => {}
        }
        let mut written = Vec::new();
        assert!(write_from_sound(&mut written, &message).is_ok());
        assert_eq!(read_from_sound(&mut written.as_slice()).ok().flatten().as_ref(), Some(&message), "it reads back as itself");
    }
});

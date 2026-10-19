//! What of each channel's sound reaches the interface (`architecture.md`
//! section 6, the engine's area AUD, decision 38). Every tab is heard, as in
//! a web browser: a background tab's sound keeps playing until the user
//! mutes it. Every voice operation is checked here again, as frames are,
//! and a ledger bounds the samples the decoders can make the interface hold.

use std::collections::BTreeMap;

use interface_core::canvas::PageKey;
use protocol::ToInterface;
use protocol::audio::{Audio, Sound};
use protocol::decoder::ToDecoder;

/// The most samples of sound the interface holds for one channel: 32 MB
/// of 16-bit PCM.
pub const MAX_HELD_SAMPLES: u64 = 16 * 1024 * 1024;
/// The most it holds for every channel together: two channels' worth.
pub const MAX_TOTAL_HELD_SAMPLES: u64 = 2 * MAX_HELD_SAMPLES;
/// What a live sound is charged in the ledger: two seconds at the highest
/// rate, above the 1.6-second ring the interface's mixer keeps of one.
pub const STREAM_HELD_SAMPLES: u32 = 48_000 * 2;

/// A message for a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Send {
    /// To the decoder of a channel.
    Decoder(u32, ToDecoder),
    /// To the interface.
    Interface(ToInterface),
}

/// What of one channel's sound reached the interface.
#[derive(Debug, Default)]
struct Heard {
    /// The page the user last opened in the channel's tab.
    opened: Option<u16>,
    /// Each defined sound and what it is charged; live sounds are marked.
    sounds: BTreeMap<Sound, (u32, bool)>,
    held: u64,
    muted: bool,
}

/// The trusted core's record of what is heard.
#[derive(Debug, Default)]
pub struct Hearing {
    channels: BTreeMap<u32, Heard>,
    held: u64,
}

impl Hearing {
    /// The interface reports the page the user opened in the active tab,
    /// whose channel is `active`. Only a page of that channel counts. Other
    /// tabs keep the page they had open, and its sound.
    pub fn open(&mut self, active: Option<u32>, key: Option<PageKey>) -> Vec<Send> {
        let Some(active) = active else { return Vec::new() };
        let page = key.filter(|key| key.channel == active).map(|key| key.page);
        let heard = self.channels.entry(active).or_default();
        if page == heard.opened {
            return Vec::new();
        }
        heard.opened = page;
        vec![Send::Decoder(active, ToDecoder::Opened(page))]
    }

    /// The user muted or unmuted `channel`'s tab: what to tell the
    /// interface, if it changed.
    pub fn mute(&mut self, channel: u32, muted: bool) -> Option<Send> {
        let heard = self.channels.entry(channel).or_default();
        if heard.muted == muted {
            return None;
        }
        heard.muted = muted;
        Some(Send::Interface(ToInterface::Audio { channel, audio: Audio::Mute(muted) }))
    }

    /// A voice operation from the decoder of `channel`: what to pass on, if
    /// anything. An operation on a sound never defined, beyond its length,
    /// or beyond the ledger is dropped.
    pub fn decoded(&mut self, channel: u32, audio: Audio) -> Option<ToInterface> {
        let heard = self.channels.entry(channel).or_default();
        let charge = |heard: &mut Heard, total: &mut u64, sound: Sound, samples: u32, live: bool| -> Option<()> {
            let previous = u64::from(heard.sounds.get(&sound).map_or(0, |(samples, _)| *samples));
            let held = heard.held.saturating_sub(previous).saturating_add(u64::from(samples));
            let all = total.saturating_sub(previous).saturating_add(u64::from(samples));
            if held > MAX_HELD_SAMPLES || all > MAX_TOTAL_HELD_SAMPLES {
                return None;
            }
            heard.held = held;
            *total = all;
            heard.sounds.insert(sound, (samples, live));
            Some(())
        };
        match &audio {
            Audio::Define { sound, samples, .. } => charge(heard, &mut self.held, *sound, *samples, false)?,
            Audio::Stream { sound, .. } => charge(heard, &mut self.held, *sound, STREAM_HELD_SAMPLES, true)?,
            Audio::Samples { sound, first, pcm } => {
                let (length, live) = heard.sounds.get(sound)?;
                let end = u64::from(*first).saturating_add(u64::try_from(pcm.len()).unwrap_or(u64::MAX));
                if *live || end > u64::from(*length) {
                    return None;
                }
            }
            Audio::StreamSamples { sound, .. } => {
                if !heard.sounds.get(sound)?.1 {
                    return None;
                }
            }
            Audio::Forget(sound) => {
                let (length, _) = heard.sounds.remove(sound)?;
                heard.held = heard.held.saturating_sub(u64::from(length));
                self.held = self.held.saturating_sub(u64::from(length));
            }
            Audio::Start { sound, .. } => {
                heard.sounds.get(sound)?;
            }
            Audio::Stop { .. } | Audio::Gain { .. } => {}
            // Only the trusted core resets or mutes; the protocol refuses
            // both from a decoder, and they are refused here too.
            Audio::Reset | Audio::Mute(_) => return None,
        }
        Some(ToInterface::Audio { channel, audio })
    }

    /// `channel` has no decoder any more (its tab closed or moved on, or
    /// the decoder ended): its sounds go.
    pub fn channel_gone(&mut self, channel: u32) -> Vec<Send> {
        let Some(heard) = self.channels.remove(&channel) else { return Vec::new() };
        self.held = self.held.saturating_sub(heard.held);
        vec![Send::Interface(ToInterface::Audio { channel, audio: Audio::Reset })]
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(hearing.decoded(1, define(10)), Some(ToInterface::Audio { channel: 1, audio: define(10) }));
        assert_eq!(hearing.decoded(2, define(10)), Some(ToInterface::Audio { channel: 2, audio: define(10) }), "a background tab is heard");
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
        assert_eq!(hearing.mute(3, true), Some(Send::Interface(ToInterface::Audio { channel: 3, audio: Audio::Mute(true) })));
        assert_eq!(hearing.mute(3, true), None);
        assert_eq!(hearing.mute(3, false), Some(Send::Interface(ToInterface::Audio { channel: 3, audio: Audio::Mute(false) })));
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
        assert_eq!(hearing.channel_gone(1), [Send::Interface(ToInterface::Audio { channel: 1, audio: Audio::Reset })]);
        assert_eq!(hearing.channel_gone(1), []);
        assert!(hearing.decoded(3, Audio::Define { sound: SOUND, rate_hz: 48_000, samples: big }).is_some(), "the gone channel's room is free");
    }
}

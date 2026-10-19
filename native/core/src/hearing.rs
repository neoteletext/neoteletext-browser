//! Which channel is heard, and what of its sound reaches the interface
//! (`architecture.md` section 6, the engine's area AUD). Only the active
//! tab's decoder is heard; switching tabs makes the interface forget every
//! sound. Every voice operation is checked here again, as frames are, and a
//! ledger bounds the samples one decoder can make the interface hold.

use std::collections::BTreeMap;

use interface_core::canvas::PageKey;
use protocol::ToInterface;
use protocol::audio::{Audio, Sound};
use protocol::decoder::ToDecoder;

/// The most samples of sound the interface holds for the channel heard:
/// 32 MB of 16-bit PCM.
pub const MAX_HELD_SAMPLES: u64 = 16 * 1024 * 1024;

/// A message for a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Send {
    /// To the decoder of a channel.
    Decoder(u32, ToDecoder),
    /// To the interface.
    Interface(ToInterface),
}

/// The trusted core's record of what is heard.
#[derive(Debug, Default)]
pub struct Hearing {
    heard: Option<u32>,
    opened: Option<PageKey>,
    /// Each defined sound of the channel heard, and its length.
    sounds: BTreeMap<Sound, u32>,
    held: u64,
}

impl Hearing {
    /// The channel heard.
    #[must_use]
    pub fn heard(&self) -> Option<u32> {
        self.heard
    }

    /// The active tab shows `active`: it becomes the channel heard.
    pub fn hear(&mut self, active: Option<u32>) -> Vec<Send> {
        if active == self.heard {
            return Vec::new();
        }
        let mut sends = Vec::new();
        if let Some(old) = self.heard {
            sends.push(Send::Decoder(old, ToDecoder::Opened(None)));
            sends.push(Send::Decoder(old, ToDecoder::Audible(false)));
        }
        sends.push(Send::Interface(ToInterface::Audio(Audio::Reset)));
        self.sounds.clear();
        self.held = 0;
        self.opened = None;
        self.heard = active;
        if let Some(new) = active {
            sends.push(Send::Decoder(new, ToDecoder::Audible(true)));
        }
        sends
    }

    /// The interface reports the page the user opened. Only a page of the
    /// channel heard counts.
    pub fn open(&mut self, key: Option<PageKey>) -> Vec<Send> {
        let Some(heard) = self.heard else { return Vec::new() };
        let key = key.filter(|key| key.channel == heard);
        if key == self.opened {
            return Vec::new();
        }
        self.opened = key;
        vec![Send::Decoder(heard, ToDecoder::Opened(key.map(|key| key.page)))]
    }

    /// A voice operation from the decoder of `channel`: what to pass on, if
    /// anything. A channel not heard is silent; an operation on a sound never
    /// defined, beyond its length, or beyond the ledger is dropped.
    pub fn decoded(&mut self, channel: u32, audio: Audio) -> Option<ToInterface> {
        if self.heard != Some(channel) {
            return None;
        }
        match &audio {
            Audio::Define { sound, samples, .. } => {
                let previous = self.sounds.get(sound).copied().unwrap_or(0);
                let held = self.held.saturating_sub(u64::from(previous)).saturating_add(u64::from(*samples));
                if held > MAX_HELD_SAMPLES {
                    return None;
                }
                self.held = held;
                self.sounds.insert(*sound, *samples);
            }
            Audio::Samples { sound, first, pcm } => {
                let length = self.sounds.get(sound)?;
                let end = u64::from(*first).saturating_add(u64::try_from(pcm.len()).unwrap_or(u64::MAX));
                if end > u64::from(*length) {
                    return None;
                }
            }
            Audio::Forget(sound) => {
                let length = self.sounds.remove(sound)?;
                self.held = self.held.saturating_sub(u64::from(length));
            }
            Audio::Start { sound, .. } => {
                self.sounds.get(sound)?;
            }
            Audio::Stop { .. } | Audio::Gain { .. } => {}
            // Only the trusted core resets; the protocol refuses it from a
            // decoder, and it is refused here too.
            Audio::Reset => return None,
        }
        Some(ToInterface::Audio(audio))
    }

    /// The decoder of `channel` ended: if it was heard, its sounds go.
    pub fn decoder_ended(&mut self, channel: u32) -> Vec<Send> {
        if self.heard != Some(channel) {
            return Vec::new();
        }
        self.sounds.clear();
        self.held = 0;
        vec![Send::Interface(ToInterface::Audio(Audio::Reset))]
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
    fn only_the_active_channel_is_heard_and_switching_resets_the_interface() {
        let mut hearing = Hearing::default();
        assert_eq!(hearing.hear(Some(1)), [Send::Interface(ToInterface::Audio(Audio::Reset)), Send::Decoder(1, ToDecoder::Audible(true))]);
        assert!(hearing.decoded(2, define(10)).is_none(), "a background tab is silent");
        assert!(hearing.decoded(1, define(10)).is_some());
        assert_eq!(
            hearing.hear(Some(2)),
            [
                Send::Decoder(1, ToDecoder::Opened(None)),
                Send::Decoder(1, ToDecoder::Audible(false)),
                Send::Interface(ToInterface::Audio(Audio::Reset)),
                Send::Decoder(2, ToDecoder::Audible(true)),
            ]
        );
        assert!(hearing.decoded(1, start()).is_none());
        assert_eq!(hearing.hear(Some(2)), [], "the same channel again changes nothing");
    }

    #[test]
    fn the_opened_page_goes_to_the_heard_channels_decoder() {
        let mut hearing = Hearing::default();
        hearing.hear(Some(4));
        let page = |channel, page| Some(PageKey { channel, page });
        assert_eq!(hearing.open(page(4, 0x100)), [Send::Decoder(4, ToDecoder::Opened(Some(0x100)))]);
        assert_eq!(hearing.open(page(4, 0x100)), []);
        assert_eq!(hearing.open(page(9, 0x200)), [Send::Decoder(4, ToDecoder::Opened(None))], "another channel's page is no page");
        assert_eq!(hearing.open(None), []);
    }

    #[test]
    fn operations_are_checked_against_what_was_defined() {
        let mut hearing = Hearing::default();
        hearing.hear(Some(1));
        assert!(hearing.decoded(1, start()).is_none(), "a sound never defined");
        assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 0, pcm: vec![0; 4] }).is_none());
        hearing.decoded(1, define(10)).unwrap();
        assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 8, pcm: vec![0; 2] }).is_some());
        assert!(hearing.decoded(1, Audio::Samples { sound: SOUND, first: 9, pcm: vec![0; 2] }).is_none(), "beyond its length");
        assert!(hearing.decoded(1, start()).is_some());
        assert!(hearing.decoded(1, Audio::Reset).is_none());
        assert!(hearing.decoded(1, Audio::Forget(SOUND)).is_some());
        assert!(hearing.decoded(1, start()).is_none(), "forgotten");
    }

    #[test]
    fn the_ledger_bounds_what_the_interface_holds() {
        let mut hearing = Hearing::default();
        hearing.hear(Some(1));
        let big = u32::try_from(MAX_HELD_SAMPLES / 2).unwrap();
        for handle in 0..2 {
            let sound = Sound { handle, generation: 1 };
            assert!(hearing.decoded(1, Audio::Define { sound, rate_hz: 48_000, samples: big }).is_some());
        }
        let third = Sound { handle: 2, generation: 1 };
        assert!(hearing.decoded(1, Audio::Define { sound: third, rate_hz: 48_000, samples: 1 }).is_none(), "over the ledger");
        // Redefining a sound counts it once; forgetting one frees its room.
        assert!(hearing.decoded(1, Audio::Define { sound: Sound { handle: 0, generation: 1 }, rate_hz: 48_000, samples: big }).is_some());
        hearing.decoded(1, Audio::Forget(Sound { handle: 0, generation: 1 })).unwrap();
        assert!(hearing.decoded(1, Audio::Define { sound: third, rate_hz: 48_000, samples: 1 }).is_some());
        assert_eq!(hearing.decoder_ended(1), [Send::Interface(ToInterface::Audio(Audio::Reset))]);
        assert_eq!(hearing.decoder_ended(7), []);
    }
}

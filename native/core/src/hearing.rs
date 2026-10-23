//! What of each channel's sound reaches the sound helper (`architecture.md`
//! section 6, the engine's area AUD, decisions 38 and 55). Every tab is
//! heard, as in a web browser: a background tab's sound keeps playing until
//! the user mutes it. Every voice operation is checked here again, as
//! frames are, and a ledger bounds the samples the decoders can make the
//! sound helper hold.

use std::collections::BTreeMap;

use interface_core::canvas::PageKey;
use protocol::audio::{Audio, Sound};
use protocol::decoder::ToDecoder;
use protocol::sound::ToSound;

/// The most samples of sound the sound helper holds for one channel: 32 MB
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
    /// To the sound helper.
    Sound(ToSound),
}

/// What of one channel's sound reached the sound helper.
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

    /// The user muted or unmuted `channel`'s tab: what to tell the sound
    /// helper, if it changed.
    pub fn mute(&mut self, channel: u32, muted: bool) -> Option<Send> {
        let heard = self.channels.entry(channel).or_default();
        if heard.muted == muted {
            return None;
        }
        heard.muted = muted;
        Some(Send::Sound(ToSound::Audio { channel, audio: Audio::Mute(muted) }))
    }

    /// A voice operation from the decoder of `channel`: what to pass on, if
    /// anything. An operation on a sound never defined, beyond its length,
    /// or beyond the ledger is dropped.
    pub fn decoded(&mut self, channel: u32, audio: Audio) -> Option<ToSound> {
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
        Some(ToSound::Audio { channel, audio })
    }

    /// `channel` has no decoder any more (its tab closed or moved on, or
    /// the decoder ended): its sounds go.
    pub fn channel_gone(&mut self, channel: u32) -> Vec<Send> {
        let Some(heard) = self.channels.remove(&channel) else { return Vec::new() };
        self.held = self.held.saturating_sub(heard.held);
        vec![Send::Sound(ToSound::Audio { channel, audio: Audio::Reset })]
    }
}

#[cfg(test)]
mod tests;

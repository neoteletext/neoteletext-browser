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
/// What a stream is charged in the ledger: two seconds at the highest
/// rate, above the 1.6-second ring the interface's mixer keeps of one, and
/// what the engine says a sink holds of a stream (its `STREAM_HELD_MS`).
pub const STREAM_HELD_SAMPLES: u32 = 48_000 * 2;
const _: () = assert!(STREAM_HELD_SAMPLES as u64 == 48_000 * neoteletext_audio::STREAM_HELD_MS / 1_000);

/// What the sound helper holds under one name: a stored sound, or a stream
/// of a sound on one audio channel (the engine's INT-37), so one sound may
/// stream on two audio channels at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Held {
    /// A sound defined whole.
    Stored(Sound),
    /// The stream of a sound on an audio channel.
    Stream(u8, Sound),
}

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
    /// Each defined sound and each stream, and what it is charged.
    sounds: BTreeMap<Held, u32>,
    held: u64,
    muted: bool,
    /// The audio channels a local sound was started on and not stopped or
    /// replaced since (page-rules.md RULE-60).
    local: [bool; neoteletext_audio::CHANNELS as usize],
    /// The presentation tick the sound commands were last counted in, and
    /// how many passed in it (page-rules.md RULE-52 item 5).
    commands_tick: u32,
    commands: u32,
    /// Sound commands dropped by the budget since last taken (fault 10).
    commands_dropped: u32,
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
    pub fn decoded(&mut self, channel: u32, audio: Audio, now_us: u64) -> Option<ToSound> {
        let heard = self.channels.entry(channel).or_default();
        // RULE-52 item 5: at most `SOUND_COMMAND_BUDGET` sound commands a
        // tick from one view's decoder, however they arose, the rest
        // dropped and counted. A general protection of this core and the
        // sound helper, not a cap on a group (page-rules.md section 12.4).
        if matches!(audio, Audio::Start { .. } | Audio::Stop { .. } | Audio::Gain { .. } | Audio::Local { .. } | Audio::LocalMidi { .. }) {
            let tick = neoteletext_core::consts::presentation_tick_at_us(now_us);
            if tick != heard.commands_tick {
                heard.commands_tick = tick;
                heard.commands = 0;
            }
            if heard.commands >= protocol::rules::SOUND_COMMAND_BUDGET {
                heard.commands_dropped = heard.commands_dropped.saturating_add(1);
                return None;
            }
            heard.commands = heard.commands.saturating_add(1);
        }
        let charge = |heard: &mut Heard, total: &mut u64, name: Held, samples: u32| -> Option<()> {
            let previous = u64::from(heard.sounds.get(&name).copied().unwrap_or(0));
            let held = heard.held.saturating_sub(previous).saturating_add(u64::from(samples));
            let all = total.saturating_sub(previous).saturating_add(u64::from(samples));
            if held > MAX_HELD_SAMPLES || all > MAX_TOTAL_HELD_SAMPLES {
                return None;
            }
            heard.held = held;
            *total = all;
            heard.sounds.insert(name, samples);
            Some(())
        };
        let release = |heard: &mut Heard, total: &mut u64, name: Held| -> Option<()> {
            let length = u64::from(heard.sounds.remove(&name)?);
            heard.held = heard.held.saturating_sub(length);
            *total = total.saturating_sub(length);
            Some(())
        };
        match &audio {
            Audio::Define { sound, samples, .. } => charge(heard, &mut self.held, Held::Stored(*sound), *samples)?,
            Audio::Stream { channel: voice, sound, .. } => charge(heard, &mut self.held, Held::Stream(*voice, *sound), STREAM_HELD_SAMPLES)?,
            Audio::Samples { sound, first, pcm } => {
                let length = heard.sounds.get(&Held::Stored(*sound))?;
                let end = u64::from(*first).saturating_add(u64::try_from(pcm.len()).unwrap_or(u64::MAX));
                if end > u64::from(*length) {
                    return None;
                }
            }
            Audio::StreamSamples { channel: voice, sound, .. } => {
                heard.sounds.get(&Held::Stream(*voice, *sound))?;
            }
            Audio::EndStream { channel: voice, sound } => release(heard, &mut self.held, Held::Stream(*voice, *sound))?,
            // The sound and every stream of it, on any audio channel.
            Audio::Forget(sound) => {
                let names: Vec<Held> = heard
                    .sounds
                    .keys()
                    .copied()
                    .filter(|name| match name {
                        Held::Stored(held) | Held::Stream(_, held) => held == sound,
                    })
                    .collect();
                if names.is_empty() {
                    return None;
                }
                for name in names {
                    release(heard, &mut self.held, name)?;
                }
            }
            // A start plays the stream of its sound on its audio channel, or
            // the sound stored whole.
            Audio::Start { channel: voice, sound, .. } => {
                if !heard.sounds.contains_key(&Held::Stream(*voice, *sound)) && !heard.sounds.contains_key(&Held::Stored(*sound)) {
                    return None;
                }
                // It replaces a local sound on that audio channel.
                if let Some(local) = heard.local.get_mut(usize::from(*voice)) {
                    *local = false;
                }
            }
            // A local sound (page-rules.md RULE-63): its fields were
            // bounded as the message was read; the bank image and the
            // setup block are passed on unread, as samples are. At most
            // `MOST_LOCAL_SOUNDS` play at once in a view (RULE-60), and a
            // MIDI message goes only to an audio channel a local sound
            // was started on (RULE-21).
            Audio::LocalBank { .. } | Audio::Gain { .. } => {}
            Audio::Local { channel: voice, .. } => {
                let others = heard.local.iter().zip(0_u8..).filter(|(local, other)| **local && other != voice).count();
                if others >= neoteletext_audio::MOST_LOCAL_SOUNDS {
                    return None;
                }
                *heard.local.get_mut(usize::from(*voice))? = true;
            }
            Audio::LocalMidi { channel: voice, .. } => {
                if !heard.local.get(usize::from(*voice)).copied().unwrap_or(false) {
                    return None;
                }
            }
            Audio::Stop { channel: voice, .. } => {
                if let Some(local) = heard.local.get_mut(usize::from(*voice)) {
                    *local = false;
                }
            }
            // Only the trusted core resets, mutes or gives a watch list;
            // the protocol refuses them from a decoder, and they are
            // refused here too.
            Audio::Reset | Audio::Mute(_) | Audio::Watch(_) | Audio::Pause(_) => return None,
            // A cue mark of a sound the view has (RULE-109): of a stream
            // on its audio channel or a sound stored whole.
            Audio::Cue { channel: voice, sound, .. } => {
                if !heard.sounds.contains_key(&Held::Stream(*voice, *sound)) && !heard.sounds.contains_key(&Held::Stored(*sound)) {
                    return None;
                }
            }
        }
        Some(ToSound::Audio { channel, audio })
    }

    /// The sound commands of `channel`'s decoder the budget dropped since
    /// this was last asked (fault 10, RULE-52 item 5).
    pub fn take_commands_dropped(&mut self, channel: u32) -> u32 {
        self.channels.get_mut(&channel).map_or(0, |heard| core::mem::take(&mut heard.commands_dropped))
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

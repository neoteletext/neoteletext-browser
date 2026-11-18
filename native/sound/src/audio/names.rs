//! The mixer's keys for what a channel's decoder names (the engine's INT-37):
//! a sound stored whole is named by its sound, a stream by its audio channel
//! and its sound together, so one sound streams on two audio channels as two
//! voices. The engine mixer knows a sound by one key alone, so each name is
//! given a key of its own here, and a stream under a name already held gets
//! a new key, leaving the old one's voice to play out what it holds.

use std::collections::{BTreeMap, BTreeSet};

use neoteletext_audio::CHANNELS;
use neoteletext_audio_mix::SoundKey;
use protocol::audio::Sound;

/// The sounds a decoder's audio receiver holds at once: the decoder's
/// `AUDIO_OBJECTS`, every handle of an OBJECT record.
const RECEIVER_OBJECTS: usize = 1 << u8::BITS;
/// The most names one channel holds: each of the receiver's sounds stored
/// whole and streaming on every audio channel. A name beyond it is dropped,
/// so a faulty decoder cannot make the search for a free key long.
pub(super) const MAX_NAMES: usize = RECEIVER_OBJECTS * (1 + CHANNELS as usize);
/// Every key the mixer can hold: a handle of 8 bits and a generation of 16.
const KEY_SPACE: u32 = 1 << (u8::BITS + u16::BITS);
const _: () = assert!(MAX_NAMES < KEY_SPACE as usize);

/// What a decoder names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Name {
    /// A sound defined whole.
    Stored(Sound),
    /// The stream of a sound on an audio channel.
    Stream(u8, Sound),
}

impl Name {
    /// The sound it names.
    fn sound(self) -> Sound {
        match self {
            Self::Stored(sound) | Self::Stream(_, sound) => sound,
        }
    }
}

/// One channel's names and their keys.
#[derive(Debug, Default)]
pub(super) struct Names {
    keys: BTreeMap<Name, SoundKey>,
    used: BTreeSet<SoundKey>,
    /// The next key tried, counting through [`KEY_SPACE`]: a key let go is
    /// given again only after every other has been, by when the voice that
    /// played its sound out has long ended.
    next: u32,
}

impl Names {
    /// The key of `name`, if it is held.
    pub(super) fn get(&self, name: Name) -> Option<SoundKey> {
        self.keys.get(&name).copied()
    }

    /// The key a start of `sound` on audio channel `channel` plays: the
    /// stream of it there, or else the sound stored whole.
    pub(super) fn started(&self, channel: u8, sound: Sound) -> Option<SoundKey> {
        self.get(Name::Stream(channel, sound)).or_else(|| self.get(Name::Stored(sound)))
    }

    /// The key of `name`, kept where it is held, else a new one; `None`
    /// when the channel holds [`MAX_NAMES`] already.
    pub(super) fn keep(&mut self, name: Name) -> Option<SoundKey> {
        self.get(name).or_else(|| self.fresh(name))
    }

    /// A new key for `name`, and the key it held before, if any, which is
    /// let go; `None` when the channel holds [`MAX_NAMES`] already.
    pub(super) fn renew(&mut self, name: Name) -> Option<(SoundKey, Option<SoundKey>)> {
        let old = self.release(name);
        self.fresh(name).map(|key| (key, old))
    }

    /// Lets `name` go: the key it held.
    pub(super) fn release(&mut self, name: Name) -> Option<SoundKey> {
        let key = self.keys.remove(&name)?;
        self.used.remove(&key);
        Some(key)
    }

    /// Lets go of `sound` stored whole and every stream of it, on every
    /// audio channel: the keys they held.
    pub(super) fn release_sound(&mut self, sound: Sound) -> Vec<SoundKey> {
        let names: Vec<Name> = self.keys.keys().copied().filter(|name| name.sound() == sound).collect();
        names.into_iter().filter_map(|name| self.release(name)).collect()
    }

    fn fresh(&mut self, name: Name) -> Option<SoundKey> {
        if self.keys.len() >= MAX_NAMES {
            return None;
        }
        // Fewer than MAX_NAMES keys are used, so one of the next
        // MAX_NAMES + 1 is free.
        for _ in 0..=MAX_NAMES {
            let candidate = key_of(self.next);
            self.next = self.next.wrapping_add(1) % KEY_SPACE;
            if self.used.insert(candidate) {
                self.keys.insert(name, candidate);
                return Some(candidate);
            }
        }
        None
    }
}

/// The key numbered `index`, its handle the high 8 bits.
fn key_of(index: u32) -> SoundKey {
    let [_, handle, high, low] = index.to_be_bytes();
    (handle, u16::from_be_bytes([high, low]))
}

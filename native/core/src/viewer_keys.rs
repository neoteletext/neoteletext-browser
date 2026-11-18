//! The trusted core's viewer keys (neoteletext-common's
//! `docs/viewer-state.md` VS-101 to VS-113, DEC-6471, DEC-6478): one
//! Ed25519 key pair (RFC 8032) for each channel identity in each jar, made
//! from the system's random source only when the channel asks and the
//! viewer has just acted, held in memory as a visit key, and kept, sealed,
//! only when the viewer chooses to be remembered. No other process holds,
//! receives or sends a private key or seed: a fetcher is given a signature
//! over bytes the core built itself (VS-103).
//!
//! Nothing here implements `Debug` or `Display`, so no key, seed or
//! signature can reach a log line by a format string (VS-209).

use std::collections::{BTreeMap, BTreeSet};

use neoteletext_web::viewer_state::{
    Algorithm, CHALLENGE_BYTES, ChannelIdentity, KeyId, Lifetime, MAX_SIGNED_BYTES, NoneReason, PUBLIC_KEY_BYTES, SIGNATURE_BYTES, key_id, signed_bytes,
};
use ring::signature::{Ed25519KeyPair, KeyPair as _};

pub mod file;

/// An Ed25519 seed (RFC 8032 section 5.1.5): the private key.
pub const SEED_BYTES: usize = 32;
const SECONDS_PER_HOUR: u64 = 60 * 60;
const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;
/// VS-106 and open question Q2: a visit key unused for 12 hours goes.
pub const VISIT_KEY_IDLE_LIMIT: u64 = 12 * SECONDS_PER_HOUR;
/// VS-106 and open question Q2: a visit key goes 7 days after it was made.
pub const VISIT_KEY_ABSOLUTE_LIMIT: u64 = 7 * SECONDS_PER_DAY;
/// VS-107 and open question Q2: a remembered key unused for 180 days goes.
pub const REMEMBERED_KEY_IDLE_LIMIT: u64 = 180 * SECONDS_PER_DAY;
/// The days a remembered key is kept at most: RFC 6265bis's cap on a
/// cookie's life (VS-107, open question Q2).
pub const REMEMBERED_DAYS: u32 = 400;
/// VS-107: a remembered key goes 400 days after it was first remembered.
pub const REMEMBERED_KEY_ABSOLUTE_LIMIT: u64 = REMEMBERED_DAYS as u64 * SECONDS_PER_DAY;
// The strip's question says the same days.
const _: () = assert!(REMEMBERED_DAYS == interface_core::browser::REMEMBERED_DAYS, "the days the strip says");
/// Section 10.3 row 14: keys held in one jar at most, the cookie total's
/// bound. Beyond it the key used longest ago goes.
pub const MOST_KEYS_PER_JAR: usize = 3_000;
/// VS-112: the channels blocked at most, the cookie block list's bound.
pub const MOST_BLOCKED_CHANNELS: usize = 1_000;

/// What a channel knows of this browser.
pub use interface_core::viewer_state::Known;

/// One key: its pair, its seed for the key file, and its times in seconds
/// since 1970.
struct Key {
    seed: [u8; SEED_BYTES],
    pair: Ed25519KeyPair,
    created: u64,
    /// When it was first remembered; none for a visit key.
    remembered: Option<u64>,
    used: u64,
}

impl Key {
    /// The key a seed makes; none for a seed `ring` refuses, which no seed
    /// of 32 bytes is.
    fn of(seed: [u8; SEED_BYTES], created: u64, remembered: Option<u64>, used: u64) -> Option<Self> {
        let pair = Ed25519KeyPair::from_seed_unchecked(&seed).ok()?;
        Some(Self { seed, pair, created, remembered, used })
    }

    /// A new key from the system's cryptographic random source (VS-101).
    fn made(now: u64) -> Option<Self> {
        use ring::rand::SecureRandom as _;
        let mut seed = [0_u8; SEED_BYTES];
        ring::rand::SystemRandom::new().fill(&mut seed).ok()?;
        Self::of(seed, now, None, now)
    }

    fn public_key(&self) -> [u8; PUBLIC_KEY_BYTES] {
        self.pair.public_key().as_ref().try_into().unwrap_or([0; PUBLIC_KEY_BYTES])
    }

    const fn lifetime(&self) -> Lifetime {
        if self.remembered.is_some() { Lifetime::Remembered } else { Lifetime::Visit }
    }

    /// Whether the key has outlived its limits at `now` (VS-106, VS-107).
    fn expired(&self, now: u64) -> bool {
        let idle = now.saturating_sub(self.used);
        match self.remembered {
            None => idle >= VISIT_KEY_IDLE_LIMIT || now.saturating_sub(self.created) >= VISIT_KEY_ABSOLUTE_LIMIT,
            Some(remembered) => idle >= REMEMBERED_KEY_IDLE_LIMIT || now.saturating_sub(remembered) >= REMEMBERED_KEY_ABSOLUTE_LIMIT,
        }
    }
}

/// The trusted core's answer to one challenge (VS-204).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// A proof: the key's lifetime, its public key and the signature over
    /// the bytes of VS-203.
    Proof {
        /// Visit or remembered.
        lifetime: Lifetime,
        /// The public key.
        public_key: [u8; PUBLIC_KEY_BYTES],
        /// The signature.
        signature: [u8; SIGNATURE_BYTES],
    },
    /// No proof, and why.
    None(NoneReason),
}

/// What the core knows of a challenge besides its bytes: the core alone
/// sees the viewer's input (VS-104), and the core alone takes the channel
/// from the tab (VS-103).
#[derive(Clone, Copy)]
pub struct Asked<'a> {
    /// The tab is private: the private jar.
    pub private: bool,
    /// The channel, from the tab's address.
    pub identity: &'a ChannelIdentity,
    /// The server's challenge.
    pub challenge: &'a [u8; CHALLENGE_BYTES],
    /// The channel asks for a key to be made.
    pub create: bool,
    /// The viewer acted on the tab within `STATE_GESTURE_MS` (VS-212).
    pub gesture: bool,
}

/// One channel in the list "Channels that know you" (VS-112): never the
/// key or any part of it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Listed {
    /// When its key was made, in seconds since 1970.
    pub created: u64,
    /// Whether it is remembered.
    pub remembered: bool,
    /// When it was last used.
    pub used: u64,
}

/// Every key of both jars, and the viewer's blocks.
#[derive(Default)]
pub struct Store {
    ordinary: BTreeMap<String, Key>,
    /// Private windows' keys: memory only, never written, never listed
    /// with the ordinary ones (VS-110).
    private: BTreeMap<String, Key>,
    /// Channels the viewer blocked (VS-112).
    pub blocked: BTreeSet<String>,
    /// "Channels may ask to know you: Never" (VS-112, open question Q16).
    pub never: bool,
    /// The remembered keys changed since they were last written.
    changed: bool,
}

impl Store {
    fn jar(&mut self, private: bool) -> &mut BTreeMap<String, Key> {
        if private { &mut self.private } else { &mut self.ordinary }
    }

    /// Deletes every key past its limits (VS-106, VS-107); returns the
    /// ordinary jar's channels whose keys went.
    pub fn expire(&mut self, now: u64) -> Vec<String> {
        let expired: Vec<String> = self.ordinary.iter().filter(|(_, key)| key.expired(now)).map(|(identity, _)| identity.clone()).collect();
        for identity in &expired {
            if self.ordinary.remove(identity).is_some_and(|key| key.remembered.is_some()) {
                self.changed = true;
            }
        }
        self.private.retain(|_, key| !key.expired(now));
        expired
    }

    /// The answer to a challenge (VS-104, VS-203, VS-204): a proof with the
    /// channel's key; else `STATE_NONE` `no_key` when the channel did not ask
    /// to create; `declined` when the viewer blocked it or every channel;
    /// `no_gesture` without the viewer's recent input; else a new visit key
    /// and its proof. The same path is taken whether or not a key exists,
    /// up to the lookup (section 10.3 row 15).
    pub fn answer(&mut self, asked: &Asked<'_>, now: u64) -> Answer {
        self.expire(now);
        let channel = asked.identity.as_str().to_owned();
        let blocked = self.never || self.blocked.contains(&channel);
        let jar = self.jar(asked.private);
        if !jar.contains_key(&channel) {
            let refusal = if !asked.create {
                Some(NoneReason::NoKey)
            } else if blocked {
                Some(NoneReason::Declined)
            } else if !asked.gesture {
                Some(NoneReason::NoGesture)
            } else {
                None
            };
            if let Some(reason) = refusal {
                return Answer::None(reason);
            }
            let Some(key) = Key::made(now) else { return Answer::None(NoneReason::NoKey) };
            if jar.len() >= MOST_KEYS_PER_JAR
                && let Some(oldest) = jar.iter().min_by_key(|(_, key)| key.used).map(|(identity, _)| identity.clone())
            {
                jar.remove(&oldest);
            }
            jar.insert(channel.clone(), key);
        }
        let Some(key) = jar.get_mut(&channel) else { return Answer::None(NoneReason::NoKey) };
        key.used = now;
        let lifetime = key.lifetime();
        let mut bytes = [0_u8; MAX_SIGNED_BYTES];
        let Ok(length) = signed_bytes(Algorithm::Ed25519, lifetime, asked.challenge, asked.identity, &mut bytes) else {
            return Answer::None(NoneReason::NoKey);
        };
        let Some(signed) = bytes.get(..length) else { return Answer::None(NoneReason::NoKey) };
        let signature: [u8; SIGNATURE_BYTES] = key.pair.sign(signed).as_ref().try_into().unwrap_or([0; SIGNATURE_BYTES]);
        let public_key = key.public_key();
        if lifetime == Lifetime::Remembered {
            self.changed = true;
        }
        Answer::Proof { lifetime, public_key, signature }
    }

    /// What `channel` knows of this browser in a jar.
    #[must_use]
    pub fn known(&self, private: bool, channel: &str) -> Known {
        let jar = if private { &self.private } else { &self.ordinary };
        match jar.get(channel).map(|key| key.remembered.is_some()) {
            None => Known::Not,
            Some(false) => Known::Visit,
            Some(true) => Known::Remembered,
        }
    }

    /// Remembers the ordinary jar's key of `channel` (VS-109); false when
    /// it has none.
    pub fn remember(&mut self, channel: &str, now: u64) -> bool {
        let Some(key) = self.ordinary.get_mut(channel) else { return false };
        if key.remembered.is_none() {
            key.remembered = Some(now);
            key.used = now;
            self.changed = true;
        }
        true
    }

    /// Makes the ordinary jar's key of `channel` a visit key again: kept
    /// until the browser closes or its visit limits, never written.
    pub fn stop_remembering(&mut self, channel: &str, now: u64) -> bool {
        let Some(key) = self.ordinary.get_mut(channel) else { return false };
        if key.remembered.take().is_some() {
            // Its visit limits count from now: stopping is not forgetting.
            key.created = now;
            key.used = now;
            self.changed = true;
        }
        true
    }

    /// Deletes the key of `channel` in a jar (VS-113); whether it had one.
    pub fn forget(&mut self, private: bool, channel: &str) -> bool {
        let removed = self.jar(private).remove(channel);
        if removed.as_ref().is_some_and(|key| key.remembered.is_some()) {
            self.changed = true;
        }
        removed.is_some()
    }

    /// Deletes every ordinary key; returns their channels.
    pub fn forget_all(&mut self) -> Vec<String> {
        let gone: Vec<String> = self.ordinary.keys().cloned().collect();
        if self.ordinary.values().any(|key| key.remembered.is_some()) {
            self.changed = true;
        }
        self.ordinary.clear();
        gone
    }

    /// Deletes the ordinary keys made at `from` or later, in seconds since
    /// 1970, with history (VS-112); returns their channels.
    pub fn forget_made_since(&mut self, from: u64) -> Vec<String> {
        let gone: Vec<String> = self.ordinary.iter().filter(|(_, key)| key.created >= from).map(|(identity, _)| identity.clone()).collect();
        for identity in &gone {
            self.forget(false, identity);
        }
        gone
    }

    /// Blocks `channel`, forgetting its keys, or unblocks it (VS-112).
    /// Returns whether an ordinary key went.
    pub fn block(&mut self, channel: &str, blocked: bool) -> bool {
        if !blocked {
            self.blocked.remove(channel);
            return false;
        }
        if self.blocked.len() < MOST_BLOCKED_CHANNELS {
            self.blocked.insert(channel.to_owned());
        }
        self.forget(true, channel);
        self.forget(false, channel)
    }

    /// `STATE_END` (VS-204): the key of `channel` in a jar is deleted only
    /// when its identifier is `ended`, so a late end never deletes a newer
    /// key. Returns whether it was.
    pub fn end(&mut self, private: bool, channel: &str, ended: &KeyId) -> bool {
        let matches = self.jar(private).get(channel).is_some_and(|key| key_id(Algorithm::Ed25519, &key.public_key()) == *ended);
        matches && self.forget(private, channel)
    }

    /// The last private window closed: its keys go (VS-110).
    pub fn close_private(&mut self) {
        self.private.clear();
    }

    /// The ordinary jar's channels with their keys' facts, by identity
    /// (VS-112).
    #[must_use]
    pub fn listed(&self) -> Vec<(String, Listed)> {
        self.ordinary
            .iter()
            .map(|(identity, key)| (identity.clone(), Listed { created: key.created, remembered: key.remembered.is_some(), used: key.used }))
            .collect()
    }

    /// Whether the remembered keys changed since they were written.
    #[must_use]
    pub const fn changed(&self) -> bool {
        self.changed
    }

    /// The remembered keys were written.
    pub const fn written(&mut self) {
        self.changed = false;
    }
}

#[cfg(test)]
mod tests;

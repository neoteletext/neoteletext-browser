//! The terminal channel's podcast (page 700): an episode read from a WAVE
//! file and streamed as the engine's live sound (its REQ-AUD-014), paced
//! in real time a little ahead of the listener, with play, pause and
//! skipping on the viewer's keys.
//!
//! Each start is a new generation of the live sound: the receiver joins a
//! live sound at its newest block, so a play that follows a pause or a skip
//! begins a fresh stream rather than resuming one whose blocks stopped.

use std::time::Instant;

use neoteletext_audio::encode::{Command, Sound, packets};
use neoteletext_audio::{Codec, IMA_BLOCK_LARGE};

mod wave;

pub use wave::read;

/// The podcast's handle in the audio profile, after the channel's other
/// sounds, and its channel: the last of the four, which the other sounds
/// leave to it.
pub const HANDLE: u8 = 15;
pub const CHANNEL: u8 = 3;
const GAIN: u8 = 90;
const FADE_MS: u16 = 20;
/// The operations of a CONTROL record (the engine's REQ-AUD-006).
const PLAY: u8 = 1;
const STOP: u8 = 2;
/// Blocks are sent this far ahead of the sender's clock, in milliseconds:
/// more than the mixer's buffer of a live sound (`STREAM_TARGET_MS`, 400)
/// plus a frame of the channel's loop (100), well inside the receiver's
/// ring of 32 blocks and the mixer's of 1600 ms.
const LEAD_MS: u64 = 600;
const MILLISECONDS_PER_SECOND: u64 = 1_000;
/// A skip, in seconds.
pub const SKIP_SECONDS: i64 = 10;

/// An episode: its samples, mono, at a rate the profile carries.
pub struct Episode {
    /// What the page calls it: the file's name.
    pub title: String,
    /// Samples a second.
    pub rate: u32,
    /// The samples.
    pub samples: Vec<i16>,
}

impl Episode {
    /// Whole seconds in `samples` samples.
    pub fn seconds(&self, samples: usize) -> u64 {
        u64::try_from(samples).unwrap_or(u64::MAX) / u64::from(self.rate.max(1))
    }
}

/// A listener's place in the episode.
pub struct Player {
    /// The live sound now streaming, if one has started.
    sound: Option<Sound>,
    generation: u16,
    /// Samples sent so far; the next block starts here.
    sent: usize,
    /// When playing: when it started, and where it was then.
    playing: Option<(Instant, usize)>,
    /// The live sound's next block number.
    next_block: u16,
    /// The last command's number on the podcast's channel.
    sequence: u16,
}

impl Player {
    /// A player at the start, paused.
    pub fn new() -> Self {
        Self { sound: None, generation: 0, sent: 0, playing: None, next_block: 0, sequence: 0 }
    }

    /// Whether it is playing.
    pub fn playing(&self) -> bool {
        self.playing.is_some()
    }

    /// Where the listener is, in samples, by the sender's clock.
    pub fn position(&self, episode: &Episode) -> usize {
        let at = match self.playing {
            Some((started, at)) => {
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let heard = elapsed.saturating_mul(u64::from(episode.rate)) / MILLISECONDS_PER_SECOND;
                at.saturating_add(usize::try_from(heard).unwrap_or(usize::MAX)).min(self.sent)
            }
            None => self.sent,
        };
        at.min(episode.samples.len())
    }

    fn command(&mut self, operation: u8) -> Vec<u8> {
        self.sequence = self.sequence.wrapping_add(1);
        Command {
            channel: CHANNEL,
            sequence: self.sequence,
            operation,
            handle: HANDLE,
            generation: self.generation,
            gain: GAIN,
            fade_ms: FADE_MS,
            flags: 0,
            duck: 0,
            page: None,
        }
        .record()
    }

    /// Starts a new stream from `self.sent`: the sound announced, then the
    /// play, then the first blocks, so the receiver joins at the first.
    fn start(&mut self, episode: &Episode) -> Vec<Vec<u8>> {
        self.generation = self.generation.wrapping_add(1);
        let Ok(sound) = Sound::live(HANDLE, self.generation, Codec::ImaAdpcm, episode.rate, IMA_BLOCK_LARGE) else { return Vec::new() };
        let Ok(object) = sound.object_record() else { return Vec::new() };
        self.sound = Some(sound);
        self.next_block = 0;
        self.playing = Some((Instant::now(), self.sent));
        let mut records = vec![object, self.command(PLAY)];
        records.extend(self.due(episode));
        records
    }

    fn stop(&mut self, episode: &Episode) -> Vec<Vec<u8>> {
        if self.playing.is_none() {
            return Vec::new();
        }
        self.sent = self.position(episode);
        self.playing = None;
        vec![self.command(STOP)]
    }

    /// Play from where it is, or pause there.
    pub fn toggle(&mut self, episode: &Episode, record_id: &mut u8) -> Vec<u8> {
        let records = if self.playing() {
            self.stop(episode)
        } else {
            if self.sent >= episode.samples.len() {
                self.sent = 0;
            }
            self.start(episode)
        };
        encode(&records, record_id)
    }

    /// Skips forward or back by `seconds`, playing on if it was playing.
    pub fn skip(&mut self, episode: &Episode, seconds: i64, record_id: &mut u8) -> Vec<u8> {
        let was_playing = self.playing();
        let at = i64::try_from(self.position(episode)).unwrap_or(i64::MAX);
        let mut records = self.stop(episode);
        let end = i64::try_from(episode.samples.len()).unwrap_or(i64::MAX);
        let to = at.saturating_add(seconds.saturating_mul(i64::from(episode.rate))).clamp(0, end);
        self.sent = usize::try_from(to).unwrap_or(0);
        if was_playing && self.sent < episode.samples.len() {
            records.extend(self.start(episode));
        }
        encode(&records, record_id)
    }

    /// Back to the start, and play.
    pub fn restart(&mut self, episode: &Episode, record_id: &mut u8) -> Vec<u8> {
        let mut records = self.stop(episode);
        self.sent = 0;
        records.extend(self.start(episode));
        encode(&records, record_id)
    }

    /// The blocks now due, as records.
    fn due(&mut self, episode: &Episode) -> Vec<Vec<u8>> {
        let (Some((started, at)), Some(sound)) = (self.playing, &self.sound) else { return Vec::new() };
        let ahead = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX).saturating_add(LEAD_MS);
        let due = at.saturating_add(usize::try_from(ahead.saturating_mul(u64::from(episode.rate)) / MILLISECONDS_PER_SECOND).unwrap_or(usize::MAX));
        let block = usize::try_from(sound.samples_per_block()).unwrap_or(1).max(1);
        let mut records = Vec::new();
        // Whole blocks only: a block is sent once all of it is due, and the
        // last, part one is padded with silence by `live_records`.
        while self.sent < episode.samples.len() && self.sent.saturating_add(block) <= due {
            let end = self.sent.saturating_add(block).min(episode.samples.len());
            records.extend(sound.live_records(self.next_block, episode.samples.get(self.sent..end).unwrap_or_default()));
            self.next_block = self.next_block.wrapping_add(1);
            self.sent = end;
        }
        records
    }

    /// What is due now: the next blocks, and at the episode's end a stop.
    pub fn stream(&mut self, episode: &Episode, record_id: &mut u8) -> Vec<u8> {
        let mut records = self.due(episode);
        if self.playing() && self.position(episode) >= episode.samples.len() {
            records.extend(self.stop(episode));
        }
        encode(&records, record_id)
    }
}

/// Records as the audio profile's packets, numbered on from `record_id`.
fn encode(records: &[Vec<u8>], record_id: &mut u8) -> Vec<u8> {
    if records.is_empty() {
        return Vec::new();
    }
    let (packets, next) = packets(crate::terminal::AUDIO_SERVICE, *record_id, records);
    *record_id = next;
    packets.iter().flat_map(|packet| packet.bytes).collect()
}

#[cfg(test)]
mod tests;

//! The terminal channel's podcast (page 700): an episode read from a WAVE
//! file and streamed as the engine's live sound (its REQ-AUD-014), paced
//! in real time a little ahead of the listener, with play, pause and
//! skipping on the viewer's keys.
//!
//! Each start is a new generation of the live sound: the receiver joins a
//! live sound at its newest block, so a play that follows a pause or a skip
//! begins a fresh stream rather than resuming one whose blocks stopped.

use std::path::Path;
use std::time::Instant;

use neoteletext_audio::encode::{Command, Sound, packets};
use neoteletext_audio::{Codec, IMA_BLOCK_LARGE, rate_code};

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
/// The largest file read: an hour of 48 kHz stereo is about 700 MB.
const MAX_FILE_BYTES: u64 = 1 << 30;
// The WAVE format: a RIFF header of 12 bytes, chunks of an 8-byte header,
// and a `fmt ` chunk of at least 16 bytes whose tag 1 is PCM.
const RIFF_HEADER: usize = 12;
const CHUNK_HEADER: usize = 8;
const FMT_BYTES: usize = 16;
const FMT_CHANNELS: usize = 2;
const FMT_RATE: usize = 4;
const FMT_BITS: usize = 14;
const WAVE_PCM: u16 = 1;
const BITS_16: u16 = 16;
const BYTES_PER_SAMPLE: usize = 2;

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

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// Reads a WAVE file of 16-bit PCM at one of the profile's rates; stereo
/// is mixed to mono.
///
/// # Errors
/// A file that cannot be read, is too large, or is not such a file.
pub fn read(path: &Path) -> Result<Episode, String> {
    let bad = |why: &str| format!("{}: {why}", path.display());
    let size = std::fs::metadata(path).map_err(|error| bad(&error.to_string()))?.len();
    if size > MAX_FILE_BYTES {
        return Err(bad("larger than a gigabyte"));
    }
    let bytes = std::fs::read(path).map_err(|error| bad(&error.to_string()))?;
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..RIFF_HEADER) != Some(b"WAVE") {
        return Err(bad("not a WAVE file"));
    }
    let (mut at, mut format, mut data) = (RIFF_HEADER, None, None);
    while let (Some(id), Some(length)) = (bytes.get(at..at.saturating_add(4)), u32_at(&bytes, at.saturating_add(4))) {
        let body = at.saturating_add(CHUNK_HEADER);
        let length = usize::try_from(length).unwrap_or(usize::MAX);
        let end = body.saturating_add(length).min(bytes.len());
        match id {
            b"fmt " if length >= FMT_BYTES => {
                let field = |offset: usize| u16_at(&bytes, body.saturating_add(offset));
                format = Some((field(0), field(FMT_CHANNELS), u32_at(&bytes, body.saturating_add(FMT_RATE)), field(FMT_BITS)));
            }
            b"data" => data = bytes.get(body..end),
            _ => {}
        }
        // Chunks are padded to an even length.
        at = end.saturating_add(length % 2);
    }
    let Some((Some(WAVE_PCM), Some(channels @ (1 | 2)), Some(rate), Some(BITS_16))) = format else {
        return Err(bad("not 16-bit PCM, mono or stereo"));
    };
    rate_code(rate).ok_or_else(|| bad("a rate the audio profile does not carry"))?;
    let data = data.ok_or_else(|| bad("no data"))?;
    let samples = data
        .chunks_exact(usize::from(channels) * BYTES_PER_SAMPLE)
        .map(|frame| {
            let sum: i32 = frame.as_chunks::<BYTES_PER_SAMPLE>().0.iter().map(|pair| i32::from(i16::from_le_bytes(*pair))).sum();
            i16::try_from(sum / i32::from(channels)).unwrap_or(0)
        })
        .collect();
    let title = path.file_stem().map_or_else(|| "Episode".to_owned(), |stem| stem.to_string_lossy().into_owned());
    Ok(Episode { title, rate, samples })
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
mod tests {
    use super::*;
    use neoteletext_audio::port::{SoundFormat, SoundId, Start, VoiceSink};
    use neoteletext_audio::receiver::Receiver;

    const RATE: u32 = 8_000;

    fn wave(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|sample| sample.to_le_bytes()).collect();
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&u32::try_from(36 + data.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&WAVE_PCM.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2 * u32::from(channels)).to_le_bytes());
        bytes.extend_from_slice(&(2 * channels).to_le_bytes());
        bytes.extend_from_slice(&BITS_16.to_le_bytes());
        // A chunk the reader passes over, of odd length and so padded.
        bytes.extend_from_slice(b"LIST\x03\0\0\0abc\0");
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&data);
        bytes
    }

    fn episode(seconds: usize) -> Episode {
        Episode { title: "Test".to_owned(), rate: RATE, samples: vec![1_000; usize::try_from(RATE).unwrap() * seconds] }
    }

    /// What the receiver gave the sink: streams begun, samples streamed,
    /// and plays and stops.
    #[derive(Default)]
    struct Heard {
        streams: Vec<(SoundId, u32)>,
        samples: usize,
        starts: Vec<u8>,
        stops: Vec<u8>,
    }

    impl VoiceSink for Heard {
        fn define(&mut self, _: SoundId, _: SoundFormat) {}
        fn samples(&mut self, _: SoundId, _: u32, _: &[i16]) {}
        fn forget(&mut self, _: SoundId) {}
        fn stream(&mut self, id: SoundId, rate_hz: u32) {
            self.streams.push((id, rate_hz));
        }
        fn stream_samples(&mut self, _: SoundId, pcm: &[i16]) {
            self.samples += pcm.len();
        }
        fn start(&mut self, channel: u8, _: Start) {
            self.starts.push(channel);
        }
        fn stop(&mut self, channel: u8, _: u16) {
            self.stops.push(channel);
        }
        fn gain(&mut self, _: u8, _: u8, _: u16) {}
    }

    fn hear(receiver: &mut Receiver<'_, 16, 4>, sink: &mut Heard, bytes: &[u8]) {
        for chunk in bytes.as_chunks::<42>().0 {
            receiver.push_packet(&neoteletext_core::packet::Packet { bytes: *chunk }, sink);
        }
    }

    #[test]
    fn a_wave_file_is_read_and_stereo_mixed_to_mono() {
        let directory = std::env::temp_dir().join(format!("t42-podcast-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("Episode one.wav");
        std::fs::write(&path, wave(16_000, 2, &[100, 300, -100, -300])).unwrap();
        let episode = read(&path).unwrap();
        assert_eq!((episode.title.as_str(), episode.rate, episode.samples), ("Episode one", 16_000, vec![200, -200]));
        std::fs::write(&path, b"RIFF\0\0\0\0WAVE").unwrap();
        assert!(read(&path).is_err(), "no format");
        std::fs::write(&path, wave(12_345, 1, &[0; 10])).unwrap();
        assert!(read(&path).is_err(), "a rate the profile does not carry");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_receiver_hears_the_stream_from_its_first_block() {
        let episode = episode(5);
        let mut player = Player::new();
        let mut record_id = 0;
        let mut arena = vec![0_u8; 1 << 20];
        let mut receiver = Receiver::<16, 4>::new(&mut arena);
        let mut sink = Heard::default();
        let started = player.toggle(&episode, &mut record_id);
        hear(&mut receiver, &mut sink, &started);
        assert_eq!((sink.streams.len(), sink.starts.as_slice()), (1, &[CHANNEL][..]), "one stream, played on the podcast's channel");
        // The whole lead went at once, in whole blocks, and every block of
        // it reached the sink: the play joined at the first.
        let lead = usize::try_from(LEAD_MS * u64::from(RATE) / MILLISECONDS_PER_SECOND).unwrap();
        let block = usize::try_from(Codec::ImaAdpcm.samples_per_block(IMA_BLOCK_LARGE)).unwrap();
        assert!(player.sent <= lead && player.sent + block > lead, "{} of {lead}", player.sent);
        assert_eq!(sink.samples, player.sent);
        let paused = player.toggle(&episode, &mut record_id);
        hear(&mut receiver, &mut sink, &paused);
        assert!(!player.playing());
        assert_eq!(sink.stops, [CHANNEL]);
        // Playing again is a new stream, joined at its first block too.
        let before = sink.samples;
        let resumed = player.toggle(&episode, &mut record_id);
        hear(&mut receiver, &mut sink, &resumed);
        assert_eq!(sink.streams.len(), 2);
        assert!(sink.samples - before >= block, "the new stream's first blocks were heard");
    }

    #[test]
    fn a_skip_while_playing_is_heard_as_a_new_stream() {
        let episode = episode(25);
        let mut player = Player::new();
        let mut record_id = 0;
        let mut arena = vec![0_u8; 1 << 20];
        let mut receiver = Receiver::<16, 4>::new(&mut arena);
        let mut sink = Heard::default();
        let started = player.toggle(&episode, &mut record_id);
        hear(&mut receiver, &mut sink, &started);
        let skipped = player.skip(&episode, SKIP_SECONDS, &mut record_id);
        hear(&mut receiver, &mut sink, &skipped);
        assert_eq!(sink.stops, [CHANNEL]);
        assert_eq!((sink.streams.len(), sink.starts.as_slice()), (2, &[CHANNEL, CHANNEL][..]), "a second stream, played");
    }

    #[test]
    fn skipping_moves_by_ten_seconds_within_the_episode() {
        let episode = episode(25);
        let mut player = Player::new();
        let mut record_id = 0;
        let rate = usize::try_from(RATE).unwrap();
        assert!(player.skip(&episode, SKIP_SECONDS, &mut record_id).is_empty(), "paused: nothing to send");
        assert_eq!(player.position(&episode), 10 * rate);
        player.skip(&episode, SKIP_SECONDS, &mut record_id);
        player.skip(&episode, SKIP_SECONDS, &mut record_id);
        assert_eq!(player.position(&episode), episode.samples.len(), "held at the end");
        player.skip(&episode, -3 * SKIP_SECONDS, &mut record_id);
        assert_eq!(player.position(&episode), 0, "held at the start");
        player.toggle(&episode, &mut record_id);
        let skipped = player.skip(&episode, SKIP_SECONDS, &mut record_id);
        assert!(player.playing() && !skipped.is_empty(), "playing on, from ten seconds");
        assert!(player.sent > 10 * rate);
    }
}

//! The episode read from a WAVE file and streamed as live sound, heard
//! through the engine's own receiver.

use super::*;
use crate::podcast::wave::{BITS_16, WAVE_PCM};
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
    let directory = std::env::temp_dir().join(format!("neoteletext-podcast-{}", std::process::id()));
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

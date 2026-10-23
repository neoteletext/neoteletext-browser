//! The decoder's voice sink: the audio receiver's operations become
//! messages for the trusted core.

use neoteletext_audio::port::{SoundFormat, SoundId, Start, VoiceSink};
use protocol::audio::{Audio, MAX_MESSAGE_SAMPLES, Sound};

/// The voice sink of a decoder: each operation becomes a message for the
/// trusted core.
pub(crate) struct Outbox<'a>(pub(crate) &'a mut Vec<Audio>);

fn sound(id: SoundId) -> Sound {
    Sound { handle: id.handle, generation: id.generation }
}

impl VoiceSink for Outbox<'_> {
    fn define(&mut self, id: SoundId, format: SoundFormat) {
        self.0.push(Audio::Define { sound: sound(id), rate_hz: format.rate_hz, samples: format.samples });
    }

    fn samples(&mut self, id: SoundId, first: u32, pcm: &[i16]) {
        let mut start = first;
        for piece in pcm.chunks(MAX_MESSAGE_SAMPLES) {
            self.0.push(Audio::Samples { sound: sound(id), first: start, pcm: piece.to_vec() });
            start = start.saturating_add(u32::try_from(piece.len()).unwrap_or(u32::MAX));
        }
    }

    fn forget(&mut self, id: SoundId) {
        self.0.push(Audio::Forget(sound(id)));
    }

    fn stream(&mut self, id: SoundId, rate_hz: u32) {
        self.0.push(Audio::Stream { sound: sound(id), rate_hz });
    }

    fn stream_samples(&mut self, id: SoundId, pcm: &[i16]) {
        for piece in pcm.chunks(MAX_MESSAGE_SAMPLES) {
            self.0.push(Audio::StreamSamples { sound: sound(id), pcm: piece.to_vec() });
        }
    }

    fn start(&mut self, channel: u8, start: Start) {
        self.0.push(Audio::Start {
            channel,
            sound: sound(start.sound),
            gain: start.gain,
            fade_ms: start.fade_ms,
            looped: start.looped,
            duck_percent: start.duck_percent,
            after_current: start.after_current,
        });
    }

    fn stop(&mut self, channel: u8, fade_ms: u16) {
        self.0.push(Audio::Stop { channel, fade_ms });
    }

    fn gain(&mut self, channel: u8, gain: u8, fade_ms: u16) {
        self.0.push(Audio::Gain { channel, gain, fade_ms });
    }
}

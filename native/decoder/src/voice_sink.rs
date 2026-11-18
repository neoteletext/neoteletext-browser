//! The decoder's voice sink: the audio receiver's operations become
//! messages for the trusted core. It also keeps what a page's rule table
//! reads of each audio channel (neoteletext-common's `docs/page-rules.md`
//! RULE-27, RULE-30): what plays there, and for a local sound the
//! family's tracker, fed with the sound's setup and then with every `midi`
//! action, so that no question is ever put to the sound helper.

use neoteletext_audio::port::{LocalStart, SoundFormat, SoundId, Start, VoiceSink};
use neoteletext_audio::{CHANNELS, MOST_BANKS};
use neoteletext_audio_instruments::midi::Banks;
use neoteletext_audio_instruments::watch::Watched;
use protocol::audio::{Audio, MAX_MESSAGE_SAMPLES, Sound};

/// What plays on one audio channel, as the sink was last told.
#[derive(Debug, Default)]
pub(crate) struct ChannelSound {
    /// A sound was started and not stopped since.
    playing: bool,
    /// The sound playing, while one is.
    sound: Option<SoundId>,
    /// The channel's gain as last commanded, 0 to 100.
    gain: u8,
    /// The bank images given for the local sound about to start, by index.
    banks: Vec<Vec<u8>>,
    /// The local sound's family, variant and tracker (RULE-30), while a
    /// local sound plays here.
    local: Option<(u8, u8, Box<Watched>)>,
}

/// What a page's rules read of the four audio channels.
#[derive(Debug, Default)]
pub(crate) struct Heard {
    channels: [ChannelSound; CHANNELS as usize],
}

impl Heard {
    fn channel(&mut self, channel: u8) -> Option<&mut ChannelSound> {
        self.channels.get_mut(usize::from(channel))
    }

    /// The sound on `channel` as a rule's condition reads it (RULE-27):
    /// whether one plays, its handle and gain, whether it is local, and a
    /// local sound's family and variant.
    pub(crate) fn state(&self, channel: u8) -> neoteletext_rules::evaluator::SoundState {
        let Some(heard) = self.channels.get(usize::from(channel)) else { return neoteletext_rules::evaluator::SoundState::default() };
        let (family, variant) = heard.local.as_ref().map_or((0, 0), |(family, variant, _)| (*family, *variant));
        neoteletext_rules::evaluator::SoundState {
            playing: heard.playing,
            local: heard.local.is_some(),
            handle: heard.sound.map_or(0, |sound| sound.handle),
            gain: heard.gain,
            family,
            variant,
        }
    }

    /// Whether a local sound plays on `channel` (RULE-21).
    pub(crate) fn is_local(&self, channel: u8) -> bool {
        self.channels.get(usize::from(channel)).is_some_and(|heard| heard.local.is_some())
    }

    /// The audio channels a local sound plays on.
    pub(crate) fn locals(&self) -> impl Iterator<Item = u8> + '_ {
        (0..CHANNELS).filter(|channel| self.is_local(*channel))
    }

    /// A tracked value of the local sound on `channel` (RULE-28 classes 0
    /// to 4, 8 and 9; RULE-30); 0 when none plays there.
    pub(crate) fn synth(&self, channel: u8, part: u8, class: u8, index: u16) -> i32 {
        let Some(heard) = self.channels.get(usize::from(channel)) else { return 0 };
        let Some((_, _, tracker)) = &heard.local else { return 0 };
        let images: Vec<&[u8]> = heard.banks.iter().map(Vec::as_slice).collect();
        tracker.read(class, part, index, &Banks::new(&images))
    }

    /// A rule's `midi` action was given to the local sound on `channel`:
    /// its tracker follows (RULE-30).
    pub(crate) fn midi(&mut self, channel: u8, status: u8, data1: u8, data2: u8) {
        let Some(heard) = self.channel(channel) else { return };
        let images: Vec<&[u8]> = heard.banks.iter().map(Vec::as_slice).collect();
        if let Some((_, _, tracker)) = &mut heard.local {
            tracker.channel_message(status, data1, data2, &Banks::new(&images));
        }
    }

    fn started(&mut self, channel: u8, sound: SoundId, gain: u8) {
        if let Some(heard) = self.channel(channel) {
            *heard = ChannelSound { playing: true, sound: Some(sound), gain, banks: Vec::new(), local: None };
        }
    }
}

/// The voice sink of a decoder: each operation becomes a message for the
/// trusted core, and what plays on each channel is noted.
pub(crate) struct Outbox<'a> {
    sounds: &'a mut Vec<Audio>,
    heard: &'a mut Heard,
}

impl<'a> Outbox<'a> {
    /// The sink that adds each operation's message to `sounds` and notes
    /// what plays in `heard`.
    pub(crate) fn new(sounds: &'a mut Vec<Audio>, heard: &'a mut Heard) -> Self {
        Self { sounds, heard }
    }
}

fn sound(id: SoundId) -> Sound {
    Sound { handle: id.handle, generation: id.generation }
}

impl VoiceSink for Outbox<'_> {
    fn define(&mut self, id: SoundId, format: SoundFormat) {
        self.sounds.push(Audio::Define { sound: sound(id), rate_hz: format.rate_hz, samples: format.samples });
    }

    fn samples(&mut self, id: SoundId, first: u32, pcm: &[i16]) {
        let mut start = first;
        for piece in pcm.chunks(MAX_MESSAGE_SAMPLES) {
            self.sounds.push(Audio::Samples { sound: sound(id), first: start, pcm: piece.to_vec() });
            start = start.saturating_add(u32::try_from(piece.len()).unwrap_or(u32::MAX));
        }
    }

    fn forget(&mut self, id: SoundId) {
        self.sounds.push(Audio::Forget(sound(id)));
    }

    fn stream(&mut self, channel: u8, id: SoundId, rate_hz: u32) {
        self.sounds.push(Audio::Stream { channel, sound: sound(id), rate_hz });
    }

    fn stream_samples(&mut self, channel: u8, id: SoundId, pcm: &[i16]) {
        for piece in pcm.chunks(MAX_MESSAGE_SAMPLES) {
            self.sounds.push(Audio::StreamSamples { channel, sound: sound(id), pcm: piece.to_vec() });
        }
    }

    fn end_stream(&mut self, channel: u8, id: SoundId) {
        self.sounds.push(Audio::EndStream { channel, sound: sound(id) });
    }

    fn start(&mut self, channel: u8, start: Start) {
        self.heard.started(channel, start.sound, start.gain);
        self.sounds.push(Audio::Start {
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
        if let Some(heard) = self.heard.channel(channel) {
            *heard = ChannelSound { gain: heard.gain, ..ChannelSound::default() };
        }
        self.sounds.push(Audio::Stop { channel, fade_ms });
    }

    fn gain(&mut self, channel: u8, gain: u8, fade_ms: u16) {
        if let Some(heard) = self.heard.channel(channel) {
            heard.gain = gain;
        }
        self.sounds.push(Audio::Gain { channel, gain, fade_ms });
    }

    /// The sound helper holds a family's instrument and renders it as it
    /// mixes (RULE-62).
    fn plays_local(&self) -> bool {
        true
    }

    fn local_bank(&mut self, channel: u8, id: SoundId, index: u8, image: &[u8]) {
        if usize::from(index) >= MOST_BANKS || image.is_empty() || image.len() > protocol::audio::MAX_BANK_IMAGE {
            return;
        }
        if let Some(heard) = self.heard.channel(channel) {
            // The banks of a start come in index order, from 0.
            heard.banks.truncate(usize::from(index));
            heard.banks.push(image.to_vec());
        }
        self.sounds.push(Audio::LocalBank { channel, sound: sound(id), index, image: image.to_vec() });
    }

    fn local(&mut self, channel: u8, start: LocalStart<'_>) {
        if start.setup.len() > protocol::audio::MAX_LOCAL_SETUP {
            return;
        }
        if let Some(heard) = self.heard.channel(channel) {
            let banks = core::mem::take(&mut heard.banks);
            let images: Vec<&[u8]> = banks.iter().map(Vec::as_slice).collect();
            // The tracker is fed the setup, as the instrument is (RULE-30).
            let tracker = Watched::new(start.family, start.variant, &Banks::new(&images)).map(|mut tracker| {
                tracker.take_block(start.setup, &Banks::new(&images));
                (start.family, start.variant, Box::new(tracker))
            });
            *heard = ChannelSound { playing: true, sound: Some(start.sound), gain: start.gain, banks, local: tracker };
        }
        self.sounds.push(Audio::Local {
            channel,
            start: protocol::audio::LocalStart {
                sound: sound(start.sound),
                family: start.family,
                variant: start.variant,
                option: start.option,
                split: start.split,
                gain: start.gain,
                fade_ms: start.fade_ms,
                duck_percent: start.duck_percent,
                setup: start.setup.to_vec(),
            },
        });
    }
}

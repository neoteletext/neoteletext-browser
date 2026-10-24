//! What a session sends of sound: the channel's sounds as packets, the
//! command that plays one, and the podcast's blocks.

use neoteletext_audio::Codec;
use neoteletext_audio::encode::{Command, Sound, packets};

use super::super::Session;
use super::super::sounds::{AUDIO_CHANNELS, AUDIO_SERVICE, NOTE_BLOCK, NOTE_FADE_MS, NOTE_GAIN, NOTE_RATE, library};
use super::Push;
use crate::podcast::{Episode, Player};

impl Session {
    /// Every sound of the channel, as packets, one message a sound: the
    /// piano's notes and the game's effects.
    pub(super) fn sounds(&mut self) -> Vec<Push> {
        library()
            .into_iter()
            .filter_map(|(handle, pcm)| {
                let sound = Sound::new(handle, 1, Codec::ImaAdpcm, NOTE_RATE, NOTE_BLOCK, &pcm).ok()?;
                let mut records = vec![sound.object_record().ok()?];
                records.extend(sound.data_records());
                let (packets, next) = packets(AUDIO_SERVICE, self.record_id, &records);
                self.record_id = next;
                Some(Push::Packets(packets.iter().flat_map(|packet| packet.bytes).collect()))
            })
            .collect()
    }

    /// The command that plays a sound, on the next of the effects' channels.
    pub(super) fn play(&mut self, handle: u8) -> Push {
        let channel = self.next_channel;
        self.next_channel = (self.next_channel + 1) % AUDIO_CHANNELS;
        let sequence = &mut self.audio_sequence[usize::from(channel)];
        *sequence = sequence.wrapping_add(1);
        let command = Command {
            channel,
            sequence: *sequence,
            operation: 1,
            handle,
            generation: 1,
            gain: NOTE_GAIN,
            fade_ms: NOTE_FADE_MS,
            flags: 0,
            duck: 0,
            page: None,
        };
        let (packets, next) = packets(AUDIO_SERVICE, self.record_id, &[command.record()]);
        self.record_id = next;
        Push::Packets(packets.iter().flat_map(|packet| packet.bytes).collect())
    }

    /// Does `action` to this viewer's podcast: what it sends, pushed.
    pub(super) fn listen(&mut self, action: impl FnOnce(&mut Player, &Episode, &mut u8) -> Vec<u8>) -> Vec<Push> {
        let Some((episode, player)) = &mut self.podcast else { return Vec::new() };
        let bytes = action(player, episode, &mut self.record_id);
        if bytes.is_empty() { Vec::new() } else { vec![Push::Packets(bytes)] }
    }

    /// The podcast's next blocks, wherever the viewer is, and page 700
    /// again each second and when it stops, while it is open.
    pub(in crate::terminal) fn podcast_frame(&mut self) -> Vec<Push> {
        let mut pushes = self.listen(Player::stream);
        if self.page == 0x700
            && let Some((episode, player)) = &self.podcast
        {
            let now = (episode.seconds(player.position(episode)), player.playing());
            if self.shown.replace(now) != Some(now) {
                pushes.push(Push::Page(self.render().tti()));
            }
        }
        pushes
    }
}

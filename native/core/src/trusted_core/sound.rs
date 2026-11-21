//! The sound helper (decision 55, requirement 16.6): started, told each
//! checked operation and the settings, and which channels it says sound.

use confine::broker::Program;
use neoteletext::outbox::Outbox;
use protocol::ToInterface;
use protocol::console::Level;
use protocol::decoder::ToDecoder;
use protocol::sound::{ToSound, read_from_sound, write_to_sound};

use super::processes::{relay, spawn};
use super::{Core, Event};

impl Core {
    /// Passes a checked voice operation or the settings to the sound
    /// helper; with none, the browser is silent and they go nowhere.
    pub(super) fn tell_sound(&mut self, message: &ToSound) {
        if let Some((_, input)) = self.sound.as_mut() {
            let _ = write_to_sound(input, message);
        }
    }

    /// Whether any view's sound reaches the viewer (page-rules.md RULE-27,
    /// DEC-7143): the sound helper runs, which it does only while it has
    /// an output device, and the setting Sound is on with a volume above 0.
    pub(super) fn sound_plays(&self) -> bool {
        self.sound.is_some() && self.settings.sound && self.settings.volume != 0
    }

    /// Whether `channel`'s sound reaches the viewer: sound plays and the
    /// channel's tab is not muted.
    pub(super) fn sound_heard(&self, channel: u32) -> bool {
        self.sound_plays() && !self.hearing.muted(channel)
    }

    /// Tells `channel`'s decoder whether its sound reaches the viewer, for
    /// what its open page's rules read as playing (RULE-27).
    pub(super) fn tell_sound_heard(&mut self, channel: u32) {
        let heard = self.sound_heard(channel);
        self.tell(channel, ToDecoder::SoundHeard(heard));
    }

    /// Tells every decoder whether its sound reaches the viewer: the sound
    /// helper started or ended, or the settings' sound or volume changed.
    pub(super) fn tell_sound_heard_all(&mut self) {
        let channels: Vec<u32> = self.decoders.keys().copied().collect();
        for channel in channels {
            self.tell_sound_heard(channel);
        }
    }

    /// Which channels sound, as the sound helper said: a sounding tab stays
    /// awake (decision 47) and shows its speaker.
    pub(super) fn sounding(&mut self, channels: &[u32]) -> Result<(), String> {
        self.sleep.set_sounding(channels);
        // Every window marks its own tabs among them.
        let here = self.session.window;
        for window in self.session.window_ids() {
            self.switch_window(window);
            self.send(&ToInterface::Sounding(channels.to_vec()))?;
        }
        self.switch_window(here);
        Ok(())
    }

    /// Starts the sound helper, whose messages arrive as events; when it
    /// cannot start, as on a Linux without the system's sound library, the
    /// browser runs silent (requirement 16.6).
    pub(super) fn start_sound(&mut self) {
        let Ok(broker) = self.broker().cloned() else { return };
        let mut child = match spawn(&broker, Program::Sound, &[], false) {
            Ok(child) => child,
            Err(error) => {
                self.log(Level::Error, None, &format!("no sound: {error}"));
                return;
            }
        };
        let (Some(output), Some(input)) = (child.stdout.take(), child.stdin.take()) else { return };
        let events = self.events.clone();
        relay(output, read_from_sound, events, Event::Sound, Event::SoundEnded);
        self.note(Level::Info, None, &format!("sound helper: confined: {}", child.report));
        self.sound = Some((child, Outbox::new(input)));
        let message = ToSound::Settings { sound: self.settings.sound, volume: self.settings.volume };
        self.tell_sound(&message);
        self.tell_sound_heard_all();
    }
}

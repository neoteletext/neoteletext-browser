//! The sound helper (decision 55, requirement 16.6): started, told each
//! checked operation and the settings, and which channels it says sound.

use std::io::BufReader;
use std::thread;

use confine::broker::Program;
use neoteletext::outbox::Outbox;
use protocol::ToInterface;
use protocol::console::Level;
use protocol::sound::{ToSound, read_from_sound, write_to_sound};

use super::processes::spawn;
use super::{Core, Event};

impl Core {
    /// Passes a checked voice operation or the settings to the sound
    /// helper; with none, the browser is silent and they go nowhere.
    pub(super) fn tell_sound(&mut self, message: &ToSound) {
        if let Some((_, input)) = self.sound.as_mut() {
            let _ = write_to_sound(input, message);
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
        thread::spawn(move || {
            let mut output = BufReader::new(output);
            let end = loop {
                match read_from_sound(&mut output) {
                    Ok(Some(message)) => {
                        if events.send(Event::Sound(message)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => break None,
                    Err(error) => break Some(error.to_string()),
                }
            };
            let _ = events.send(Event::SoundEnded(end));
        });
        self.note(Level::Info, None, &format!("sound helper: confined: {}", child.report));
        self.sound = Some((child, Outbox::new(input)));
        let message = ToSound::Settings { sound: self.settings.sound, volume: self.settings.volume };
        self.tell_sound(&message);
    }
}

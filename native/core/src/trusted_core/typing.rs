//! Typing into a page (`interaction.md`, DOC-1781): the keyboard's intents
//! for the open page's channel, and what the typing rules decided,
//! delivered.

use protocol::{ToCore, ToInterface};

use super::Core;

impl Core {
    /// PICK for a pointer event on a page drawn in ReGIS: the numbered
    /// drawing under the pointer, from the pick record of the picture the
    /// interface was last sent. Nothing for any other page or event.
    #[cfg(feature = "interaction")]
    fn pick_under(&mut self, input: &protocol::interaction::InputIntent) -> Vec<neoteletext::typing::Send> {
        use protocol::interaction::Intent;
        let (Intent::Mouse { at, .. } | Intent::Touch { at, .. } | Intent::Wheel { at, .. }) = &input.intent else { return Vec::new() };
        // The place is the picture's own pixel, as the interface said it
        // before this event (REQ-6377), not the event's dot of the page image.
        let _ = at;
        let (Some(pixel), Some(on)) = (self.regis_pointing.at(input.key), self.typing.on_screen(input.key.channel)) else { return Vec::new() };
        let Some(under) = self.regis.under(input.key, on.sub, pixel) else { return Vec::new() };
        let pick = neoteletext_input::event::Pick {
            on,
            drawing: under.drawing,
            x: under.x,
            y: under.y,
            picture_x: under.picture_x,
            picture_y: under.picture_y,
            commits: under.commits,
        };
        self.typing.pick(input.key, pick, self.now_us())
    }

    /// The keyboard's intents: input for the open page's channel, and the
    /// phone keyboard's panel.
    #[cfg(feature = "interaction")]
    pub(super) fn input_intent(&mut self, intent: ToCore) -> Result<(), String> {
        match intent {
            ToCore::Input(intent) => {
                // A HOVER names a control of the page's latest image, or
                // goes nowhere (REQ-INT-121).
                #[cfg(feature = "links")]
                if let protocol::interaction::Intent::Hover { column, row, .. } = &intent.intent
                    && !self.links.is_control_at(intent.key, *column, *row)
                {
                    return Ok(());
                }
                let mut sends = self.typing.input_intent(&intent, self.now_us());
                // On a page drawn in ReGIS, a pointer event that went is
                // followed by what lay under it (PICK, the engine's REQ-6337).
                if sends.iter().any(|send| matches!(send, neoteletext::typing::Send::Fetcher { .. })) {
                    sends.extend(self.pick_under(&intent));
                }
                // What went is answered by the page's next frame, which
                // its interval does not hold (REQ-RCH-002).
                if sends.iter().any(|send| matches!(send, neoteletext::typing::Send::Fetcher { .. })) {
                    self.frames.input(intent.key);
                    if intent.intent.is_activation() {
                        // The viewer acted: a channel may now ask to know
                        // them (VS-104, VS-212).
                        self.viewer_gesture(intent.key.channel);
                        self.session.viewer_input(intent.key.channel);
                    }
                }
                self.deliver_typing(sends)?;
            }
            ToCore::Editing(editing) => {
                let sends = self.typing.editing(editing, self.now_us());
                self.deliver_typing(sends)?;
            }
            ToCore::WallWithoutPage => {
                if let Some(channel) = self.active_channel() {
                    let sends = self.typing.selects_nothing(channel, self.now_us());
                    self.deliver_typing(sends)?;
                }
            }
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhonePanel(open) => self.phone_panel(open)?,
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhonePin => self.phone_with_pin()?,
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhoneFinish => {
                if let Some((_, input)) = &mut self.phone {
                    let _ = protocol::keyboard::write_to_keyboard(input, &protocol::keyboard::ToKeyboard::Finish);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Delivers what the typing rules decided: input messages to a
    /// channel's fetcher, each move tagged with its contact and the
    /// messages of one moment marked to leave together; the round trip's
    /// measurement on or off; the typing state to the interface.
    #[cfg(feature = "interaction")]
    pub(super) fn deliver_typing(&mut self, sends: Vec<neoteletext::typing::Send>) -> Result<(), String> {
        use neoteletext::typing::Send;
        use protocol::fetcher::{ToFetcher, write_to_fetcher};
        for send in sends {
            match send {
                Send::Fetcher { channel, bytes, move_of, more } => {
                    // The trace reads the message as it goes (`input_trace.rs`).
                    let line = self.input_trace.as_mut().map(|trace| trace.note(&bytes));
                    self.record_sent_bytes(channel, &bytes);
                    let Some(input) = self.fetchers.get_mut(&channel).and_then(super::processes::Fetcher::input) else { continue };
                    let length = bytes.len();
                    // A fetcher that cannot be written to has stopped; its
                    // reader reports that, and the socket's state follows.
                    if write_to_fetcher(input, &ToFetcher::Send { bytes, move_of, more }).is_ok() {
                        self.console.sent(channel, length);
                        if let Some(line) = line {
                            self.send(&ToInterface::InputTrace(line))?;
                        }
                    }
                }
                Send::Measure { channel, on } => {
                    if let Some(input) = self.fetchers.get_mut(&channel).and_then(super::processes::Fetcher::input) {
                        let _ = write_to_fetcher(input, &ToFetcher::MeasureRoundTrip(on));
                    }
                }
                Send::Interface(state) => {
                    self.send(&ToInterface::Typing(state))?;
                    // Keys may have started or stopped going to the
                    // channel: the looks and the hover follow (REQ-6374).
                    self.regis_repoint()?;
                }
            }
        }
        Ok(())
    }

    /// When the pointer's next waiting move falls due (REQ-RCH-032): the
    /// trusted core's timer for it, none while nothing waits.
    #[cfg(feature = "interaction")]
    pub(super) fn moves_due_at(&self) -> Option<std::time::Instant> {
        self.typing.due().and_then(|due| self.started.checked_add(std::time::Duration::from_micros(due)))
    }

    /// Writes the moves whose period has ended (REQ-RCH-031).
    #[cfg(feature = "interaction")]
    pub(super) fn send_due_moves(&mut self) -> Result<(), String> {
        let sends = self.typing.take_due(self.now_us());
        self.deliver_typing(sends)
    }
}

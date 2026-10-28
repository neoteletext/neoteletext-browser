//! Typing into a page (`interaction.md`, INPUT.md): the keyboard's intents
//! for the open page's channel, and what the typing rules decided,
//! delivered.

use protocol::{ToCore, ToInterface};

use super::Core;

impl Core {
    /// The keyboard's intents: input for the open page's channel, and the
    /// phone keyboard's panel.
    #[cfg(feature = "interaction")]
    pub(super) fn input_intent(&mut self, intent: ToCore) -> Result<(), String> {
        match intent {
            ToCore::Input(intent) => {
                // A move with no button held goes only in a pointer area of
                // the page's latest image (INPUT.md section 7.4): the
                // interface asks, the trusted core decides.
                #[cfg(feature = "links")]
                if let protocol::interaction::Intent::Mouse { action: neoteletext_input::event::MouseAction::Move, button: 0, at } = &intent.intent {
                    let cell = (u8::try_from(at.column).unwrap_or(u8::MAX), u8::try_from(at.row).unwrap_or(u8::MAX));
                    if !self.links.may_move(intent.key, cell.0, cell.1) {
                        return Ok(());
                    }
                }
                // A HOVER names a control of the page's latest image, or
                // goes nowhere (REQ-INT-121).
                #[cfg(feature = "links")]
                if let protocol::interaction::Intent::Hover { column, row, .. } = &intent.intent
                    && !self.links.is_control_at(intent.key, *column, *row)
                {
                    return Ok(());
                }
                let sends = self.typing.input_intent(&intent, self.now());
                // What went is answered by the page's next frame, which
                // its interval does not hold (REQ-RCH-002).
                if sends.iter().any(|send| matches!(send, neoteletext::typing::Send::Fetcher(..))) {
                    self.frames.input(intent.key);
                }
                self.deliver_typing(sends)?;
            }
            ToCore::Editing(editing) => {
                let sends = self.typing.editing(editing, self.now());
                self.deliver_typing(sends)?;
            }
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhonePanel(open) => self.phone_panel(open)?,
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
    /// channel's fetcher, the typing state to the interface.
    #[cfg(feature = "interaction")]
    pub(super) fn deliver_typing(&mut self, sends: Vec<neoteletext::typing::Send>) -> Result<(), String> {
        use protocol::fetcher::{ToFetcher, write_to_fetcher};
        for send in sends {
            match send {
                neoteletext::typing::Send::Fetcher(channel, record) => {
                    // The trace reads the message as it goes (`input_trace.rs`).
                    let line = self.input_trace.as_mut().map(|trace| trace.note(&record));
                    let Some(input) = self.fetchers.get_mut(&channel).and_then(super::processes::Fetcher::input) else { continue };
                    let length = record.len();
                    // A fetcher that cannot be written to has stopped; its
                    // reader reports that, and the socket's state follows.
                    if write_to_fetcher(input, &ToFetcher::Send(record)).is_ok() {
                        self.console.sent(channel, length);
                        if let Some(line) = line {
                            self.send(&ToInterface::InputTrace(line))?;
                        }
                    }
                }
                neoteletext::typing::Send::Interface(state) => self.send(&ToInterface::Typing(state))?,
            }
        }
        Ok(())
    }
}

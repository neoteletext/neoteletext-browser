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
                let sends = self.typing.input_intent(&intent, self.now());
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
                    let Some(input) = self.fetchers.get_mut(&channel).and_then(|fetcher| fetcher.input.as_mut()) else { continue };
                    // A fetcher that cannot be written to has stopped; its
                    // reader reports that, and the socket's state follows.
                    let _ = write_to_fetcher(input, &ToFetcher::Send(record));
                }
                neoteletext::typing::Send::Interface(state) => self.send(&ToInterface::Typing(state))?,
            }
        }
        Ok(())
    }
}

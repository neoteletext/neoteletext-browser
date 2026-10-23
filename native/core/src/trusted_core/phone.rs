//! The phone keyboard (INPUT.md section 10, REQ-INT-138): a
//! process of its own holds the pairing secret; the trusted core shows its
//! QR in the panel and gives the interface the phone's keys, which act as
//! the interface's own.

use confine::broker::Program;
use neoteletext::outbox::Outbox;
use protocol::console::Level;
use protocol::{ToInterface, clean};

use super::processes::{relay, spawn};
use super::{Core, Event};

/// A relay other than qrkeyboard's own, for testing against one run
/// locally.
const RELAY_VARIABLE: &str = "NEOTELETEXT_QRKEYBOARD_RELAY";

impl Core {
    fn send_phone(&mut self) -> Result<(), String> {
        let view = self.phone_view.clone();
        self.send(&ToInterface::Phone(view))
    }

    /// The panel opened or closed. Opening starts the keyboard process if
    /// none runs; closing it before a phone is paired ends it.
    pub(super) fn phone_panel(&mut self, open: bool) -> Result<(), String> {
        use protocol::keyboard::PhoneState;
        if !open {
            if self.phone_view.state != PhoneState::Connected
                && let Some((mut child, input)) = self.phone.take()
            {
                // Its input closing ends it, and the relay hears it leave.
                drop(input);
                let _ = child.wait();
                self.phone_view = protocol::keyboard::PhoneView { state: PhoneState::Finished, qr: None, pin: None };
                self.send_phone()?;
            }
            return Ok(());
        }
        if self.phone.is_some() {
            return self.send_phone();
        }
        let mut arguments = Vec::new();
        // Without one, the keyboard process uses qrkeyboard's own relay. Only
        // an address is passed on: anything else would be read as a flag.
        if let Ok(relay) = std::env::var(RELAY_VARIABLE)
            && (relay.starts_with("https://") || relay.starts_with("http://"))
        {
            arguments.push(relay);
        }
        // The proxy, given only at its start (requirement 19.12).
        if let Some(proxy) = &self.settings.proxy {
            arguments.push(String::from("--proxy"));
            arguments.push(proxy.url());
        }
        let mut child = spawn(self.broker()?, Program::Keyboard, &arguments, true)?;
        let output = child.stdout.take().ok_or("the phone keyboard has no output")?;
        let input = Outbox::new(child.stdin.take().ok_or("the phone keyboard has no input")?);
        let events = self.events.clone();
        relay(output, protocol::keyboard::read_from_keyboard, events, Event::Phone, Event::PhoneEnded);
        self.note(Level::Info, None, &format!("phone keyboard: confined: {}", child.report));
        self.phone = Some((child, input));
        self.phone_view = protocol::keyboard::PhoneView { state: PhoneState::Preparing, qr: None, pin: None };
        self.send_phone()
    }

    /// What the keyboard process said: its QR and state for the panel, and
    /// the phone's keys, only while it is paired.
    pub(super) fn phone_said(&mut self, message: protocol::keyboard::FromKeyboard) -> Result<(), String> {
        use protocol::keyboard::{FromKeyboard, PhoneKey, PhoneState};
        match message {
            FromKeyboard::Join { qr, pin } => {
                self.phone_view.qr = Some(qr);
                self.phone_view.pin = pin;
                self.send_phone()
            }
            FromKeyboard::State(state) => {
                self.phone_view.state = state;
                if matches!(state, PhoneState::Finished | PhoneState::Connected) {
                    // Once paired, or over, the QR is not shown again.
                    self.phone_view.qr = None;
                    self.phone_view.pin = None;
                }
                self.send_phone()
            }
            FromKeyboard::Key { down, name } if self.phone_view.state == PhoneState::Connected => {
                self.send(&ToInterface::PhoneKey(PhoneKey::Key { down, name }))
            }
            FromKeyboard::Text(text) if self.phone_view.state == PhoneState::Connected => {
                self.send(&ToInterface::PhoneKey(PhoneKey::Text(clean(&text, protocol::keyboard::MAX_PHONE_TEXT))))
            }
            FromKeyboard::Key { .. } | FromKeyboard::Text(_) => Ok(()),
        }
    }

    pub(super) fn phone_ended(&mut self, reason: Option<String>) -> Result<(), String> {
        if let Some(reason) = reason {
            self.log(Level::Error, None, &format!("the phone keyboard stopped: {reason}"));
        }
        if let Some((mut child, _)) = self.phone.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.phone_view = protocol::keyboard::PhoneView { state: protocol::keyboard::PhoneState::Finished, qr: None, pin: None };
        self.send_phone()
    }
}

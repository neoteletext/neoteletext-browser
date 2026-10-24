//! Page 600's phone keyboard: the channel as the screen of qrkeyboard,
//! in a thread of its own, and what the phone sends.

use std::fmt::Write as _;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::Session;

/// The phone keyboard of page 600: the screen side in its own thread.
pub(super) struct Phone {
    events: mpsc::Receiver<PhoneEvent>,
    rows: Vec<Vec<u8>>,
    state: String,
    typed: String,
}

impl Phone {
    /// The QR as mosaic rows, once the relay gave the instance.
    pub(super) fn rows(&self) -> &[Vec<u8>] {
        &self.rows
    }

    /// What the page says of the phone.
    pub(super) fn state(&self) -> &str {
        &self.state
    }

    /// What the phone typed, its last characters.
    pub(super) fn typed(&self) -> &str {
        &self.typed
    }
}

enum PhoneEvent {
    Join(Vec<Vec<u8>>, Option<String>),
    State(String),
    Text(String),
    Key(String),
}

impl Session {
    pub(super) fn start_phone(&mut self) {
        let Some(relay) = self.relay.clone() else { return };
        let (sender, events) = mpsc::channel();
        thread::spawn(move || phone(&relay, &sender));
        self.phone = Some(Phone { events, rows: Vec::new(), state: "Preparing...".to_owned(), typed: String::new() });
    }

    /// What the phone did since the last look; whether the page changed.
    pub(super) fn phone_events(&mut self) -> bool {
        let Some(phone) = &mut self.phone else { return false };
        let mut changed = false;
        while let Ok(event) = phone.events.try_recv() {
            changed = true;
            match event {
                PhoneEvent::Join(rows, pin) => {
                    phone.rows = rows;
                    // Without a PIN the phone is connected by its first key
                    // (the engine's decision E.116 item 3); a row holds 39
                    // characters after its colour.
                    phone.state = pin.map_or_else(|| "Scan with your phone, then press a key.".to_owned(), |pin| format!("Scan, then enter PIN {pin}"));
                }
                PhoneEvent::State(state) => phone.state = state,
                PhoneEvent::Text(text) => phone.typed.push_str(&text),
                PhoneEvent::Key(key) if key == "Backspace" => {
                    phone.typed.pop();
                }
                PhoneEvent::Key(key) => {
                    let _ = write!(phone.typed, "({key})");
                }
            }
            let excess = phone.typed.chars().count().saturating_sub(24);
            phone.typed = phone.typed.chars().skip(excess).collect();
        }
        changed && self.page == 0x600
    }
}

/// The channel as the screen of qrkeyboard (INPUT.md section 10):
/// the instance, its QR as mosaic rows, and the phone's keys.
fn phone(relay: &str, events: &mpsc::Sender<PhoneEvent>) {
    use neoteletext_qrkeyboard::config::{Layout, ScreenConfig};
    use neoteletext_qrkeyboard::qr::{Correction, Matrix};
    use neoteletext_qrkeyboard::screen::{Event as ScreenEvent, Screen};
    let config = ScreenConfig { name: "NeoTeletext terminal channel".into(), layout: Layout::Terminal, pin: false, rejoin_seconds: Some(30) };
    let mut screen = match Screen::create(relay, config, None) {
        Ok(screen) => screen,
        Err(error) => {
            let _ = events.send(PhoneEvent::State(format!("No relay: {error}")));
            return;
        }
    };
    let rows = Matrix::new(&screen.join().url, Correction::Low).map(|matrix| matrix.mosaic_rows()).unwrap_or_default();
    if events.send(PhoneEvent::Join(rows, screen.join().pin.clone())).is_err() {
        return;
    }
    loop {
        let Ok(happened) = screen.wait(Some(Duration::from_millis(200))) else { return };
        for event in happened {
            let sent = match event {
                ScreenEvent::Connected => events.send(PhoneEvent::State("Phone connected: type on it.".into())),
                ScreenEvent::Text(text) => events.send(PhoneEvent::Text(text)),
                ScreenEvent::Special { key, .. } => events.send(PhoneEvent::Key(key.to_owned())),
                ScreenEvent::Down(key) => events.send(PhoneEvent::Key(key)),
                ScreenEvent::Disconnected(_) => events.send(PhoneEvent::State("Phone gone: scan again.".into())),
                ScreenEvent::Finished => {
                    let _ = events.send(PhoneEvent::State("Session finished.".into()));
                    return;
                }
                _ => Ok(()),
            };
            if sent.is_err() {
                screen.close();
                return;
            }
        }
    }
}

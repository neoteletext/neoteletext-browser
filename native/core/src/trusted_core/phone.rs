//! The phone keyboard (INPUT.md section 10, REQ-INT-138): a
//! process of its own holds the pairing secret; the trusted core shows its
//! QR in the panel and gives the interface the phone's keys, which act as
//! the interface's own.

use confine::broker::Program;
use neoteletext::outbox::Outbox;
use protocol::console::Level;
use protocol::{ToInterface, clean};

use super::processes::{end, relay, spawn};
use super::{Core, Event};

/// The phone keys passed on while no page is open for typing (decision
/// 99): the digits, the four arrows and the four Fastext colour keys, by
/// their W3C names as the phone's layout sends them.
// Owed: the protocol crate is to hold this list beside the layout (the
// security fix plan's WP-5); this is then to use it by name.
const PAGE_NAVIGATION_KEYS: [&str; 18] = [
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "ColorF0Red",
    "ColorF1Green",
    "ColorF2Yellow",
    "ColorF3Blue",
];

/// How long a keyboard process the panel closed has to end by itself, as
/// its input closing tells it to, before it is killed: time to tell the
/// relay it leaves, which a held connection cannot hold up longer.
pub(super) const PHONE_END_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// A relay other than qrkeyboard's own, for testing against one run
/// locally.
const RELAY_VARIABLE: &str = "NEOTELETEXT_QRKEYBOARD_RELAY";

/// The relay a test names in [`RELAY_VARIABLE`], honoured only in a debug
/// build (decision 100, finding F-64): a release build pairs only through
/// qrkeyboard's own relay, and its keyboard process has no local network.
/// Only an address is passed on: anything else would be read as a flag.
fn relay_override(named: Option<String>, debug: bool) -> Option<String> {
    named.filter(|relay| debug && (relay.starts_with("https://") || relay.starts_with("http://")))
}

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
                && let Some((child, input)) = self.phone.take()
            {
                // Its input closing ends it, and the relay hears it leave.
                // It is not waited for here: one held connecting to a relay
                // would hold every tab (finding F-26). Its end is reaped
                // when its pipe closes, or it is killed once its grace is up.
                drop(input);
                if let Some((mut earlier, _, _)) = self.phone_ending.take() {
                    end(&mut earlier);
                }
                let grace = std::time::Instant::now().checked_add(PHONE_END_GRACE).unwrap_or_else(std::time::Instant::now);
                self.phone_ending = Some((child, self.phone_generation, grace));
                self.phone_view = protocol::keyboard::PhoneView { state: PhoneState::Finished, qr: None, pin: None };
                self.send_phone()?;
            }
            return Ok(());
        }
        if self.phone.is_some() {
            return self.send_phone();
        }
        let mut arguments = Vec::new();
        // Without one, the keyboard process uses qrkeyboard's own relay.
        let test_relay = relay_override(std::env::var(RELAY_VARIABLE).ok(), cfg!(debug_assertions));
        arguments.extend(test_relay.clone());
        // The proxy, given only at its start (requirement 19.12).
        if let Some(proxy) = &self.settings.proxy {
            arguments.push(String::from("--proxy"));
            arguments.push(proxy.url());
        }
        // The local network only for a test's own relay (decision 100).
        let mut child = spawn(self.broker()?, Program::Keyboard, &arguments, test_relay.is_some())?;
        let output = child.stdout.take().ok_or("the phone keyboard has no output")?;
        let input = Outbox::new(child.stdin.take().ok_or("the phone keyboard has no input")?);
        let events = self.events.clone();
        self.phone_generation = self.phone_generation.wrapping_add(1);
        let generation = self.phone_generation;
        relay(
            output,
            protocol::keyboard::read_from_keyboard,
            events,
            move |message| Event::Phone(generation, message),
            move |end| Event::PhoneEnded(generation, end),
        );
        self.note(Level::Info, None, &format!("phone keyboard: confined: {}", child.report));
        self.phone = Some((child, input));
        self.phone_view = protocol::keyboard::PhoneView { state: PhoneState::Preparing, qr: None, pin: None };
        self.send_phone()
    }

    /// What the keyboard process said: its QR and state for the panel, and
    /// the phone's keys, only while it is paired.
    pub(super) fn phone_said(&mut self, generation: u64, message: protocol::keyboard::FromKeyboard) -> Result<(), String> {
        use protocol::keyboard::{FromKeyboard, PhoneKey, PhoneState};
        // A keyboard process the panel closed has nothing more to say.
        if generation != self.phone_generation || self.phone_ending.as_ref().is_some_and(|(_, ending, _)| *ending == generation) {
            return Ok(());
        }
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
            // Outside a page open for typing, only the keys that move
            // between pages: the interface's code is not trusted to keep
            // the rest from a field, a prompt or a panel (decision 99).
            FromKeyboard::Key { down, name }
                if self.phone_view.state == PhoneState::Connected && (self.typing.page_takes_keys() || PAGE_NAVIGATION_KEYS.contains(&name.as_str())) =>
            {
                self.send(&ToInterface::PhoneKey(PhoneKey::Key { down, name }))
            }
            FromKeyboard::Text(text) if self.phone_view.state == PhoneState::Connected && self.typing.page_takes_keys() => {
                self.send(&ToInterface::PhoneKey(PhoneKey::Text(clean(&text, protocol::keyboard::MAX_PHONE_TEXT))))
            }
            FromKeyboard::Key { .. } | FromKeyboard::Text(_) => Ok(()),
        }
    }

    /// When a keyboard process the panel closed is to be killed.
    pub(super) fn phone_ending_until(&self) -> Option<std::time::Instant> {
        self.phone_ending.as_ref().map(|(_, _, until)| *until)
    }

    /// Kills a keyboard process the panel closed that has not ended in its
    /// grace: killed, it ends at once, so it is reaped without a wait.
    pub(super) fn phone_tick(&mut self, now: std::time::Instant) {
        if let Some((mut child, _, _)) = self.phone_ending.take_if(|(_, _, until)| now >= *until) {
            end(&mut child);
        }
    }

    pub(super) fn phone_ended(&mut self, generation: u64, reason: Option<String>) -> Result<(), String> {
        // One the panel closed is reaped: it has ended, so this does not
        // wait (finding F-26).
        if let Some((mut child, _, _)) = self.phone_ending.take_if(|(_, ending, _)| *ending == generation) {
            end(&mut child);
            return Ok(());
        }
        if generation != self.phone_generation {
            return Ok(());
        }
        if let Some(reason) = reason {
            self.log(Level::Error, None, &format!("the phone keyboard stopped: {reason}"));
        }
        if let Some((mut child, _)) = self.phone.take() {
            end(&mut child);
        }
        self.phone_view = protocol::keyboard::PhoneView { state: protocol::keyboard::PhoneState::Finished, qr: None, pin: None };
        self.send_phone()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::super::test_support::Test;
    use protocol::ToInterface;
    use protocol::keyboard::{FromKeyboard, PhoneKey, PhoneState};

    /// A keyboard of a test that ignores its input closing: it sleeps for
    /// `seconds`.
    #[cfg(unix)]
    fn deaf_keyboard(seconds: &str) -> confine::broker::Helper {
        let program = std::path::PathBuf::from("/bin/sleep");
        let arguments = [std::ffi::OsString::from(seconds)];
        confine::broker::Helper::local(
            confine::spawn(&confine::Spawn {
                program: &program,
                arguments: &arguments,
                role: confine::Role::Network,
                environment: &[],
                local_network: false,
                writable: None,
            })
            .unwrap(),
        )
    }

    /// Finding F-26: closing the panel never waits on the keyboard process
    /// on the trusted core's thread, however long it takes to go, as one
    /// held connecting to a relay does; it is killed once its grace is up.
    #[cfg(unix)]
    #[test]
    fn closing_the_panel_never_waits_on_the_keyboard() {
        let mut test = Test::new("phone-close");
        let mut child = deaf_keyboard("5");
        let input = neoteletext::outbox::Outbox::new(child.stdin.take().unwrap());
        test.core.phone = Some((child, input));
        test.core.phone_view.state = PhoneState::Preparing;
        let started = std::time::Instant::now();
        test.core.phone_panel(false).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(100), "the panel closed at once: {:?}", started.elapsed());
        assert_eq!(test.core.phone_view.state, PhoneState::Finished);
        test.core.phone_tick(std::time::Instant::now() + super::PHONE_END_GRACE);
        assert!(test.core.phone_ending.is_none(), "killed and reaped once its grace was up");
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "killed, not waited for: {:?}", started.elapsed());
    }

    /// Finding F-64: a release build honours no relay override, so its
    /// keyboard process gets no local network; a debug build, for tests,
    /// does.
    #[test]
    fn only_a_debug_build_takes_another_relay() {
        let local = Some(String::from("http://127.0.0.1:8090"));
        assert_eq!(super::relay_override(local.clone(), false), None);
        assert_eq!(super::relay_override(local.clone(), true), local);
        assert_eq!(super::relay_override(Some(String::from("--proxy")), true), None, "only an address");
    }

    fn key(name: &str) -> FromKeyboard {
        FromKeyboard::Key { down: true, name: name.into() }
    }

    /// The phone keys the interface was given.
    fn given(test: &mut Test) -> Vec<PhoneKey> {
        test.sent().into_iter().filter_map(|message| if let ToInterface::PhoneKey(key) = message { Some(key) } else { None }).collect()
    }

    /// Decision 99 (finding F-20): with no page open for typing, a paired
    /// phone moves between pages and does nothing else: its digits,
    /// arrows and colour keys reach the interface; Tab, Enter, Escape,
    /// Backspace, letters and text do not, so it cannot reach a field, a
    /// prompt or a panel of the browser's own.
    #[test]
    fn a_phone_outside_a_typing_page_only_moves_between_pages() {
        let mut test = Test::new("phone-keys");
        test.core.phone_view.state = PhoneState::Connected;
        for name in ["Tab", "Enter", "Escape", "Backspace", "a", "Z"] {
            test.core.phone_said(0, key(name)).unwrap();
        }
        test.core.phone_said(0, FromKeyboard::Text("example.com".into())).unwrap();
        assert_eq!(given(&mut test), Vec::new(), "nothing but page navigation");
        for name in ["1", "0", "ArrowUp", "ArrowLeft", "ColorF0Red", "ColorF3Blue"] {
            test.core.phone_said(0, key(name)).unwrap();
        }
        let names: Vec<String> = given(&mut test).into_iter().map(|key| if let PhoneKey::Key { name, .. } = key { name } else { String::new() }).collect();
        assert_eq!(names, ["1", "0", "ArrowUp", "ArrowLeft", "ColorF0Red", "ColorF3Blue"]);
    }
}

//! The pointer and the contacts of a touch screen (`neoteletext-common/
//! INPUT.md` sections 5.4, 9.8 and 9.9; `RETURN-CHANNEL.md` sections 4.1
//! and 4.4): every change of the dot over the open page as a move, at most
//! 60 a second by the engine's [`Pacer`](neoteletext_input::pace::Pacer),
//! the latest kept and flushed before any other event; gone when the
//! pointer leaves the page or keys stop going; and each contact numbered by
//! the engine's [`Contacts`](neoteletext_input::contacts::Contacts), the
//! first as MOUSE, button 0, and the others as TOUCH.
//!
//! The interface passes what it saw; nothing it sends sets the pace, a
//! contact's number, the buttons a move holds or where a gone was
//! (REQ-RCH-036, REQ-RCH-053): those are this module's.

use neoteletext_input::caps::TOUCH_CAP;
use neoteletext_input::contacts::MOUSE_CONTACT;
use neoteletext_input::event::{Device, Event, Item, Mouse, MouseAction, Point, Touch};
use neoteletext_input::pace::Offer;
use protocol::interaction::{Intent, TouchPhase};

use super::{Channel, Send, Typing, together};

/// The bit of the mask of buttons held that is button `button`, or none
/// for a button the mask cannot hold (MOUSE's `button` on a move is a
/// byte, INPUT.md section 5.4).
fn button_bit(button: u8) -> u8 {
    1_u8.checked_shl(u32::from(button)).unwrap_or(0)
}

/// What a finger as MOUSE holds: button 0, the main one (REQ-RCH-041).
const FINGER_BUTTON: u8 = 0;

impl Channel {
    /// Records where a pointer event that went was, and what the mouse
    /// holds, for the pointer's gone.
    pub(super) fn sent_pointer(&mut self, item: &Item<'_>) {
        match item {
            Item::Event(Event::Mouse(mouse)) => {
                let index = usize::from(MOUSE_CONTACT);
                match mouse.action {
                    MouseAction::Gone => {
                        self.held = 0;
                        set(&mut self.last_at, index, None);
                    }
                    // A finger that lifts is gone from the page: no hover
                    // from a finger (REQ-RCH-049).
                    MouseAction::Release if mouse.device == Device::Touch => set(&mut self.last_at, index, None),
                    _ => set(&mut self.last_at, index, Some(mouse.at)),
                }
                if mouse.device == Device::Mouse {
                    match mouse.action {
                        MouseAction::Press => self.held |= button_bit(mouse.button),
                        MouseAction::Release => self.held &= !button_bit(mouse.button),
                        MouseAction::Move | MouseAction::Gone => {}
                    }
                }
            }
            Item::Event(Event::Touch(touch)) => {
                let at = Point { column: touch.column, row: touch.row, pixels: touch.pixels, mods: 0 };
                let ended = matches!(touch.action, MouseAction::Release | MouseAction::Gone);
                set(&mut self.last_at, usize::from(touch.id), (!ended).then_some(at));
            }
            Item::Event(_) | Item::Text(_) => {}
        }
    }

    /// Writes every waiting move, before any other event (REQ-RCH-033).
    pub(super) fn flush(&mut self, channel: u32, now_us: u64, sends: &mut Vec<Send>) {
        let mut waiting = Vec::new();
        self.pacer.flush(now_us, &mut |event| waiting.push(event));
        for event in waiting {
            self.send(channel, &Item::Event(event), now_us, sends);
        }
    }

    /// A move, paced: sent now if its contact's period has ended, else
    /// kept as the contact's latest for [`Typing::take_due`]; never an
    /// unchanged one (REQ-RCH-032).
    fn offer(&mut self, channel: u32, event: Event<'static>, now_us: u64, sends: &mut Vec<Send>) {
        if let Offer::Now(event) = self.pacer.offer(event, now_us) {
            self.send(channel, &Item::Event(event), now_us, sends);
        }
    }

    /// The waiting moves, then gone for the pointer and every contact the
    /// channel knows of (REQ-RCH-034, REQ-RCH-046): what they held is
    /// released without a click. Then nothing is down and nothing waits.
    pub(super) fn let_go(&mut self, channel: u32, now_us: u64, sends: &mut Vec<Send>) {
        self.flush(channel, now_us, sends);
        for contact in 0..self.last_at.len() {
            if let Some(event) = self.gone(contact) {
                self.send(channel, &Item::Event(event), now_us, sends);
            }
        }
        self.pointer_forgotten();
    }

    /// The gone of `contact` from where it was last sent, if the channel
    /// knows of it.
    fn gone(&self, contact: usize) -> Option<Event<'static>> {
        let at = self.last_at.get(contact).copied().flatten()?;
        if contact == usize::from(MOUSE_CONTACT) {
            let (button, device) = if self.finger.is_some() { (button_bit(FINGER_BUTTON), Device::Touch) } else { (self.held, Device::Mouse) };
            return Some(Event::Mouse(Mouse { action: MouseAction::Gone, button, at, device }));
        }
        let id = u8::try_from(contact).ok()?;
        Some(Event::Touch(Touch { action: MouseAction::Gone, id, column: at.column, row: at.row, pixels: at.pixels }))
    }

    /// A pointer intent: the mouse, its leaving the page, or a contact.
    pub(super) fn pointer(&mut self, channel: u32, intent: &Intent, now_us: u64, sends: &mut Vec<Send>) {
        match *intent {
            // While a finger is MOUSE, the mouse does not share it
            // (REQ-RCH-041).
            Intent::Mouse { .. } | Intent::Gone if self.finger.is_some() => {}
            Intent::Mouse { action: MouseAction::Move, at, .. } => {
                // The buttons held are those this core sent pressed, never
                // what the interface says.
                let event = Event::Mouse(Mouse { action: MouseAction::Move, button: self.held, at, device: Device::Mouse });
                self.offer(channel, event, now_us, sends);
            }
            Intent::Mouse { action, button, at } => {
                if action == MouseAction::Gone || (action == MouseAction::Release && self.held & button_bit(button) == 0) {
                    return;
                }
                self.flush(channel, now_us, sends);
                self.send(channel, &Item::Event(Event::Mouse(Mouse { action, button, at, device: Device::Mouse })), now_us, sends);
            }
            Intent::Gone => {
                self.flush(channel, now_us, sends);
                if let Some(event) = self.gone(usize::from(MOUSE_CONTACT)) {
                    self.send(channel, &Item::Event(event), now_us, sends);
                }
                self.held = 0;
                self.pacer.forget(usize::from(MOUSE_CONTACT));
            }
            Intent::Touch { phase, contact, at } => self.touch(channel, phase, contact, at, now_us, sends),
            _ => {}
        }
    }

    /// A contact, as the system reported it: numbered here, so the
    /// interface cannot invent one, move or end one not begun, or pass
    /// `MAX_CONTACTS` (REQ-RCH-053).
    fn touch(&mut self, channel: u32, phase: TouchPhase, system: u64, at: Point, now_us: u64, sends: &mut Vec<Send>) {
        let touch_agreed = self.session.caps() & TOUCH_CAP != 0;
        let event = |action: MouseAction, number: u8, at: Point| {
            if number == MOUSE_CONTACT {
                let button = if action == MouseAction::Move { button_bit(FINGER_BUTTON) } else { FINGER_BUTTON };
                Event::Mouse(Mouse { action, button, at, device: Device::Touch })
            } else {
                Event::Touch(Touch { action, id: number, column: at.column, row: at.row, pixels: at.pixels })
            }
        };
        match phase {
            TouchPhase::Began => {
                // Down at once, never held to tell a gesture (REQ-RCH-044).
                let Some(number) = self.contacts.begin(system, self.held != 0, touch_agreed) else { return };
                self.flush(channel, now_us, sends);
                if number == MOUSE_CONTACT {
                    self.finger = Some(system);
                }
                self.send(channel, &Item::Event(event(MouseAction::Press, number, at)), now_us, sends);
            }
            TouchPhase::Moved => {
                let Some(number) = self.contacts.number(system) else { return };
                self.offer(channel, event(MouseAction::Move, number, at), now_us, sends);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let Some(number) = self.contacts.number(system) else { return };
                self.flush(channel, now_us, sends);
                self.contacts.end(system);
                let ended = if phase == TouchPhase::Ended {
                    Some(event(MouseAction::Release, number, at))
                } else {
                    // Cancelled: gone, from where it was last sent, so the
                    // channel releases it without a click (REQ-RCH-046).
                    self.gone(usize::from(number))
                };
                if let Some(ended) = ended {
                    self.send(channel, &Item::Event(ended), now_us, sends);
                }
                if number == MOUSE_CONTACT {
                    self.finger = None;
                }
                set(&mut self.last_at, usize::from(number), None);
                self.pacer.forget(usize::from(number));
            }
        }
    }
}

/// Sets `slots[index]`, when there is one.
fn set(slots: &mut [Option<Point>], index: usize, value: Option<Point>) {
    if let Some(slot) = slots.get_mut(index) {
        *slot = value;
    }
}

impl Typing {
    /// When the next waiting move falls due, in the trusted core's
    /// microseconds: its one timer for the pointer, and none while nothing
    /// waits (REQ-RCH-032).
    #[must_use]
    pub fn due(&self) -> Option<u64> {
        self.channels.values().filter_map(|state| state.pacer.due()).min()
    }

    /// Writes every move whose period has ended by `now_us`; the moves of
    /// one channel due together leave in one write (REQ-RCH-045).
    pub fn take_due(&mut self, now_us: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        for (&channel, state) in &mut self.channels {
            let mut due = Vec::new();
            state.pacer.take_due(now_us, &mut |event| due.push(event));
            for event in due {
                state.send(channel, &Item::Event(event), now_us, &mut sends);
            }
        }
        together(sends)
    }
}

//! Input from the user's real devices and the user's input method, through
//! the window on the user's desktop. Nothing a hosted program sends can reach
//! this path: the host offers no protocol for injecting input.

use std::io::Write;

use smithay::backend::input::{Axis, ButtonState, KeyState};
use smithay::input::keyboard::{FilterResult, Keycode};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use smithay::reexports::winit::platform::scancode::PhysicalKeyExtScancode;
use smithay::utils::SERIAL_COUNTER;

use crate::state::HostState;
use crate::strip::STRIP_HEIGHT;

/// Linux key codes are X key codes less 8 (xkbcommon, "keycodes").
const EVDEV_TO_XKB: u32 = 8;
/// Linux button codes (linux/input-event-codes.h).
const BUTTON_LEFT: u32 = 0x110;
const BUTTON_RIGHT: u32 = 0x111;
const BUTTON_MIDDLE: u32 = 0x112;
/// Pixels scrolled per wheel line, and the high-resolution wheel unit
/// (Wayland's `wl_pointer.axis_value120`: 120 per detent).
const PIXELS_PER_LINE: f64 = 15.0;
const VALUE120_PER_LINE: f64 = 120.0;

impl HostState {
    fn now_milliseconds(&self) -> u32 {
        u32::try_from(self.start_time.elapsed().as_millis()).unwrap_or(u32::MAX)
    }

    pub fn window_input(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, is_synthetic: false, .. } if !event.repeat => {
                let Some(scancode) = event.physical_key.to_scancode() else { return };
                let pressed = event.state == ElementState::Pressed;
                if pressed {
                    self.keys_down.push(scancode);
                } else if let Some(index) = self.keys_down.iter().position(|&down| down == scancode) {
                    self.keys_down.swap_remove(index);
                } else {
                    return;
                }
                let state = if pressed { KeyState::Pressed } else { KeyState::Released };
                self.key(Keycode::new(scancode + EVDEV_TO_XKB), state);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let position = (position.x, position.y).into();
                let under = self.surface_under(position);
                let time = self.now_milliseconds();
                let Some(pointer) = self.seat.get_pointer() else { return };
                pointer.motion(self, under, &MotionEvent { location: position, serial: SERIAL_COUNTER.next_serial(), time });
                pointer.frame(self);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let code = match button {
                    MouseButton::Left => BUTTON_LEFT,
                    MouseButton::Right => BUTTON_RIGHT,
                    MouseButton::Middle => BUTTON_MIDDLE,
                    _ => return,
                };
                let state = if state == ElementState::Pressed { ButtonState::Pressed } else { ButtonState::Released };
                self.button(code, state);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let time = self.now_milliseconds();
                let Some(pointer) = self.seat.get_pointer() else { return };
                let mut frame = AxisFrame::new(time);
                match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        for (axis, lines) in [(Axis::Horizontal, f64::from(x)), (Axis::Vertical, f64::from(y))] {
                            if lines != 0.0 {
                                let value120 = (-lines * VALUE120_PER_LINE).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX));
                                #[allow(clippy::cast_possible_truncation, reason = "rounded and clamped to the range of i32 above")]
                                let value120 = value120 as i32;
                                frame = frame.value(axis, -lines * PIXELS_PER_LINE).v120(axis, value120);
                            }
                        }
                    }
                    MouseScrollDelta::PixelDelta(position) => {
                        frame = frame.value(Axis::Horizontal, -position.x).value(Axis::Vertical, -position.y);
                    }
                }
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            WindowEvent::Ime(ime) => self.input_method(ime),
            _ => {}
        }
    }

    fn key(&mut self, code: Keycode, key_state: KeyState) {
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        let time = self.now_milliseconds();
        keyboard.input::<(), _>(self, code, key_state, SERIAL_COUNTER.next_serial(), time, |state, _, handle| {
            if !state.strip.focused() {
                return FilterResult::Forward;
            }
            if key_state == KeyState::Pressed
                && let Some(line) = state.strip.key(handle.modified_sym())
            {
                state.strip_input(&line);
            } else {
                state.accessibility_changed();
            }
            FilterResult::Intercept(())
        });
    }

    /// The real host sends this to the trusted core as strip input.
    pub fn strip_input(&mut self, line: &str) {
        println!("STRIP INPUT: \"{line}\"");
        let _ = std::io::stdout().flush();
        self.accessibility_changed();
    }

    fn button(&mut self, code: u32, button_state: ButtonState) {
        let (Some(pointer), Some(keyboard)) = (self.seat.get_pointer(), self.seat.get_keyboard()) else { return };
        let serial = SERIAL_COUNTER.next_serial();
        if button_state == ButtonState::Pressed {
            let location = pointer.current_location();
            if location.y < f64::from(STRIP_HEIGHT) {
                self.focus_strip();
            } else {
                self.strip.set_focused(false);
                let focus = self
                    .space
                    .element_under(location)
                    .and_then(|(window, _)| window.toplevel().map(|toplevel| toplevel.wl_surface().clone()));
                keyboard.set_focus(self, focus, serial);
                self.accessibility_changed();
            }
        }
        let time = self.now_milliseconds();
        pointer.button(self, &ButtonEvent { button: code, state: button_state, serial, time });
        pointer.frame(self);
    }

    pub fn focus_strip(&mut self) {
        self.strip.set_focused(true);
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Option::<WlSurface>::None, SERIAL_COUNTER.next_serial());
        }
        self.accessibility_changed();
    }

    /// Composed text from the user's input method goes to the strip, or to
    /// the focused hosted window through the text-input protocol; nothing of
    /// the input method itself is reachable from the nested display.
    fn input_method(&mut self, ime: Ime) {
        if self.strip.focused() {
            if let Ime::Commit(text) = ime {
                self.strip.insert(&text);
                self.accessibility_changed();
            }
            return;
        }
        match ime {
            Ime::Preedit(text, cursor) => {
                let (begin, end) = cursor.map_or((-1, -1), |(begin, end)| {
                    (i32::try_from(begin).unwrap_or(-1), i32::try_from(end).unwrap_or(-1))
                });
                self.text_inputs.deliver(Some((&text, begin, end)), None);
            }
            Ime::Commit(text) => self.text_inputs.deliver(None, Some(&text)),
            Ime::Enabled | Ime::Disabled => {}
        }
    }
}

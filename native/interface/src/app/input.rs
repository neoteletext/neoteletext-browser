//! The user's input to the window: the pointer, the buttons, the wheel and
//! the keys, given to the interface core.

use interface_core::browser::{Key as BrowserKey, Modifiers};
use interface_core::controls::FastextKey;
use protocol::ToCore;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::WindowId;

use super::App;
use super::clipboard::clipboard_key;

impl App {
    fn modifiers(&self) -> Modifiers {
        let command = if cfg!(target_os = "macos") { self.modifiers.super_key() } else { self.modifiers.control_key() };
        Modifiers { shift: self.modifiers.shift_key(), command, alt: self.modifiers.alt_key(), repeat: false }
    }

    /// The wheel: the channel's over its page when it asked for the mouse
    /// (INPUT.md section 9.1), in lines positive right and down; otherwise
    /// the browser's, to scroll. Returns whether the window needs drawing.
    fn wheel(&mut self, event_loop: &ActiveEventLoop, delta: MouseScrollDelta, now: u64) -> bool {
        let scale = self.scale();
        #[cfg(feature = "interaction")]
        {
            let lines = |pixels: f64| pixels / scale / interface_core::browser::LINE_HEIGHT;
            let channels = match delta {
                MouseScrollDelta::LineDelta(x, y) => self.view.browser.terminal_wheel(f64::from(x), -f64::from(y)),
                MouseScrollDelta::PixelDelta(position) => self.view.browser.terminal_wheel(lines(position.x), lines(-position.y)),
            };
            if let Some(changed) = channels {
                self.act(event_loop);
                return changed;
            }
        }
        #[cfg(not(feature = "interaction"))]
        let _ = event_loop;
        match delta {
            // A wheel that moves in notches; a trackpad reports pixels.
            MouseScrollDelta::LineDelta(_, y) => self.view.browser.wheel_lines(-f64::from(y), now),
            MouseScrollDelta::PixelDelta(position) => self.view.browser.scroll_by(-position.y / scale, now),
        }
    }

    fn key(&mut self, key: &Key, text: Option<&str>, repeat: bool) -> bool {
        let now = self.now();
        let modifiers = Modifiers { repeat, ..self.modifiers() };
        let named = match key {
            Key::Named(NamedKey::Tab) => Some(BrowserKey::Tab),
            Key::Named(NamedKey::Enter) => Some(BrowserKey::Enter),
            Key::Named(NamedKey::Escape) => Some(BrowserKey::Escape),
            Key::Named(NamedKey::Backspace) => Some(BrowserKey::Backspace),
            Key::Named(NamedKey::Delete) => Some(BrowserKey::Delete),
            Key::Named(NamedKey::ArrowLeft) => Some(BrowserKey::Left),
            Key::Named(NamedKey::ArrowRight) => Some(BrowserKey::Right),
            Key::Named(NamedKey::ArrowUp) => Some(BrowserKey::Up),
            Key::Named(NamedKey::ArrowDown) => Some(BrowserKey::Down),
            Key::Named(NamedKey::Home) => Some(BrowserKey::Home),
            Key::Named(NamedKey::End) => Some(BrowserKey::End),
            _ => None,
        };
        // F1 to F4: the coloured keys, as in the engine's viewer.
        let colour = match key {
            Key::Named(NamedKey::F1) => Some(FastextKey::Red),
            Key::Named(NamedKey::F2) => Some(FastextKey::Green),
            Key::Named(NamedKey::F3) => Some(FastextKey::Yellow),
            Key::Named(NamedKey::F4) => Some(FastextKey::Cyan),
            _ => None,
        };
        if let Some(colour) = colour {
            return self.view.browser.fastext_key(colour);
        }
        if let Some(named) = named {
            return self.view.browser.key(named, modifiers, now);
        }
        if modifiers.command {
            // The fields' clipboard keys (decision 84): on macOS the Edit
            // menu takes them first.
            if let Key::Character(character) = key
                && !modifiers.shift
                && !modifiers.alt
                && let Some(action) = clipboard_key(character)
            {
                return self.edit_field(action);
            }
            // Shortcuts use the key itself, not what it would type.
            if let Key::Character(character) = key {
                return self.view.browser.key(BrowserKey::Text(character), modifiers, now);
            }
            return false;
        }
        text.is_some_and(|text| self.view.browser.key(BrowserKey::Text(text), modifiers, now))
    }

    /// A press of the middle button: on a tab, it closes.
    fn middle_pressed(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let changed = self.view.browser.middle_pressed();
        self.act(event_loop);
        changed
    }

    pub(super) fn window_input(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        let scale = self.scale();
        let now = self.now();
        let changed = match event {
            // The trusted core closes the window and its tabs, or, for the
            // last window, ends the browser (`multiple-windows.md`).
            WindowEvent::CloseRequested => {
                self.send(event_loop, &ToCore::CloseWindow);
                false
            }
            WindowEvent::RedrawRequested => {
                self.redraw(event_loop);
                false
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => true,
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                #[cfg(feature = "interaction")]
                {
                    let held = self.browser_modifier();
                    self.view.browser.set_browser_modifier(held);
                }
                false
            }
            // A web page is handed over only while the window has the
            // keyboard focus (requirement 17.9).
            #[cfg(any(feature = "links", feature = "interaction"))]
            WindowEvent::Focused(focused) => {
                self.send(event_loop, &ToCore::Focused(focused));
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.view.cursor = (position.x / scale, position.y / scale);
                let changed = self.view.browser.pointer_moved(self.view.cursor.0, self.view.cursor.1, now);
                self.update_cursor();
                changed
            }
            // Leaving the window puts the pointer nowhere: what pops out
            // under it settles back.
            WindowEvent::CursorLeft { .. } => {
                self.view.cursor = (-1.0, -1.0);
                self.view.browser.pointer_moved(self.view.cursor.0, self.view.cursor.1, now)
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if self.resize_from_edge(self.view.cursor.0, self.view.cursor.1) {
                    return;
                }
                // Command-click (Control-click elsewhere) opens in a new tab
                // what opens a channel; anywhere else it is a press.
                let changed = (self.modifiers().command && self.view.browser.new_tab_pressed())
                    || self.view.browser.pointer_pressed(&mut self.view.painter, self.modifiers.shift_key(), now);
                self.act(event_loop);
                changed
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                self.view.browser.pointer_released();
                self.act(event_loop);
                true
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Middle, .. } => self.middle_pressed(event_loop),
            // A secondary click: Back's or Forward's list (requirement 20.10).
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                let changed = self.view.browser.secondary_pressed();
                self.act(event_loop);
                changed
            }
            WindowEvent::MouseWheel { delta, .. } => self.wheel(event_loop, delta, now),
            #[cfg(feature = "interaction")]
            WindowEvent::KeyboardInput { ref event, .. } if self.view.browser.typing_target().is_some() && !self.browser_modifier() => {
                let changed = self.terminal_key(event).unwrap_or(false);
                self.act(event_loop);
                changed
            }
            // The browser's paste shortcut while keys go to a channel: the
            // clipboard's text as one PASTE (INPUT.md section 9.1).
            #[cfg(feature = "interaction")]
            WindowEvent::KeyboardInput { ref event, .. } if event.state == ElementState::Pressed && self.is_paste(event) => {
                let changed = self.paste();
                self.act(event_loop);
                changed
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
                // A shortcut is its key without the modifiers: Option on
                // macOS changes the character a key types.
                let key = if self.modifiers().command { event.key_without_modifiers() } else { event.logical_key.clone() };
                let changed = self.key(&key, event.text.as_deref(), event.repeat);
                self.act(event_loop);
                changed
            }
            #[cfg(feature = "interaction")]
            WindowEvent::Ime(Ime::Commit(text)) if self.view.browser.typing_target().is_some() => {
                let changed = self.view.browser.terminal_text(&text, now);
                self.act(event_loop);
                changed
            }
            // The input method's preedit goes to the channel in the COMPOSE
            // mode (INPUT.md section 9.1), its cursor as a code-point
            // offset.
            #[cfg(feature = "interaction")]
            WindowEvent::Ime(Ime::Preedit(text, cursor)) if self.view.browser.composes_for_channel() => {
                let at = cursor.map_or(text.len(), |(start, _)| start);
                let offset = text.get(..at).map_or(0, |before| before.chars().count());
                let changed = self.view.browser.terminal_compose(u32::try_from(offset).unwrap_or(u32::MAX), &text);
                self.act(event_loop);
                changed
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                let changed = self.view.browser.key(BrowserKey::Text(&text), Modifiers::default(), now);
                self.act(event_loop);
                changed
            }
            _ => false,
        };
        if changed {
            self.request_redraw();
        }
    }
}

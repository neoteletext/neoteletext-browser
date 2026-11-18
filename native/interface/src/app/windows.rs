//! The windows: the one at hand and the others set aside, opening and
//! closing them as the trusted core asks, their titles and the Dock menu,
//! the frame's buttons and edges, and tabs moved between windows.

use std::rc::Rc;

use dock_menu::DockTab;
use interface_core::browser::{TabInfo, WindowAction};
use interface_core::library::WindowGeometry;
use protocol::ToCore;
use softbuffer::{Context, Surface};
use winit::dpi::LogicalPosition;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Fullscreen, ResizeDirection};

use crate::window::{FRAME, TORN_OFFSET, WINDOW_TITLE, attributes, geometry, on_a_display, resize_edge};

use super::{App, Graphics, Heard, View, WindowFull};

impl App {
    /// The window's title and the Dock menu follow the tabs: the title is
    /// the selected tab's, marked when it plays sound as a web browser
    /// marks it, so the Dock's window list names the selected tab.
    pub(super) fn show_tabs(&self) {
        let tab = |(index, tab): (usize, &TabInfo)| DockTab {
            title: tab.title.clone(),
            selected: index == self.view.active,
            playing: self.view.browser.tab_sounding(tab),
            muted: tab.muted,
        };
        let tabs: Vec<DockTab> = self.view.tabs.iter().enumerate().map(tab).collect();
        if let Some(graphics) = &self.view.graphics {
            let title = match tabs.get(self.view.active) {
                Some(tab) if tab.muted && tab.playing => format!("{} \u{2013} Muted \u{2013} {WINDOW_TITLE}", tab.title),
                Some(tab) if tab.playing => format!("{} \u{2013} Audio playing \u{2013} {WINDOW_TITLE}", tab.title),
                Some(tab) => format!("{} \u{2013} {WINDOW_TITLE}", tab.title),
                None => WINDOW_TITLE.to_owned(),
            };
            graphics.window.set_title(&title);
        }
        dock_menu::set(tabs);
    }

    /// Makes window `id` the one at hand; false when there is none.
    pub(super) fn switch_view(&mut self, id: u32) -> bool {
        if id == self.window {
            return true;
        }
        let Some(mut other) = self.parked.remove(&id) else { return false };
        std::mem::swap(&mut self.view, &mut other);
        self.parked.insert(self.window, other);
        self.window = id;
        true
    }

    /// Opens window `id`, as the trusted core asked: at its kept place, or
    /// where a tab was let go to make it, or where the system puts it.
    pub(super) fn open_another(&mut self, event_loop: &ActiveEventLoop, id: u32, geometry: Option<WindowGeometry>) {
        if id == self.window || self.parked.contains_key(&id) {
            return;
        }
        let torn = self.torn_at.take();
        let kept = geometry.or(torn).filter(|kept| on_a_display(event_loop, kept));
        let previous = std::mem::replace(&mut self.view, View::new());
        self.parked.insert(self.window, previous);
        self.window = id;
        self.create_window(event_loop, kept);
    }

    /// Closes window `id`, as the trusted core asked.
    pub(super) fn close_view(&mut self, id: u32) {
        if id == self.window {
            let Some(&other) = self.parked.keys().next() else { return };
            self.switch_view(other);
        }
        if let Some(view) = self.parked.remove(&id)
            && let Some(graphics) = &view.graphics
        {
            self.windows.remove(&graphics.window.id());
        }
        if self.focused == id {
            self.focused = self.window;
        }
    }

    /// A tab let go off its window's strip at `(x, y)` of that window: onto
    /// another window's strip, it joins that window there; anywhere else it
    /// opens a window of its own, unless it is its window's only tab.
    pub(super) fn tab_out(&mut self, index: usize, x: i32, y: i32) -> Option<ToCore> {
        let tab = u8::try_from(index).ok()?;
        let origin = self.view.graphics.as_ref()?.window.inner_position().ok()?;
        let scale = self.view.graphics.as_ref()?.window.scale_factor();
        let origin: LogicalPosition<f64> = origin.to_logical(scale);
        let (screen_x, screen_y) = (origin.x + f64::from(x), origin.y + f64::from(y));
        for (&id, view) in &self.parked {
            let Some(graphics) = &view.graphics else { continue };
            let scale = graphics.window.scale_factor();
            let Ok(position) = graphics.window.inner_position() else { continue };
            let position: LogicalPosition<f64> = position.to_logical(scale);
            let point = interface_core::geometry::Point::new(screen_x - position.x, screen_y - position.y);
            if let Some(place) = view.browser.tab_place_at(point) {
                return Some(ToCore::TabToWindow { tab, window: id, index: u8::try_from(place).unwrap_or(u8::MAX) });
            }
        }
        if self.view.tabs.len() <= 1 {
            return None;
        }
        // The new window's strip comes under the pointer, as a tab torn off
        // in a web browser does.
        let size = self.view.reported.map_or((1, 1), |reported| (reported.width, reported.height));
        #[allow(clippy::cast_possible_truncation, reason = "a point on the screen, rounded; checked plausible before use")]
        let corner = ((screen_x - TORN_OFFSET).round() as i32, (screen_y - TORN_OFFSET).round() as i32);
        self.torn_at = Some(WindowGeometry { x: corner.0, y: corner.1, width: size.0, height: size.1, maximised: false }).filter(WindowGeometry::is_plausible);
        Some(ToCore::TabToWindow { tab, window: protocol::NEW_WINDOW, index: 0 })
    }

    pub(super) fn window_action(&mut self, event_loop: &ActiveEventLoop, action: WindowAction) {
        if action == WindowAction::Close {
            // The trusted core closes the window and its tabs, or, for the
            // last window, ends the browser (`multiple-windows.md`).
            self.send(event_loop, &ToCore::CloseWindow);
            return;
        }
        let Some(graphics) = &self.view.graphics else { return };
        let window = &graphics.window;
        match action {
            WindowAction::Drag => {
                let _ = window.drag_window();
            }
            WindowAction::Minimise => window.set_minimized(true),
            WindowAction::Maximise => window.set_maximized(!window.is_maximized()),
            WindowAction::Close => {}
        }
    }

    /// The window follows the browser into full screen, the page alone,
    /// and out (decision 85): borderless on its display, on macOS a
    /// full-screen space of its own. A window full screen already, by the
    /// system's green button, stays so when the page's full screen ends.
    pub(super) fn sync_full_screen(&mut self) {
        let wanted = self.view.browser.full_screen();
        if wanted == self.view.page_full_screen.is_some() {
            return;
        }
        let Some(graphics) = &self.view.graphics else { return };
        let window = &graphics.window;
        if wanted {
            let full = window.fullscreen().is_some();
            if !full {
                window.set_fullscreen(Some(Fullscreen::Borderless(None)));
            }
            // A window the system made full screen for this, by the green
            // button or Fn-F, is left with the page's full screen; one that
            // was full screen before stays so (REQ-6362 item 3).
            let already = full && self.view.window_full != WindowFull::EnteredForPage;
            if self.view.window_full == WindowFull::EnteredForPage {
                self.view.window_full = WindowFull::Yes;
            }
            self.view.page_full_screen = Some((already, full));
        } else {
            if self.view.page_full_screen.is_some_and(|(already, _)| !already) {
                window.set_fullscreen(None);
            }
            self.view.page_full_screen = None;
        }
        dock_menu::set_full_screen(wanted);
    }

    /// The window changed size: in the page's full screen, the system's
    /// leaving it (its key, Mission Control, a display unplugged) ends it.
    pub(super) fn full_screen_resized(&mut self) {
        let Some(graphics) = &self.view.graphics else { return };
        let full = graphics.window.fullscreen().is_some();
        // macOS's own ways in, the green button and Fn-F, put the window
        // into the system's full screen: the page's full screen follows,
        // when the window has a page to show (REQ-6362 item 3).
        let entered_by_system = cfg!(target_os = "macos") && full && self.view.window_full == WindowFull::No && self.view.page_full_screen.is_none();
        self.view.window_full = if full { WindowFull::Yes } else { WindowFull::No };
        if entered_by_system && !self.view.browser.full_screen() {
            let now = self.now();
            if self.view.browser.toggle_full_screen(now) {
                self.view.window_full = WindowFull::EnteredForPage;
                self.sync_full_screen();
                self.request_redraw();
            }
            return;
        }
        let Some((already, seen)) = self.view.page_full_screen else { return };
        if full {
            self.view.page_full_screen = Some((already, true));
        } else if seen {
            self.view.page_full_screen = None;
            self.view.browser.leave_full_screen();
            dock_menu::set_full_screen(false);
            self.request_redraw();
        }
    }

    /// The edge or corner of the window the pointer at `x`, `y` is on, on
    /// platforms where the browser draws its own frame: where a press
    /// resizes the window. None while the window is maximised or full
    /// screen, which have no edge to drag.
    pub(super) fn edge_at(&self, x: f64, y: f64) -> Option<ResizeDirection> {
        if !FRAME.window_buttons {
            return None;
        }
        let graphics = self.view.graphics.as_ref()?;
        let frame = self.window_frame()?;
        if frame.maximized || frame.full_screen {
            return None;
        }
        let size = frame.size.to_logical::<f64>(graphics.window.scale_factor());
        resize_edge(x, y, size.width, size.height)
    }

    /// The window at hand's size and state, asked of the window system
    /// only when it was resized, moved or rescaled since last asked.
    pub(super) fn window_frame(&self) -> Option<super::WindowFrame> {
        if let Some(frame) = self.view.window_frame.get() {
            return Some(frame);
        }
        let window = &self.view.graphics.as_ref()?.window;
        let frame = super::WindowFrame { size: window.inner_size(), maximized: window.is_maximized(), full_screen: window.fullscreen().is_some() };
        self.view.window_frame.set(Some(frame));
        Some(frame)
    }

    /// A press at the window's edge resizes it. Returns whether it did.
    pub(super) fn resize_from_edge(&self, x: f64, y: f64) -> bool {
        let (Some(direction), Some(graphics)) = (self.edge_at(x, y), &self.view.graphics) else { return false };
        graphics.window.drag_resize_window(direction).is_ok()
    }

    /// The open tab moved to a window of its own.
    pub(super) fn tab_to_new_window(&self) -> Option<ToCore> {
        let tab = u8::try_from(self.view.active).ok()?;
        Some(ToCore::TabToWindow { tab, window: protocol::NEW_WINDOW, index: 0 })
    }

    /// Opens the window once the event loop has resumed and the trusted
    /// core has spoken.
    pub(super) fn open_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.view.graphics.is_some() || !self.resumed {
            return;
        }
        let kept = match self.heard {
            Heard::Nothing => return,
            Heard::Spoke => None,
            Heard::Window(kept) => Some(kept).filter(|kept| on_a_display(event_loop, kept)),
        };
        self.create_window(event_loop, kept);
    }

    /// Opens the window at hand's window, at `kept` when there is one.
    fn create_window(&mut self, event_loop: &ActiveEventLoop, kept: Option<WindowGeometry>) {
        let Ok(window) = event_loop.create_window(attributes(kept)) else {
            self.exit = Some("cannot open a window".into());
            event_loop.exit();
            return;
        };
        let window = Rc::new(window);
        let Ok(context) = Context::new(window.clone()) else {
            self.exit = Some("cannot reach the display".into());
            event_loop.exit();
            return;
        };
        let Ok(surface) = Surface::new(&context, window.clone()) else {
            self.exit = Some("cannot draw into the window".into());
            event_loop.exit();
            return;
        };
        self.windows.insert(window.id(), self.window);
        self.view.wheel_pointers = wheel_pointers(event_loop, window.scale_factor());
        self.view.graphics = Some(Graphics { window, surface, _context: context, asked: false });
        self.follow_reduced_motion();
        self.show_tabs();
        self.report_window(event_loop);
    }

    /// Tells the trusted core where the window is and how large, to keep
    /// with the session (requirement 20.21); while maximised, the place and
    /// size it returns to.
    pub(super) fn report_window(&mut self, event_loop: &ActiveEventLoop) {
        let Some(now) = self.view.graphics.as_ref().and_then(|graphics| geometry(&graphics.window)) else { return };
        let report = if now.maximised {
            self.view.normal.map_or(now, |normal| WindowGeometry { maximised: true, ..normal })
        } else {
            self.view.normal = Some(now);
            now
        };
        if self.view.reported != Some(report) {
            self.view.reported = Some(report);
            self.send(event_loop, &ToCore::Window(report));
        }
    }
}

/// The wheel pointer's pictures, for a display of `scale` pixels to a
/// point: every state, each with its wheel lit and unlit, drawn by the
/// interface's own code and handed to the windowing system once (REQ-6356
/// item 6). One the system refuses is left out, and the window then shows
/// the system's own shape there.
fn wheel_pointers(
    event_loop: &ActiveEventLoop,
    scale: f64,
) -> std::collections::HashMap<(interface_core::wheel_cursor::WheelCursor, bool), winit::window::CustomCursor> {
    use interface_core::wheel_cursor::{Ground, HOTSPOT, MAX_SCALE, SIZE, WheelCursor, image};
    // A display's scale, upwards to a whole number within the pictures' bound.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to the bound first")]
    let scale = scale.ceil().clamp(1.0, f64::from(MAX_SCALE)) as u16;
    let (side, hotspot) = (SIZE.saturating_mul(scale), HOTSPOT.saturating_mul(scale));
    let mut pointers = std::collections::HashMap::new();
    for ground in [Ground::Dark, Ground::Light] {
        for (up, down) in [(true, true), (true, false), (false, true)] {
            for lit in [true, false] {
                let state = WheelCursor { up, down, ground };
                if let Ok(source) = winit::window::CustomCursor::from_rgba(image(state, lit, scale), side, side, hotspot, hotspot) {
                    pointers.insert((state, lit), event_loop.create_custom_cursor(source));
                }
            }
        }
    }
    pointers
}

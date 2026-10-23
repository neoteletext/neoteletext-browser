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
use winit::window::ResizeDirection;

use crate::window::{FRAME, RESIZE_MARGIN, TORN_OFFSET, WINDOW_TITLE, attributes, geometry, on_a_display};

use super::{App, Graphics, Heard, View};

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

    /// On platforms where the browser draws its own frame, a press at the
    /// window's edge resizes it. Returns whether it did.
    pub(super) fn resize_from_edge(&self, x: f64, y: f64) -> bool {
        if !FRAME.window_buttons {
            return false;
        }
        let Some(graphics) = &self.view.graphics else { return false };
        let size = graphics.window.inner_size().to_logical::<f64>(graphics.window.scale_factor());
        let (left, right) = (x < RESIZE_MARGIN, x > size.width - RESIZE_MARGIN);
        let (top, bottom) = (y < RESIZE_MARGIN, y > size.height - RESIZE_MARGIN);
        let direction = match (left, right, top, bottom) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (_, _, _, true) => ResizeDirection::South,
            _ => return false,
        };
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
        self.view.graphics = Some(Graphics { window, surface, _context: context, asked: false });
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

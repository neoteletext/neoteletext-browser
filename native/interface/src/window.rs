//! The system window: its title, size and bounds, the frame the browser
//! draws on each platform, and its place and size as the session keeps
//! them.

use interface_core::browser::Frame;
use interface_core::library::WindowGeometry;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

pub(crate) const WINDOW_TITLE: &str = "NeoTeletext";
/// The windows' application name on Linux, X11's class and instance and
/// Wayland's application identifier, which name the menu entry the
/// launcher installs, and with it the icon, for every window
/// (`StartupWMClass`; decision 124).
#[cfg(target_os = "linux")]
const APPLICATION_NAME: &str = "neoteletext";

// The art, made into the icon this platform's windows or Dock show when
// the interface was built (`build.rs`, decision 124).
include!(concat!(env!("OUT_DIR"), "/icon.rs"));
const WINDOW_WIDTH: f64 = 1440.0;
/// How far up and left of the pointer a window torn off from a tab opens,
/// in points, so that its strip is under the pointer.
pub(crate) const TORN_OFFSET: f64 = 20.0;
const WINDOW_HEIGHT: f64 = 900.0;
const MINIMUM_WIDTH: f64 = 1000.0;
const MINIMUM_HEIGHT: f64 = 640.0;
/// How far in from the window's edge a press resizes it, on platforms
/// where the browser draws its own frame.
pub(crate) const RESIZE_MARGIN: f64 = 6.0;
/// On macOS the system's window buttons sit in the tab strip, as in
/// Chrome; they take this much of its left end.
#[cfg(target_os = "macos")]
pub(crate) const FRAME: Frame = Frame { left_inset: 72.0, window_buttons: false };
/// Elsewhere the browser draws the whole frame, as Chrome does.
#[cfg(not(target_os = "macos"))]
pub(crate) const FRAME: Frame = Frame { left_inset: 0.0, window_buttons: true };

/// The windows' icon (decision 124), which Windows shows in the title bar
/// and the taskbar and X11 gives the window manager; macOS takes the
/// bundle's, and Wayland the menu entry's.
#[cfg(not(target_os = "macos"))]
fn icon() -> Option<winit::window::Icon> {
    winit::window::Icon::from_rgba(WINDOW_ICON.to_vec(), WINDOW_ICON_SIDE, WINDOW_ICON_SIDE).ok()
}

/// macOS: the Dock's icon, for the browser run outside its bundle, as
/// from `cargo run`; a bundle's own icon is left as it is (decision 124).
#[cfg(target_os = "macos")]
pub fn show_dock_icon() {
    dock_menu::set_icon(DOCK_ICON);
}

/// Elsewhere there is no Dock.
#[cfg(not(target_os = "macos"))]
pub fn show_dock_icon() {}

/// The window, at its kept place and size when there are some.
pub(crate) fn attributes(kept: Option<WindowGeometry>) -> winit::window::WindowAttributes {
    let mut attributes = Window::default_attributes()
        .with_title(WINDOW_TITLE)
        .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .with_min_inner_size(LogicalSize::new(MINIMUM_WIDTH, MINIMUM_HEIGHT));
    if let Some(kept) = kept {
        attributes = attributes
            .with_position(LogicalPosition::new(kept.x, kept.y))
            .with_inner_size(LogicalSize::new(kept.width, kept.height))
            .with_maximized(kept.maximised);
    }
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS;
        attributes.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true)
    }
    #[cfg(target_os = "windows")]
    let attributes = winit::platform::windows::WindowAttributesExtWindows::with_taskbar_icon(attributes, icon());
    #[cfg(target_os = "linux")]
    let attributes = winit::platform::x11::WindowAttributesExtX11::with_name(attributes, APPLICATION_NAME, APPLICATION_NAME);
    #[cfg(not(target_os = "macos"))]
    attributes.with_decorations(false).with_window_icon(icon())
}

/// Whether the kept window's top left corner falls on a display now: one
/// unplugged since is not used, and the system places the window
/// (`history-and-bookmarks.md` section 15.3).
pub(crate) fn on_a_display(event_loop: &ActiveEventLoop, kept: &WindowGeometry) -> bool {
    event_loop.available_monitors().any(|monitor| {
        let scale = monitor.scale_factor();
        let position: LogicalPosition<f64> = monitor.position().to_logical(scale);
        let size: LogicalSize<f64> = monitor.size().to_logical(scale);
        let (x, y) = (f64::from(kept.x), f64::from(kept.y));
        x >= position.x && x < position.x + size.width && y >= position.y && y < position.y + size.height
    })
}

/// The window's place and size in logical points, as the session keeps
/// them; none where the system does not say where the window is.
pub(crate) fn geometry(window: &Window) -> Option<WindowGeometry> {
    let scale = window.scale_factor();
    let position: LogicalPosition<f64> = window.outer_position().ok()?.to_logical(scale);
    let size: LogicalSize<f64> = window.inner_size().to_logical(scale);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "rounded, then checked within the session's bounds")]
    let geometry = WindowGeometry {
        x: position.x.round() as i32,
        y: position.y.round() as i32,
        width: size.width.round() as u32,
        height: size.height.round() as u32,
        maximised: window.is_maximized(),
    };
    geometry.is_plausible().then_some(geometry)
}

#[cfg(test)]
mod tests;

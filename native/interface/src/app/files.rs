//! Local files as the interface meets them (REQ-1700, REQ-1701, REQ-1703):
//! it never opens one. It groups what is dropped, tells the broker on its own
//! channel before the trusted core hears of it, so that no read can come
//! first, and sends the trusted core one intent.

use std::path::PathBuf;

use confine::broker::{Declaration, Declarer};
use interface_core::browser::{DropPlace, DropTarget};
use protocol::{MAX_FILES_AT_ONCE, ToCore};
use winit::event_loop::ActiveEventLoop;

use super::App;

/// What the file address of `path` is.
fn file_address(path: &str) -> String {
    format!("file://{}", if cfg!(windows) { format!("/{}", path.replace('\\', "/")) } else { path.to_owned() })
}

impl App {
    /// Tells the broker the user named `paths`, and returns once it has
    /// taken them. A private tab's are for the run alone. A channel that
    /// fails is given up: reads of what it would have declared are refused.
    pub(super) fn declare(&mut self, paths: &[String]) {
        let keep = !self.view.browser.is_private();
        if let Some(declarer) = &mut self.declarer
            && let Err(error) = declarer.declare(&Declaration { paths: paths.to_vec(), keep })
        {
            eprintln!("neoteletext-interface: the broker did not take what was declared: {error}");
            self.declarer = None;
        }
    }

    /// One file of a drop. A drop comes as one event a file, so they are
    /// kept until the events at hand are done. Linux takes no drops
    /// (DEC-1551 item 4).
    pub(super) fn file_dropped(&mut self, path: PathBuf) {
        if !cfg!(target_os = "linux") {
            self.dropped.push(path);
        }
    }

    /// What lies under a drop. winit's events do not follow a drag from
    /// another application, so the place comes from the system on macOS;
    /// where it cannot be known a drop is on the page, which replaces no tab.
    fn drop_target(&self) -> DropTarget {
        #[cfg(target_os = "macos")]
        if let Some(graphics) = &self.view.graphics
            && let Ok(handle) = winit::raw_window_handle::HasWindowHandle::window_handle(&*graphics.window)
            && let winit::raw_window_handle::RawWindowHandle::AppKit(view) = handle.as_raw()
            && let Some((x, y)) = dock_menu::pointer_in_view(view.ns_view.as_ptr() as usize)
        {
            return self.view.browser.drop_target(interface_core::geometry::Point::new(x, y));
        }
        DropTarget::Tabs(DropPlace::NewTabs)
    }

    /// The events at hand are done: a drop among them goes on as one.
    pub(super) fn drop_done(&mut self, event_loop: &ActiveEventLoop) {
        if self.dropped.is_empty() {
            return;
        }
        let dropped = std::mem::take(&mut self.dropped);
        let mut files: Vec<String> = dropped.iter().filter_map(|path| path.to_str().map(str::to_owned)).collect();
        let left_out = dropped.len().saturating_sub(files.len().min(MAX_FILES_AT_ONCE));
        files.truncate(MAX_FILES_AT_ONCE);
        match self.drop_target() {
            DropTarget::Address => {
                if let Some(first) = files.first() {
                    self.view.browser.drop_on_address(&file_address(first));
                    self.act(event_loop);
                }
            }
            DropTarget::Tabs(place) => {
                self.declare(&files);
                self.send(event_loop, &ToCore::Dropped { files, place, left_out: u16::try_from(left_out).unwrap_or(u16::MAX) });
            }
        }
    }

    /// macOS asked the application to open these: files and links.
    pub(super) fn opened_by_system(&mut self, event_loop: &ActiveEventLoop, mut opened: Vec<String>) {
        opened.truncate(MAX_FILES_AT_ONCE);
        let files: Vec<String> = opened.iter().filter(|text| interface_core::browser::names_a_file(text)).cloned().collect();
        if !files.is_empty() {
            self.declare(&files);
        }
        self.send(event_loop, &ToCore::FromSystem(opened));
    }

    /// The window to place the file dialog over, as the chooser reads it: an
    /// X window's number, which means the same in any process. Windows and
    /// Wayland need calls this crate may not make, and macOS places the
    /// panel itself.
    #[cfg_attr(not(target_os = "linux"), allow(clippy::unused_self, reason = "only an X window has a number to give"))]
    pub(super) fn window_for_dialog(&self) -> String {
        #[cfg(target_os = "linux")]
        if let Some(graphics) = &self.view.graphics
            && let Ok(handle) = winit::raw_window_handle::HasWindowHandle::window_handle(&*graphics.window)
            && let winit::raw_window_handle::RawWindowHandle::Xlib(window) = handle.as_raw()
        {
            return format!("x11:{:x}", window.window);
        }
        String::new()
    }
}

/// The interface's own channel to the broker, which a test does not have.
pub(super) fn declarer() -> Option<Declarer> {
    Declarer::from_parent().map_err(|error| eprintln!("neoteletext-interface: {error}")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_address_is_written_as_the_address_rules_read_it() {
        if !cfg!(windows) {
            assert_eq!(file_address("/a b/c.tti"), "file:///a b/c.tti");
        }
    }
}

//! Frames: drawing the window at hand and presenting only what changed,
//! a page image read against the one held, the pointer's shape, and the
//! pace of frames.

use std::io::Write;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use interface_core::browser::Cursor;
use interface_core::canvas::PageKey;
use protocol::ToCore;
use protocol::frame::{self, FrameError};
use winit::event_loop::ActiveEventLoop;
use winit::window::CursorIcon;

use super::App;

/// The shortest time between frames that only show what the trusted core
/// sent: ten a second.
const CHANGE_FRAME_MILLISECONDS: u64 = 100;
/// One frame of a 60 hertz display: what a channel being typed into
/// pushes is drawn at the next (`interaction.md` section 10).
#[cfg(feature = "interaction")]
const TYPING_FRAME_MILLISECONDS: u64 = 16;

impl App {
    pub(super) fn request_redraw(&mut self) {
        if let Some(graphics) = &mut self.view.graphics {
            graphics.asked = true;
            graphics.window.request_redraw();
        }
    }

    pub(super) fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let now = self.now();
        let Some(graphics) = self.view.graphics.as_mut() else { return };
        let size = graphics.window.inner_size();
        let (Ok(width), Ok(height)) = (u16::try_from(size.width), u16::try_from(size.height)) else { return };
        let (Some(surface_width), Some(surface_height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if graphics.surface.resize(surface_width, surface_height).is_err() {
            return;
        }
        self.view.last_frame = now;
        self.view.dirty = false;
        if !std::mem::take(&mut graphics.asked) {
            self.view.painter.redraw_whole();
        }
        let started = Instant::now();
        self.view.painter.begin(width, height, graphics.window.scale_factor());
        let logical = (f64::from(width) / self.view.painter.scale, f64::from(height) / self.view.painter.scale);
        self.view.browser.draw(&mut self.view.painter, logical.0, logical.1, now);
        // Only what changed since the last frame is drawn; a frame that
        // changed nothing is not presented, and the window keeps the last.
        let damage = self.view.painter.render();
        if damage.is_empty() {
            if self.trace_frames {
                eprintln!("neoteletext-interface: frame at {now} ms: drawn {:?}, unchanged", started.elapsed());
            }
            self.act(event_loop);
            return;
        }
        let drawn = started.elapsed();
        let Ok(mut buffer) = graphics.surface.buffer_mut() else { return };
        let age = buffer.age();
        self.view.painter.copy_to(&mut buffer, &damage, age);
        let copied = started.elapsed();
        graphics.window.pre_present_notify();
        let rects: Vec<softbuffer::Rect> = damage
            .boxes()
            .iter()
            .filter_map(|area| {
                let (width, height) = (NonZeroU32::new(u32::from(area.width()))?, NonZeroU32::new(u32::from(area.height()))?);
                Some(softbuffer::Rect { x: u32::from(area.x0), y: u32::from(area.y0), width, height })
            })
            .collect();
        let presented = buffer.present_with_damage(&rects).is_ok();
        // The first frame on the screen is a part of the version's health
        // (`installation-and-updates.md` section 3.2), said once.
        if presented && self.first_frame.is_none() {
            self.first_frame = Some(now);
            self.send(event_loop, &ToCore::Drawn);
            let _ = self.output.flush();
        }
        if self.trace_frames {
            let presented = started.elapsed();
            let pixels: u32 = damage.boxes().iter().map(|area| area.area()).sum();
            eprintln!(
                "neoteletext-interface: frame at {now} ms: drawn {:?}, rendered {:?}, presented {:?}, {width}x{height}, damage {} boxes, {pixels} pixels, age {age}",
                drawn,
                copied.saturating_sub(drawn),
                presented.saturating_sub(copied),
                damage.boxes().len()
            );
        }
        self.act(event_loop);
    }

    /// Sets the window's pointer shape to the browser's, when it changed.
    pub(super) fn update_cursor(&mut self) {
        let cursor = self.view.browser.cursor();
        if cursor == self.view.pointer_shape {
            return;
        }
        self.view.pointer_shape = cursor;
        if let Some(graphics) = &self.view.graphics {
            graphics.window.set_cursor(match cursor {
                Cursor::Arrow => CursorIcon::Default,
                Cursor::Hand => CursorIcon::Pointer,
                // A link that leaves the channel: the shape of a shortcut.
                Cursor::External => CursorIcon::Alias,
            });
        }
    }

    /// Asks for the window at hand's next frame when it is due, and says
    /// when the one after is: none while nothing in it animates or waits to
    /// be drawn.
    pub(super) fn pace(&mut self, event_loop: &ActiveEventLoop) -> Option<Instant> {
        let now = self.now();
        if self.view.browser.settle(now) {
            // What settling asked for, as a held Back's list, goes now.
            self.act(event_loop);
            if let Err(error) = self.output.flush() {
                self.exit = Some(format!("cannot write to the trusted core: {error}"));
                event_loop.exit();
                return None;
            }
            self.request_redraw();
        }
        // While keys go to a channel, what it pushes is drawn at the
        // display's rate (`interaction.md` section 10).
        #[cfg(feature = "interaction")]
        let change = if self.view.browser.typing_target().is_some() { TYPING_FRAME_MILLISECONDS } else { CHANGE_FRAME_MILLISECONDS };
        #[cfg(not(feature = "interaction"))]
        let change = CHANGE_FRAME_MILLISECONDS;
        let interval = match (self.view.browser.frame_interval(now), self.view.dirty) {
            (Some(interval), true) => Some(interval.min(change)),
            (Some(interval), false) => Some(interval),
            (None, true) => Some(change),
            (None, false) => None,
        }?;
        // Frames at the pace the browser asks for, rather than as fast as
        // the processor allows.
        if now.saturating_sub(self.view.last_frame) >= interval {
            self.request_redraw();
        }
        self.start.checked_add(Duration::from_millis(self.view.last_frame.saturating_add(interval)))
    }

    /// Reads a page's frame against the one held and shows it; false when
    /// it is not shown: changes to a page no longer held, or a broken frame,
    /// which ends the interface.
    pub(super) fn show_frame(&mut self, event_loop: &ActiveEventLoop, key: PageKey, version: u32, image: &[u8]) -> bool {
        let held = self.view.held.get(&key).map(|(version, indices)| (*version, indices.as_slice()));
        let image = match frame::decode(image, held) {
            Ok(image) => image,
            // Changes to a page this window stopped showing while
            // they were on their way: the trusted core sends the
            // page whole when it is shown again.
            Err(FrameError::Base) if held.is_none() => return false,
            Err(error) => {
                self.exit = Some(format!("bad page image from the trusted core: {error}"));
                event_loop.exit();
                return false;
            }
        };
        if let Err(error) = self.view.painter.set_page(key, &image.palette, image.screen, &image.indices) {
            self.exit = Some(format!("bad page image: {error:?}"));
            event_loop.exit();
            return false;
        }
        self.view.held.insert(key, (version, image.indices));
        self.view.browser.frame_version(key, version);
        true
    }
}

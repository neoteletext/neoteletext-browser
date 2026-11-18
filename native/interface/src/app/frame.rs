//! Frames: drawing the window at hand and presenting only what changed,
//! a page image read against the one held, the pointer's shape, and the
//! pace of frames (`pace.rs`).
//!
//! Presentation keeps at most one frame queued on every platform
//! (requirement 18.14, REQ-RCH-004): drawing is on the processor
//! (`vello_cpu`), with no swap chain, and softbuffer 0.4.8 presents
//! synchronously. On macOS it sets the layer's contents to the frame in a
//! Core Animation transaction, which the next refresh shows, a newer frame
//! replacing one not yet shown; on Wayland it attaches one of two shared
//! memory buffers and commits, waiting before drawing into the other until
//! the compositor has released it, and winit, told before each present,
//! asks for the next redraw only once the compositor's frame callback has
//! come; on X11 its next frame waits for the server to finish the last
//! shared-memory `PutImage`, or goes in the request itself without shared
//! memory, which the server handles in order; on Windows `BitBlt` copies it into the window at once, for
//! the compositor to show at its next refresh.

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use interface_core::browser::Cursor;
use interface_core::canvas::PageKey;
use protocol::ToCore;
use protocol::frame::{self, FrameError};
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::CursorIcon;

use super::App;

/// The shortest time between frames that only show what the trusted core
/// sent, counted from the last frame that changed the window: ten a
/// second. A frame of the page under the pointer is drawn as it arrives.
const CHANGE_FRAME_MILLISECONDS: u64 = 100;
/// One frame of a 60 hertz display: what else the trusted core sends while
/// a channel is typed into is drawn at the next (`interaction.md` section
/// 10); the channel's page itself is drawn as its frames arrive.
#[cfg(feature = "interaction")]
const TYPING_FRAME_MILLISECONDS: u64 = 16;

impl App {
    pub(super) fn request_redraw(&mut self) {
        if let Some(graphics) = &mut self.view.graphics {
            graphics.asked = true;
            // The timed test's paced display has a frame asked for drawn
            // at its next composition, as Wayland's frame callback paces
            // winit's redraws (RULE-66, DEC-6659 item 25).
            #[cfg(feature = "timed-test")]
            if self.test_display.defer() {
                return;
            }
            graphics.window.request_redraw();
        }
    }

    /// The timed test's display (RULE-66): its phase is told the trusted
    /// core as a platform's frame times are (RULE-98 item 1), unless it is
    /// the paced display, whose phase the interface derives from its own
    /// redraws as the shipped interface does; and a paced display's frames
    /// owed are asked for at its compositions. When the loop is to wake
    /// for it.
    #[cfg(feature = "timed-test")]
    pub(super) fn tell_test_display(&mut self, event_loop: &ActiveEventLoop) -> Instant {
        let now = Instant::now();
        if let Some(phase) = self.test_display.phase_due(now) {
            protocol::timed_test::mark("interface", "true-phase", now, format_args!("{} {}", phase.period_us, phase.next_in_us));
            if !self.test_display.paced() {
                protocol::timed_test::mark("interface", "phase", now, format_args!("{} {}", phase.period_us, phase.next_in_us));
                self.send(event_loop, &ToCore::FramePhase(phase));
                let _ = self.output.handled();
            }
        }
        if self.test_display.composition_reached(now) {
            let here = self.window;
            for id in self.window_ids() {
                self.switch_view(id);
                if let Some(graphics) = &mut self.view.graphics
                    && graphics.asked
                {
                    graphics.window.request_redraw();
                }
            }
            self.switch_view(here);
        }
        self.test_display.wake(Instant::now())
    }

    /// RULE-98 item 1, DEC-6659 item 25: the phase the interface's own
    /// redraws give, told when it is due; when the loop is to wake to tell
    /// it again. The timed test's own display tells its phase itself, and
    /// the derived one is only marked, to be measured against it, but for
    /// the paced display.
    pub(super) fn tell_present_phase(&mut self, event_loop: &ActiveEventLoop) -> Option<Instant> {
        let now = Instant::now();
        if let Some(phase) = self.present_phase.phase_due(now) {
            #[cfg(feature = "timed-test")]
            {
                protocol::timed_test::mark("interface", "derived-phase", now, format_args!("{} {}", phase.period_us, phase.next_in_us));
                if self.test_display.paced() {
                    protocol::timed_test::mark("interface", "phase", now, format_args!("{} {}", phase.period_us, phase.next_in_us));
                    self.send(event_loop, &ToCore::FramePhase(phase));
                    let _ = self.output.handled();
                }
            }
            #[cfg(not(feature = "timed-test"))]
            {
                self.send(event_loop, &ToCore::FramePhase(phase));
                let _ = self.output.handled();
            }
        }
        self.present_phase.wake()
    }

    pub(super) fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        // RULE-66's hops: when the window's thread began the frame.
        #[cfg(feature = "timed-test")]
        protocol::timed_test::mark("interface", "redraw", Instant::now(), format_args!(""));
        let now = self.now();
        let Some(frame) = self.window_frame() else { return };
        let Some(graphics) = self.view.graphics.as_mut() else { return };
        let size = frame.size;
        let (Ok(width), Ok(height)) = (u16::try_from(size.width), u16::try_from(size.height)) else { return };
        let (Some(surface_width), Some(surface_height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if graphics.surface.resize(surface_width, surface_height).is_err() {
            return;
        }
        if !std::mem::take(&mut graphics.asked) {
            self.view.painter.redraw_whole();
        }
        let started = Instant::now();
        // RULE-98 item 1: the monitor's rate, read again after a move, and
        // the moment the window system had this frame drawn.
        if std::mem::take(&mut self.rate_unread) {
            self.present_phase.set_rate(graphics.window.current_monitor().and_then(|monitor| monitor.refresh_rate_millihertz()));
        }
        self.present_phase.presented(started);
        // Which ReGIS pages show their row of prompts follows their links (REQ-6379).
        self.view.painter.set_regis_keyed(self.view.browser.regis_keyed());
        self.view.painter.begin(width, height, graphics.window.scale_factor());
        let logical = (f64::from(width) / self.view.painter.scale, f64::from(height) / self.view.painter.scale);
        self.view.browser.set_window_maximised(frame.maximized);
        self.view.browser.draw(&mut self.view.painter, logical.0, logical.1, now);
        self.view.browser.draw_window_border(&mut self.view.painter, logical.0, logical.1);
        // The input method's candidates sit beside the caret of the field
        // with the keys (REQ-763), told again only when it moved.
        if self.view.ime.allowed
            && let Some(caret) = self.view.browser.caret_area()
            && self.view.ime.caret != Some(caret)
        {
            graphics.window.set_ime_cursor_area(LogicalPosition::new(caret.x0, caret.y0), LogicalSize::new(caret.width(), caret.height()));
            self.view.ime.caret = Some(caret);
        }
        // Only what changed since the last frame is drawn; a frame that
        // changed nothing is not presented, and the window keeps the last.
        #[cfg(feature = "timed-test")]
        protocol::timed_test::mark("interface", "scene", Instant::now(), format_args!(""));
        let damage = self.view.painter.render();
        #[cfg(feature = "timed-test")]
        protocol::timed_test::mark("interface", "rendered", Instant::now(), format_args!(""));
        // A frame that changed nothing delays no frame after it
        // (REQ-RCH-001).
        self.view.pace.drawn(now, !damage.is_empty());
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
        #[cfg(feature = "timed-test")]
        protocol::timed_test::mark("interface", "copied", Instant::now(), format_args!(""));
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
        // RULE-66: when the frame was presented, the composition of the
        // test's display that shows it, and what it changed.
        #[cfg(feature = "timed-test")]
        {
            let boxes: Vec<String> = damage.boxes().iter().map(|area| format!("{},{},{},{}", area.x0, area.y0, area.x1, area.y1)).collect();
            let presented_at = Instant::now();
            protocol::timed_test::mark("interface", "presented", presented_at, format_args!(""));
            protocol::timed_test::mark("interface", "composed", self.test_display.next_composition(presented_at), format_args!("{}", boxes.join(";")));
        }
        // The first frame on the screen is a part of the version's health
        // (`installation-and-updates.md` section 3.2), said once.
        if presented && self.first_frame.is_none() {
            self.first_frame = Some(now);
            self.send(event_loop, &ToCore::Drawn);
            let _ = self.output.handled();
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

    /// Sets the window's pointer shape to the browser's, when it changed,
    /// and hides it as full screen does a still one.
    pub(super) fn update_cursor(&mut self) {
        // RULE-66's hops: what the window system was asked here.
        #[cfg(feature = "timed-test")]
        protocol::timed_test::mark("interface", "cursor-begin", Instant::now(), format_args!(""));
        let hidden = self.view.browser.pointer_hidden(self.now());
        if hidden != self.view.pointer_hidden {
            self.view.pointer_hidden = hidden;
            if let Some(graphics) = &self.view.graphics {
                graphics.window.set_cursor_visible(!hidden);
                #[cfg(feature = "timed-test")]
                protocol::timed_test::mark("interface", "cursor-visible", Instant::now(), format_args!(""));
            }
        }
        // At an edge of a window the browser frames itself, the pointer
        // shows which way a press there resizes it: nothing else does.
        let edge = self.edge_at(self.view.cursor.0, self.view.cursor.1);
        if edge != self.view.pointer_edge {
            self.view.pointer_edge = edge;
            // Off the edge again, the browser's own shape is set afresh.
            self.view.pointer_shape = (Cursor::Arrow, false);
            if let Some(graphics) = &self.view.graphics {
                graphics.window.set_cursor(edge.map_or(CursorIcon::Default, CursorIcon::from));
                #[cfg(feature = "timed-test")]
                protocol::timed_test::mark("interface", "cursor-edge", Instant::now(), format_args!(""));
            }
        }
        if edge.is_some() {
            return;
        }
        let cursor = self.view.browser.cursor();
        // The wheel pointer's wheel blinks: its picture is one of two (REQ-6356 item 3).
        let lit = matches!(cursor, Cursor::Wheel(_)) && self.view.browser.wheel_lit(self.now());
        if (cursor, lit) == self.view.pointer_shape {
            return;
        }
        self.view.pointer_shape = (cursor, lit);
        let Some(graphics) = &self.view.graphics else { return };
        let icon = match cursor {
            Cursor::Arrow => CursorIcon::Default,
            Cursor::Hand => CursorIcon::Pointer,
            // A link that leaves the channel: the shape of a shortcut.
            Cursor::External => CursorIcon::Alias,
            Cursor::ResizeColumn => CursorIcon::ColResize,
            Cursor::ResizeRow => CursorIcon::RowResize,
            Cursor::Wheel(wheel) => {
                if let Some(picture) = self.view.wheel_pointers.get(&(wheel, lit)) {
                    graphics.window.set_cursor(picture.clone());
                    #[cfg(feature = "timed-test")]
                    protocol::timed_test::mark("interface", "cursor-picture", Instant::now(), format_args!(""));
                    return;
                }
                // A platform that made no picture marks the place with its
                // own up-and-down shape (REQ-6356 item 7).
                CursorIcon::NsResize
            }
        };
        graphics.window.set_cursor(icon);
        #[cfg(feature = "timed-test")]
        protocol::timed_test::mark("interface", "cursor-icon", Instant::now(), format_args!("{icon:?}"));
    }

    /// Asks for the window at hand's next frame when it is due, and says
    /// when the one after is: none while nothing in it animates or waits to
    /// be drawn.
    pub(super) fn pace(&mut self, event_loop: &ActiveEventLoop) -> Option<Instant> {
        let now = self.now();
        if self.view.browser.settle(now) {
            // What settling asked for, as a held Back's list, goes now.
            self.act(event_loop);
            if !self.flush(event_loop) {
                return None;
            }
            self.request_redraw();
        }
        // While keys go to a channel, what else changes is drawn at the
        // display's rate (`interaction.md` section 10).
        #[cfg(feature = "interaction")]
        let change = if self.view.browser.typing_target().is_some() { TYPING_FRAME_MILLISECONDS } else { CHANGE_FRAME_MILLISECONDS };
        #[cfg(not(feature = "interaction"))]
        let change = CHANGE_FRAME_MILLISECONDS;
        // Frames at the pace the browser asks for, rather than as fast as
        // the processor allows.
        // A change that is not the page's waits one such frame for the
        // channel's answer, and is drawn with it (REQ-6400).
        #[cfg(feature = "interaction")]
        let hold = if self.view.browser.typing_target().is_some() { TYPING_FRAME_MILLISECONDS } else { 0 };
        #[cfg(not(feature = "interaction"))]
        let hold = 0;
        let next = self.view.pace.next(now, self.view.browser.frame_interval(now), change, hold);
        if next.draw {
            self.request_redraw();
        }
        self.start.checked_add(Duration::from_millis(next.wake?))
    }

    /// Whether a frame of `key` is one the viewer waits for, drawn as it
    /// arrives (REQ-RCH-001): the page keys go to, or the page under the
    /// pointer, a control's or link's look or a channel's answer to a
    /// click.
    pub(super) fn awaited(&self, key: PageKey) -> bool {
        #[cfg(feature = "interaction")]
        if self.view.browser.typing_target() == Some(key) {
            return true;
        }
        self.view.browser.pointed_page() == Some(key)
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

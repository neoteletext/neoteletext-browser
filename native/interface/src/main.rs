//! The interface process (design section 5.2): it draws the window, the
//! tabs, the trusted strip, the banks, the page list, the page area and the
//! overview, and holds no secrets. It reads only what the trusted core
//! sends on standard input, and writes only intents and strip input on
//! standard output. It opens no network connection and no file.
//!
//! A malformed message from the trusted core ends the process
//! (`architecture.md` section 6.4).

// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::io::{self, BufReader, BufWriter, Write};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use draw_vello::{Family, Painter};
use interface_core::browser::{Browser, Cursor, Frame, Key as BrowserKey, Modifiers, Output, StripState, TabInfo, WindowAction};
use interface_core::canvas::PageKey;
use interface_core::channel::Channel;
use protocol::{ToCore, ToInterface, read_from_core, write_to_core};
use softbuffer::{Context, Surface};
use t42_interface::audio::{self, Shared, Speaker};
use t42_interface::image;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

const WINDOW_TITLE: &str = "T42 Browser";
const WINDOW_WIDTH: f64 = 1440.0;
const WINDOW_HEIGHT: f64 = 900.0;
const MINIMUM_WIDTH: f64 = 1000.0;
const MINIMUM_HEIGHT: f64 = 640.0;
/// How far in from the window's edge a press resizes it, on platforms
/// where the browser draws its own frame.
const RESIZE_MARGIN: f64 = 6.0;
/// The shortest time between frames that only show what the trusted core
/// sent: ten a second.
const CHANGE_FRAME_MILLISECONDS: u64 = 100;

/// On macOS the system's window buttons sit in the tab strip, as in
/// Chrome; they take this much of its left end.
#[cfg(target_os = "macos")]
const FRAME: Frame = Frame { left_inset: 72.0, window_buttons: false };
/// Elsewhere the browser draws the whole frame, as Chrome does.
#[cfg(not(target_os = "macos"))]
const FRAME: Frame = Frame { left_inset: 0.0, window_buttons: true };

/// What the reader thread hands to the window's thread.
enum Incoming {
    Tabs(Vec<TabInfo>, usize),
    Strip(StripState),
    Channel(Channel),
    Arrived {
        channel: u32,
        page: u16,
        title: String,
    },
    Updated {
        channel: u32,
        page: u16,
    },
    Page {
        key: PageKey,
        image: image::Indexed,
    },
    /// The trusted core closed the pipe, asked to close, or broke the protocol.
    End(Option<String>),
}

struct Graphics {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    /// Kept for the surface's lifetime.
    _context: Context<Rc<Window>>,
}

struct App {
    start: Instant,
    graphics: Option<Graphics>,
    browser: Browser,
    /// Exists before the window, so page images that arrive first are kept.
    painter: Painter,
    modifiers: ModifiersState,
    /// The pointer, in logical pixels.
    cursor: (f64, f64),
    /// When the last frame was drawn, in milliseconds from `start`.
    last_frame: u64,
    /// Whether the trusted core changed something not yet drawn.
    dirty: bool,
    /// The pointer shape last set on the window.
    pointer_shape: Cursor,
    output: BufWriter<io::Stdout>,
    /// The page last reported as opened.
    opened: Option<PageKey>,
    exit: Option<String>,
}

fn attributes() -> winit::window::WindowAttributes {
    let attributes = Window::default_attributes()
        .with_title(WINDOW_TITLE)
        .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .with_min_inner_size(LogicalSize::new(MINIMUM_WIDTH, MINIMUM_HEIGHT));
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS;
        attributes.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true)
    }
    #[cfg(not(target_os = "macos"))]
    attributes.with_decorations(false)
}

impl App {
    fn now(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn scale(&self) -> f64 {
        self.graphics.as_ref().map_or(1.0, |graphics| graphics.window.scale_factor())
    }

    fn request_redraw(&self) {
        if let Some(graphics) = &self.graphics {
            graphics.window.request_redraw();
        }
    }

    fn modifiers(&self) -> Modifiers {
        let command = if cfg!(target_os = "macos") { self.modifiers.super_key() } else { self.modifiers.control_key() };
        Modifiers { shift: self.modifiers.shift_key(), command }
    }

    fn send(&mut self, event_loop: &ActiveEventLoop, message: &ToCore) {
        if let Err(error) = write_to_core(&mut self.output, message) {
            self.exit = Some(format!("cannot write to the trusted core: {error}"));
            event_loop.exit();
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let now = self.now();
        let Some(graphics) = self.graphics.as_mut() else { return };
        let size = graphics.window.inner_size();
        let (Ok(width), Ok(height)) = (u16::try_from(size.width), u16::try_from(size.height)) else { return };
        let (Some(surface_width), Some(surface_height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if graphics.surface.resize(surface_width, surface_height).is_err() {
            return;
        }
        self.last_frame = now;
        self.dirty = false;
        self.painter.begin(width, height, graphics.window.scale_factor());
        let logical = (f64::from(width) / self.painter.scale, f64::from(height) / self.painter.scale);
        self.browser.draw(&mut self.painter, logical.0, logical.1, now);
        let Ok(mut buffer) = graphics.surface.buffer_mut() else { return };
        self.painter.finish(&mut buffer);
        graphics.window.pre_present_notify();
        let _ = buffer.present();
        self.act(event_loop);
    }

    /// Sets the window's pointer shape to the browser's, when it changed.
    fn update_cursor(&mut self) {
        let cursor = self.browser.cursor();
        if cursor == self.pointer_shape {
            return;
        }
        self.pointer_shape = cursor;
        if let Some(graphics) = &self.graphics {
            graphics.window.set_cursor(match cursor {
                Cursor::Arrow => CursorIcon::Default,
                Cursor::Hand => CursorIcon::Pointer,
            });
        }
    }

    /// Sends on or carries out what the user did.
    fn act(&mut self, event_loop: &ActiveEventLoop) {
        self.update_cursor();
        for output in self.browser.take_outputs() {
            let message = match output {
                Output::Visible(visible) => {
                    // Images of pages no longer shown are dropped (requirement 9.5).
                    self.painter.retain_pages(|key| visible.contains(key));
                    ToCore::Visible(visible)
                }
                Output::Cell(click) => ToCore::Cell { key: click.key, column: click.column, row: click.row },
                Output::SelectTab(index) | Output::CloseTab(index) => {
                    let Ok(index) = u8::try_from(index) else { continue };
                    if matches!(output, Output::SelectTab(_)) { ToCore::SelectTab(index) } else { ToCore::CloseTab(index) }
                }
                Output::NewTab => ToCore::NewTab,
                Output::Navigate(address) => ToCore::Navigate(address),
                Output::Back => ToCore::Back,
                Output::Forward => ToCore::Forward,
                Output::Bookmark => ToCore::Bookmark,
                Output::Menu => continue,
                Output::Window(action) => {
                    self.window_action(event_loop, action);
                    continue;
                }
                Output::TextInput(allowed) => {
                    if let Some(graphics) = &self.graphics {
                        graphics.window.set_ime_allowed(allowed);
                    }
                    continue;
                }
            };
            self.send(event_loop, &message);
        }
        // The opened page's sounds play (the engine's REQ-AUD-012).
        let opened = self.browser.opened();
        if opened != self.opened {
            self.opened = opened;
            self.send(event_loop, &ToCore::Opened(opened));
        }
    }

    fn window_action(&self, event_loop: &ActiveEventLoop, action: WindowAction) {
        let Some(graphics) = &self.graphics else { return };
        let window = &graphics.window;
        match action {
            WindowAction::Drag => {
                let _ = window.drag_window();
            }
            WindowAction::Minimise => window.set_minimized(true),
            WindowAction::Maximise => window.set_maximized(!window.is_maximized()),
            WindowAction::Close => event_loop.exit(),
        }
    }

    /// On platforms where the browser draws its own frame, a press at the
    /// window's edge resizes it. Returns whether it did.
    fn resize_from_edge(&self, x: f64, y: f64) -> bool {
        if !FRAME.window_buttons {
            return false;
        }
        let Some(graphics) = &self.graphics else { return false };
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

    fn key(&mut self, key: &Key, text: Option<&str>) -> bool {
        let now = self.now();
        let modifiers = self.modifiers();
        let named = match key {
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
        if let Some(named) = named {
            return self.browser.key(named, modifiers, now);
        }
        if modifiers.command {
            // Shortcuts use the key itself, not what it would type.
            if let Key::Character(character) = key {
                return self.browser.key(BrowserKey::Text(character), modifiers, now);
            }
            return false;
        }
        text.is_some_and(|text| self.browser.key(BrowserKey::Text(text), modifiers, now))
    }
}

impl ApplicationHandler<Incoming> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }
        let Ok(window) = event_loop.create_window(attributes()) else {
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
        self.graphics = Some(Graphics { window, surface, _context: context });
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, incoming: Incoming) {
        let now = self.now();
        match incoming {
            Incoming::Tabs(tabs, active) => self.browser.set_tabs(tabs, active),
            Incoming::Strip(strip) => self.browser.set_strip(strip),
            Incoming::Channel(channel) => self.browser.set_channel(channel),
            Incoming::Arrived { channel, page, title } => self.browser.add_page(channel, page, title, now),
            Incoming::Updated { channel, page } => self.browser.page_updated(channel, page, now),
            Incoming::Page { key, image } => {
                if let Err(error) = self.painter.set_page(key, &image.palette, &image.indices) {
                    self.exit = Some(format!("bad page image: {error:?}"));
                    event_loop.exit();
                    return;
                }
            }
            Incoming::End(reason) => {
                self.exit = reason;
                event_loop.exit();
                return;
            }
        }
        // Drawn with the next paced frame: pages load and flash many times
        // a second, and one frame shows all that arrived since the last.
        self.dirty = true;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        let scale = self.scale();
        let now = self.now();
        let changed = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                false
            }
            WindowEvent::RedrawRequested => {
                self.redraw(event_loop);
                false
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => true,
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x / scale, position.y / scale);
                let changed = self.browser.pointer_moved(self.cursor.0, self.cursor.1, now);
                self.update_cursor();
                changed
            }
            // Leaving the window puts the pointer nowhere: what pops out
            // under it settles back.
            WindowEvent::CursorLeft { .. } => {
                self.cursor = (-1.0, -1.0);
                self.browser.pointer_moved(self.cursor.0, self.cursor.1, now)
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if self.resize_from_edge(self.cursor.0, self.cursor.1) {
                    return;
                }
                let changed = self.browser.pointer_pressed(&mut self.painter, self.modifiers.shift_key(), now);
                self.act(event_loop);
                changed
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                self.browser.pointer_released();
                false
            }
            WindowEvent::MouseWheel { delta, .. } => match delta {
                // A wheel that moves in notches; a trackpad reports pixels.
                MouseScrollDelta::LineDelta(_, y) => self.browser.wheel_lines(-f64::from(y), now),
                MouseScrollDelta::PixelDelta(position) => self.browser.scroll_by(-position.y / scale, now),
            },
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let changed = self.key(&event.logical_key, event.text.as_deref());
                self.act(event_loop);
                changed
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                let changed = self.browser.key(BrowserKey::Text(&text), Modifiers::default(), now);
                self.act(event_loop);
                changed
            }
            _ => false,
        };
        if changed {
            self.request_redraw();
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device: winit::event::DeviceId, _event: winit::event::DeviceEvent) {}

    /// Redraws while something animates; otherwise sleeps until the next
    /// event.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = self.now();
        if self.browser.settle(now) {
            self.request_redraw();
        }
        let interval = match (self.browser.frame_interval(now), self.dirty) {
            (Some(interval), _) => Some(interval),
            (None, true) => Some(CHANGE_FRAME_MILLISECONDS),
            (None, false) => None,
        };
        if let Some(interval) = interval {
            // Frames at the pace the browser asks for, rather than as fast
            // as the processor allows.
            if now.saturating_sub(self.last_frame) >= interval {
                self.request_redraw();
            }
            let next = self.start.checked_add(Duration::from_millis(self.last_frame.saturating_add(interval)));
            event_loop.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// Reads the trusted core's messages until the pipe ends, decoding page
/// images here so the window's thread never waits on them.
fn read_core(proxy: &EventLoopProxy<Incoming>, mixer: Option<&Shared>, trace: bool) {
    let mut input = BufReader::new(io::stdin().lock());
    let end = loop {
        let incoming = match read_from_core(&mut input) {
            Ok(Some(ToInterface::Tabs { tabs, active })) => Incoming::Tabs(tabs, active),
            Ok(Some(ToInterface::Strip(strip))) => Incoming::Strip(strip),
            Ok(Some(ToInterface::Channel(channel))) => Incoming::Channel(channel),
            Ok(Some(ToInterface::Page { channel, page, title })) => Incoming::Arrived { channel, page, title },
            Ok(Some(ToInterface::Updated { channel, page })) => Incoming::Updated { channel, page },
            Ok(Some(ToInterface::Frame { key, png, .. })) => match image::decode(&png) {
                Ok(image) => Incoming::Page { key, image },
                Err(error) => break Some(format!("bad page image from the trusted core: {error}")),
            },
            // Sound goes straight to the mixer; the window need not wake.
            Ok(Some(ToInterface::Audio(operation))) => {
                if trace && !matches!(operation, protocol::audio::Audio::Samples { .. }) {
                    eprintln!("t42-interface: audio {operation:?}");
                }
                if let Some(mixer) = mixer {
                    audio::apply(mixer, operation);
                }
                continue;
            }
            Ok(Some(ToInterface::Close) | None) => break None,
            Err(error) => break Some(format!("bad message from the trusted core: {error}")),
        };
        if proxy.send_event(incoming).is_err() {
            return;
        }
    };
    let _ = proxy.send_event(Incoming::End(end));
}

fn main() -> std::process::ExitCode {
    let Ok(event_loop) = EventLoop::<Incoming>::with_user_event().build() else {
        eprintln!("t42-interface: cannot open the event loop");
        return std::process::ExitCode::FAILURE;
    };
    let proxy = event_loop.create_proxy();
    // T42_AUDIO_TRACE reports the sound operations applied and the loudest
    // sample the device played each half second, for checking sound
    // without listening.
    let trace = std::env::var_os("T42_AUDIO_TRACE").is_some();
    // With no output device the browser runs silent.
    let speaker = Speaker::open().map_err(|reason| eprintln!("t42-interface: no sound: {reason}")).ok();
    let mixer = speaker.as_ref().map(|speaker| Arc::clone(&speaker.mixer));
    if trace && let Some(peak) = speaker.as_ref().map(|speaker| Arc::clone(&speaker.peak)) {
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let loudest = peak.swap(0, std::sync::atomic::Ordering::Relaxed);
                if loudest > 0 {
                    eprintln!("t42-interface: audio peak {loudest}");
                }
            }
        });
    }
    std::thread::spawn(move || read_core(&proxy, mixer.as_ref(), trace));
    let mut app = App {
        start: Instant::now(),
        graphics: None,
        browser: Browser::new(FRAME),
        painter: Painter::new(Family::Roboto),
        modifiers: ModifiersState::empty(),
        cursor: (0.0, 0.0),
        last_frame: 0,
        dirty: false,
        pointer_shape: Cursor::Arrow,
        output: BufWriter::new(io::stdout()),
        opened: None,
        exit: None,
    };
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("t42-interface: {error}");
        return std::process::ExitCode::FAILURE;
    }
    if let Some(reason) = app.exit {
        eprintln!("t42-interface: {reason}");
        return std::process::ExitCode::FAILURE;
    }
    let _ = app.output.flush();
    std::process::ExitCode::SUCCESS
}

//! The interface process (design section 5.2): it draws the window, the
//! tabs, the trusted strip, the banks, the page list, the page area and the
//! overview, and holds no secrets. It reads only what the trusted core
//! sends on standard input, and writes only intents and strip input on
//! standard output. It opens no network connection and no file.
//!
//! A malformed message from the trusted core ends the process
//! (`architecture.md` section 6.4).

// On Windows, a desktop program: no console window opens with it. The
// processes talk through the pipes they are given, not a console.
#![cfg_attr(windows, windows_subsystem = "windows")]
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

use std::collections::{BTreeSet, HashMap};
use std::io::{self, BufReader, BufWriter, Write};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dock_menu::{Choice, DockTab};
use draw_vello::{Family, Painter};
use interface_core::browser::{Browser, Cursor, Frame, Key as BrowserKey, Modifiers, Output, StripState, TabInfo, WindowAction};
use interface_core::canvas::PageKey;
use interface_core::channel::Channel;
use interface_core::controls::FastextKey;
use interface_core::library::WindowGeometry;
use protocol::frame::{self, FrameError};
use protocol::{ToCore, ToInterface, read_from_core, write_to_core};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

const WINDOW_TITLE: &str = "NeoTeletext";
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
/// One frame of a 60 hertz display: what a channel being typed into
/// pushes is drawn at the next (`interaction.md` section 10).
#[cfg(feature = "interaction")]
const TYPING_FRAME_MILLISECONDS: u64 = 16;
/// On macOS the system's window buttons sit in the tab strip, as in
/// Chrome; they take this much of its left end.
#[cfg(target_os = "macos")]
const FRAME: Frame = Frame { left_inset: 72.0, window_buttons: false };
/// Elsewhere the browser draws the whole frame, as Chrome does.
#[cfg(not(target_os = "macos"))]
const FRAME: Frame = Frame { left_inset: 0.0, window_buttons: true };

/// The trusted core's views of history and bookmarks.
enum Library {
    History(interface_core::library::HistoryView),
    Cookies(interface_core::cookies::CookiesView),
    Suggestions(interface_core::library::Suggestions),
    TabList(interface_core::library::TabList),
    Bookmarks(interface_core::library::BookmarksView),
    Star(interface_core::library::StarView),
    Bar(interface_core::library::BarView),
    PageControls(interface_core::controls::PageControls),
    Remote(interface_core::settings::RemotePlace),
}

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
        version: u32,
        /// In `protocol::frame`'s form, read on the window's thread against
        /// the frame of the page it holds.
        image: Vec<u8>,
    },
    /// About hyperlinks.
    #[cfg(feature = "links")]
    Link(protocol::links::Message),
    /// Whether keys go to the open page's channel.
    #[cfg(feature = "interaction")]
    Typing(protocol::interaction::Typing),
    /// The phone keyboard's panel and state.
    #[cfg(feature = "phone-keyboard")]
    Phone(protocol::keyboard::PhoneView),
    /// A key of the paired phone.
    #[cfg(feature = "phone-keyboard")]
    PhoneKey(protocol::keyboard::PhoneKey),
    /// The channels now playing sound, muted or not.
    Sounding(BTreeSet<u32>),
    /// The settings, for the panel of the trusted strip.
    Settings(interface_core::settings::SettingsView),
    /// New entries for the developer console.
    Console(interface_core::console::ConsoleUpdate),
    /// History, bookmarks and their panels (`history-and-bookmarks.md`).
    Library(Library),
    /// Where the window was when the session was kept.
    Window(WindowGeometry),
    /// The user chose an item of the Dock menu.
    Dock(Choice),
    /// The trusted core closed the pipe, asked to close, or broke the protocol.
    End(Option<String>),
}

#[derive(Default)]
struct InputMethod {
    /// The address is being edited: the method is the field's.
    address: bool,
    /// The method is enabled now.
    allowed: bool,
}

struct Graphics {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    /// Kept for the surface's lifetime.
    _context: Context<Rc<Window>>,
    /// The interface asked for the next redraw. One it did not ask for is
    /// the system's, as when the window is uncovered, and every pixel is
    /// drawn and presented again.
    asked: bool,
}

/// What the trusted core first said, which the window waits for.
#[derive(Clone, Copy)]
enum Heard {
    Nothing,
    /// Its first message, not the window's kept place.
    Spoke,
    /// The window's kept place, as its first message.
    Window(WindowGeometry),
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
    /// The tabs as the trusted core last sent them, for the window's title
    /// and the Dock menu.
    tabs: Vec<TabInfo>,
    active: usize,
    exit: Option<String>,
    /// The window's input method: whether the address is being edited, so
    /// the method is the field's, and whether the method is enabled now,
    /// which it is while the address is edited and while keys go to a
    /// channel (INPUT.md section 9.1).
    ime: InputMethod,
    /// Reports each frame and what it cost, for measuring the drawing.
    trace_frames: bool,
    /// The version and indices of the last frame of each page shown, which
    /// the next frame of the page may be sent as changes against.
    held: HashMap<PageKey, (u32, Vec<u8>)>,
    /// The window opens once the event loop has resumed and the trusted
    /// core's first message has come, which is the window's kept place when
    /// a session is restored (`history-and-bookmarks.md` section 15.3).
    resumed: bool,
    heard: Heard,
    /// The place and size last reported to the trusted core, and the last
    /// before the window was maximised, which it returns to.
    reported: Option<WindowGeometry>,
    normal: Option<WindowGeometry>,
}

/// The window, at its kept place and size when there are some.
fn attributes(kept: Option<WindowGeometry>) -> winit::window::WindowAttributes {
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
    #[cfg(not(target_os = "macos"))]
    attributes.with_decorations(false)
}

/// Whether the kept window's top left corner falls on a display now: one
/// unplugged since is not used, and the system places the window
/// (`history-and-bookmarks.md` section 15.3).
fn on_a_display(event_loop: &ActiveEventLoop, kept: &WindowGeometry) -> bool {
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
fn geometry(window: &Window) -> Option<WindowGeometry> {
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

impl App {
    fn now(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn scale(&self) -> f64 {
        self.graphics.as_ref().map_or(1.0, |graphics| graphics.window.scale_factor())
    }

    /// The window's title and the Dock menu follow the tabs: the title is
    /// the selected tab's, marked when it plays sound as a web browser
    /// marks it, so the Dock's window list names the selected tab.
    fn show_tabs(&self) {
        let tab = |(index, tab): (usize, &TabInfo)| DockTab {
            title: tab.title.clone(),
            selected: index == self.active,
            playing: self.browser.tab_sounding(tab),
            muted: tab.muted,
        };
        let tabs: Vec<DockTab> = self.tabs.iter().enumerate().map(tab).collect();
        if let Some(graphics) = &self.graphics {
            let title = match tabs.get(self.active) {
                Some(tab) if tab.muted && tab.playing => format!("{} \u{2013} Muted \u{2013} {WINDOW_TITLE}", tab.title),
                Some(tab) if tab.playing => format!("{} \u{2013} Audio playing \u{2013} {WINDOW_TITLE}", tab.title),
                Some(tab) => format!("{} \u{2013} {WINDOW_TITLE}", tab.title),
                None => WINDOW_TITLE.to_owned(),
            };
            graphics.window.set_title(&title);
        }
        dock_menu::set(tabs);
    }

    fn request_redraw(&mut self) {
        if let Some(graphics) = &mut self.graphics {
            graphics.asked = true;
            graphics.window.request_redraw();
        }
    }

    fn modifiers(&self) -> Modifiers {
        let command = if cfg!(target_os = "macos") { self.modifiers.super_key() } else { self.modifiers.control_key() };
        Modifiers { shift: self.modifiers.shift_key(), command, alt: self.modifiers.alt_key(), repeat: false }
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
        if !std::mem::take(&mut graphics.asked) {
            self.painter.redraw_whole();
        }
        let started = Instant::now();
        self.painter.begin(width, height, graphics.window.scale_factor());
        let logical = (f64::from(width) / self.painter.scale, f64::from(height) / self.painter.scale);
        self.browser.draw(&mut self.painter, logical.0, logical.1, now);
        // Only what changed since the last frame is drawn; a frame that
        // changed nothing is not presented, and the window keeps the last.
        let damage = self.painter.render();
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
        self.painter.copy_to(&mut buffer, &damage, age);
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
        let _ = buffer.present_with_damage(&rects);
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
                // A link that leaves the channel: the shape of a shortcut.
                Cursor::External => CursorIcon::Alias,
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
                    self.held.retain(|key, _| visible.contains(key));
                    ToCore::Visible(visible)
                }
                Output::Cell(click) => ToCore::Cell { key: click.key, version: click.version, column: click.column, row: click.row },
                #[cfg(feature = "links")]
                Output::Point(cell) => {
                    ToCore::Point(cell.map(|cell| protocol::links::Spot { key: cell.key, version: cell.version, column: cell.column, row: cell.row }))
                }
                #[cfg(feature = "links")]
                Output::Typed { number, opened } => ToCore::Typed { number, opened },
                #[cfg(feature = "links")]
                Output::Confirm(id) => ToCore::Confirm(id),
                #[cfg(feature = "links")]
                Output::Cancel(id) => ToCore::Cancel(id),
                #[cfg(feature = "interaction")]
                Output::Terminal { key, local, intent } => ToCore::Input(protocol::interaction::InputIntent { key, local, intent }),
                #[cfg(feature = "phone-keyboard")]
                Output::PhonePanel(open) => ToCore::PhonePanel(open),
                #[cfg(feature = "phone-keyboard")]
                Output::PhoneFinish => ToCore::PhoneFinish,
                Output::SelectTab(index) | Output::CloseTab(index) => {
                    let Ok(index) = u8::try_from(index) else { continue };
                    if matches!(output, Output::SelectTab(_)) { ToCore::SelectTab(index) } else { ToCore::CloseTab(index) }
                }
                Output::MoveTab { from, to } => {
                    let (Ok(from), Ok(to)) = (u8::try_from(from), u8::try_from(to)) else { continue };
                    ToCore::MoveTab { from, to }
                }
                Output::NewTab => ToCore::NewTab,
                Output::Navigate(address) => ToCore::Navigate(address),
                Output::Back => ToCore::Back,
                Output::Forward => ToCore::Forward,
                Output::Bookmark => ToCore::Bookmark,
                Output::Home => ToCore::Home,
                Output::Reload => ToCore::Reload,
                Output::PageControl(control) => ToCore::PageControl(control),
                Output::History(request) => ToCore::History(request),
                Output::Cookies(request) => ToCore::Cookies(request),
                Output::Bookmarks(request) => ToCore::Bookmarks(request),
                Output::Star(request) => ToCore::Star(request),
                Output::Bar(request) => ToCore::Bar(request),
                Output::Suggest(text) => ToCore::Suggest(text),
                Output::Suggestion { number, new_tab } => ToCore::Suggestion { number, new_tab },
                Output::TabList(forward) => ToCore::TabList(forward),
                Output::GoTo { forward, steps } => ToCore::GoTo { forward, steps },
                Output::NewPrivateTab => ToCore::NewPrivateTab,
                Output::Setting(change) => ToCore::ChangeSetting(change),
                Output::Console(open) => ToCore::Console(open),
                Output::MuteTab(index) => {
                    let Ok(index) = u8::try_from(index) else { continue };
                    ToCore::MuteTab(index)
                }
                Output::MuteAll(muted) => ToCore::MuteAll(muted),
                Output::Window(action) => {
                    self.window_action(event_loop, action);
                    continue;
                }
                // Text input is on exactly while the address is edited, when
                // the keys are not the open page's: its channel is told.
                Output::TextInput(allowed) => {
                    self.ime.address = allowed;
                    self.sync_ime();
                    #[cfg(feature = "interaction")]
                    self.send(event_loop, &ToCore::Editing(allowed));
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

    /// Enables the window's input method while the address is edited or
    /// keys go to a channel, whose composed text then goes to it.
    fn sync_ime(&mut self) {
        #[cfg(feature = "interaction")]
        let allowed = self.ime.address || self.browser.typing_target().is_some();
        #[cfg(not(feature = "interaction"))]
        let allowed = self.ime.address;
        if allowed != self.ime.allowed
            && let Some(graphics) = &self.graphics
        {
            graphics.window.set_ime_allowed(allowed);
            self.ime.allowed = allowed;
        }
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
                MouseScrollDelta::LineDelta(x, y) => self.browser.terminal_wheel(f64::from(x), -f64::from(y)),
                MouseScrollDelta::PixelDelta(position) => self.browser.terminal_wheel(lines(position.x), lines(-position.y)),
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
            MouseScrollDelta::LineDelta(_, y) => self.browser.wheel_lines(-f64::from(y), now),
            MouseScrollDelta::PixelDelta(position) => self.browser.scroll_by(-position.y / scale, now),
        }
    }

    /// Whether `event` is the browser's paste shortcut, the browser's
    /// modifier and V, while keys go to a channel.
    #[cfg(feature = "interaction")]
    fn is_paste(&self, event: &winit::event::KeyEvent) -> bool {
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        self.browser.typing_target().is_some()
            && self.browser_modifier()
            && matches!(event.key_without_modifiers(), Key::Character(ref key) if key.eq_ignore_ascii_case("v"))
    }

    /// The clipboard's text to the channel, as one PASTE; a clipboard that
    /// holds no text pastes nothing.
    #[cfg(feature = "interaction")]
    fn paste(&mut self) -> bool {
        let text = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text());
        text.is_ok_and(|text| self.browser.terminal_paste(&text))
    }

    /// Whether the browser's own modifier is held: Command on macOS,
    /// Control and Shift elsewhere, the keys a terminal emulator keeps.
    #[cfg(feature = "interaction")]
    fn browser_modifier(&self) -> bool {
        if cfg!(target_os = "macos") { self.modifiers.super_key() } else { self.modifiers.control_key() && self.modifiers.shift_key() }
    }

    /// A key while keys go to a channel: every key but the browser's own
    /// goes, pressed, repeating and released, as the trusted core's
    /// session asks for them (INPUT.md section 9.1). Returns whether the
    /// key was the channel's.
    #[cfg(feature = "interaction")]
    fn terminal_key(&mut self, event: &winit::event::KeyEvent) -> Option<bool> {
        use neoteletext_input::event::Action;
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        if self.browser.typing_target().is_none() || self.browser_modifier() {
            return None;
        }
        let now = self.now();
        let action = match (event.state, event.repeat) {
            (ElementState::Pressed, false) => Action::Press,
            (ElementState::Pressed, true) => Action::Repeat,
            (ElementState::Released, _) => Action::Release,
        };
        let unmodified = event.key_without_modifiers();
        let parts = neoteletext_interface::keys::parts(action, &event.logical_key, &unmodified, event.physical_key, event.text.as_deref(), self.modifiers);
        let changed = match parts {
            Some(parts) => self.browser.terminal_key(parts.action, parts.key, parts.mods, parts.text.as_deref(), parts.base, now),
            // A key that types several characters: they go as text.
            None => match (&event.text, action) {
                (Some(text), Action::Press) => self.browser.terminal_text(text, now),
                _ => false,
            },
        };
        Some(changed)
    }

    /// The phone keyboard's state, for its panel and the strip.
    #[cfg(feature = "phone-keyboard")]
    fn show_phone(&mut self, view: protocol::keyboard::PhoneView) {
        use interface_core::browser::{PhoneStatus, PhoneUiView};
        use protocol::keyboard::PhoneState;
        let status = match view.state {
            PhoneState::Preparing => PhoneStatus::Preparing,
            PhoneState::Scan => PhoneStatus::Scan,
            PhoneState::Connected => PhoneStatus::Connected,
            PhoneState::Waiting => PhoneStatus::Waiting,
            PhoneState::PinFailed(left) => PhoneStatus::PinFailed(left),
            PhoneState::Locked => PhoneStatus::Locked,
            PhoneState::Finished => PhoneStatus::Finished,
        };
        self.browser.set_phone(PhoneUiView { status, qr: view.qr.map(|qr| (qr.size, qr.bits)), pin: view.pin });
    }

    /// Whether keys go to the open page's channel, as the trusted core says.
    #[cfg(feature = "interaction")]
    fn show_typing(&mut self, typing: &protocol::interaction::Typing, now: u64) {
        let cursor = typing.cursor.map(|cursor| interface_core::browser::CursorView {
            row: cursor.row,
            column: cursor.column,
            width: cursor.width,
            flags: cursor.flags,
        });
        let view = interface_core::browser::TypingView {
            channel: typing.channel,
            page: typing.page,
            on: typing.on,
            secure: typing.secure,
            refused: typing.refused,
            modes: typing.modes,
            cursor,
            acked: typing.acked,
            round_trip_ms: typing.round_trip_ms,
            show: typing.show,
        };
        self.browser.set_typing(view, now);
        self.sync_ime();
    }

    /// A key of the paired phone: it acts as a key of the window's own
    /// keyboard, with no modifier (REQ-INT-138).
    #[cfg(feature = "phone-keyboard")]
    fn phone_key(&mut self, key: protocol::keyboard::PhoneKey) -> bool {
        use protocol::keyboard::PhoneKey;
        let now = self.now();
        let typing = self.browser.typing_target().is_some();
        match key {
            PhoneKey::Text(text) if typing => self.browser.terminal_text(&text, now),
            PhoneKey::Text(text) => self.browser.key(BrowserKey::Text(&text), Modifiers::default(), now),
            PhoneKey::Key { down, name } => {
                let mut characters = name.chars();
                let single = match (characters.next(), characters.next()) {
                    (Some(character), None) => Some(character),
                    _ => None,
                };
                if typing {
                    return neoteletext_interface::keys::phone_parts(down, &name)
                        .is_some_and(|parts| self.browser.terminal_key(parts.action, parts.key, parts.mods, parts.text.as_deref(), parts.base, now));
                }
                if !down {
                    return false;
                }
                let browser_key = match name.as_str() {
                    "Enter" => BrowserKey::Enter,
                    "Escape" | "BrowserBack" | "Exit" => BrowserKey::Escape,
                    "Backspace" => BrowserKey::Backspace,
                    "Tab" => BrowserKey::Tab,
                    "ArrowLeft" => BrowserKey::Left,
                    "ArrowRight" => BrowserKey::Right,
                    "ArrowUp" | "ChannelUp" => BrowserKey::Up,
                    "ArrowDown" | "ChannelDown" => BrowserKey::Down,
                    _ if single.is_some() => BrowserKey::Text(&name),
                    _ => return false,
                };
                self.browser.key(browser_key, Modifiers::default(), now)
            }
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
            return self.browser.fastext_key(colour);
        }
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
        self.resumed = true;
        self.open_window(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, incoming: Incoming) {
        if matches!(self.heard, Heard::Nothing) {
            self.heard = if let Incoming::Window(kept) = &incoming { Heard::Window(*kept) } else { Heard::Spoke };
            self.open_window(event_loop);
        }
        self.incoming(event_loop, incoming);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
            self.report_window(event_loop);
        }
        // Resized or uncovered, the window may have lost what was
        // presented: every pixel is drawn again.
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Occluded(false)) {
            self.painter.redraw_whole();
            self.request_redraw();
        }
        self.window_input(event_loop, window_id, event);
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device: winit::event::DeviceId, _event: winit::event::DeviceEvent) {}

    /// Redraws while something animates; otherwise sleeps until the next
    /// event.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // What the events produced goes to the trusted core before waiting.
        if let Err(error) = self.output.flush() {
            self.exit = Some(format!("cannot write to the trusted core: {error}"));
            event_loop.exit();
            return;
        }
        let now = self.now();
        if self.browser.settle(now) {
            // What settling asked for, as a held Back's list, goes now.
            self.act(event_loop);
            if let Err(error) = self.output.flush() {
                self.exit = Some(format!("cannot write to the trusted core: {error}"));
                event_loop.exit();
                return;
            }
            self.request_redraw();
        }
        // While keys go to a channel, what it pushes is drawn at the
        // display's rate (`interaction.md` section 10).
        #[cfg(feature = "interaction")]
        let change = if self.browser.typing_target().is_some() { TYPING_FRAME_MILLISECONDS } else { CHANGE_FRAME_MILLISECONDS };
        #[cfg(not(feature = "interaction"))]
        let change = CHANGE_FRAME_MILLISECONDS;
        let interval = match (self.browser.frame_interval(now), self.dirty) {
            (Some(interval), true) => Some(interval.min(change)),
            (Some(interval), false) => Some(interval),
            (None, true) => Some(change),
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

impl App {
    /// Reads a page's frame against the one held and shows it; false when
    /// it is not shown: changes to a page no longer held, or a broken frame,
    /// which ends the interface.
    fn show_frame(&mut self, event_loop: &ActiveEventLoop, key: PageKey, version: u32, image: &[u8]) -> bool {
        let held = self.held.get(&key).map(|(version, indices)| (*version, indices.as_slice()));
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
        if let Err(error) = self.painter.set_page(key, &image.palette, image.screen, &image.indices) {
            self.exit = Some(format!("bad page image: {error:?}"));
            event_loop.exit();
            return false;
        }
        self.held.insert(key, (version, image.indices));
        self.browser.frame_version(key, version);
        true
    }

    /// Opens the window once the event loop has resumed and the trusted
    /// core has spoken.
    fn open_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() || !self.resumed {
            return;
        }
        let kept = match self.heard {
            Heard::Nothing => return,
            Heard::Spoke => None,
            Heard::Window(kept) => Some(kept).filter(|kept| on_a_display(event_loop, kept)),
        };
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
        self.graphics = Some(Graphics { window, surface, _context: context, asked: false });
        self.show_tabs();
        self.report_window(event_loop);
    }

    /// A press of the middle button: on a tab, it closes.
    fn middle_pressed(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let changed = self.browser.middle_pressed();
        self.act(event_loop);
        changed
    }

    /// Tells the trusted core where the window is and how large, to keep
    /// with the session (requirement 20.21); while maximised, the place and
    /// size it returns to.
    fn report_window(&mut self, event_loop: &ActiveEventLoop) {
        let Some(now) = self.graphics.as_ref().and_then(|graphics| geometry(&graphics.window)) else { return };
        let report = if now.maximised {
            self.normal.map_or(now, |normal| WindowGeometry { maximised: true, ..normal })
        } else {
            self.normal = Some(now);
            now
        };
        if self.reported != Some(report) {
            self.reported = Some(report);
            self.send(event_loop, &ToCore::Window(report));
        }
    }

    fn incoming(&mut self, event_loop: &ActiveEventLoop, incoming: Incoming) {
        let now = self.now();
        match incoming {
            // Used when the window opens; it opens with the first message.
            Incoming::Window(_) => return,
            Incoming::Tabs(tabs, active) => {
                self.browser.set_tabs(tabs.clone(), active);
                self.tabs = tabs;
                self.active = active;
                self.show_tabs();
            }
            // The sound helper said which channels sound, through the
            // trusted core (decision 55).
            Incoming::Sounding(channels) => {
                if self.browser.set_sounding(channels) {
                    self.show_tabs();
                    self.request_redraw();
                }
                return;
            }
            Incoming::Dock(choice) => {
                let message = match choice {
                    Choice::Tab(index) => match u8::try_from(index) {
                        Ok(index) => ToCore::SelectTab(index),
                        Err(_) => return,
                    },
                    Choice::MuteAll(muted) => ToCore::MuteAll(muted),
                };
                self.send(event_loop, &message);
                return;
            }
            Incoming::Strip(strip) => self.browser.set_strip(strip),
            Incoming::Settings(view) => self.browser.set_settings(view),
            Incoming::Console(update) => self.browser.console_update(update),
            Incoming::Library(view) => match view {
                Library::History(view) => self.browser.set_history(view),
                Library::Cookies(view) => self.browser.set_cookies(view),
                Library::PageControls(controls) => self.browser.set_page_controls(controls),
                Library::Remote(place) => self.browser.set_remote(place),
                Library::Suggestions(suggestions) => self.browser.set_suggestions(suggestions),
                Library::TabList(list) => self.browser.set_tab_list(list),
                Library::Bookmarks(view) => self.browser.set_bookmarks(view, now),
                Library::Star(view) => self.browser.set_star(view),
                Library::Bar(view) => self.browser.set_bar(view),
            },
            Incoming::Channel(channel) => self.browser.set_channel(channel),
            Incoming::Arrived { channel, page, title } => self.browser.add_page(channel, page, title, now),
            Incoming::Updated { channel, page } => {
                if self.trace_frames {
                    eprintln!("neoteletext-interface: at {now} ms: page {page:03X} of channel {channel} updated");
                }
                self.browser.page_updated(channel, page, now);
            }
            Incoming::Page { key, version, image } => {
                if self.trace_frames {
                    eprintln!("neoteletext-interface: at {now} ms: frame {version} of page {:03X} of channel {}", key.page, key.channel);
                }
                if self.show_frame(event_loop, key, version, &image) {
                    self.act(event_loop);
                }
            }
            #[cfg(feature = "phone-keyboard")]
            Incoming::Phone(view) => {
                self.show_phone(view);
                self.request_redraw();
            }
            #[cfg(feature = "phone-keyboard")]
            Incoming::PhoneKey(key) => {
                if self.phone_key(key) {
                    self.request_redraw();
                }
                self.act(event_loop);
            }
            #[cfg(feature = "interaction")]
            Incoming::Typing(typing) => {
                self.show_typing(&typing, now);
                self.act(event_loop);
            }
            #[cfg(feature = "links")]
            Incoming::Link(message) => {
                use protocol::links::Message;
                match message {
                    Message::Page { key, links } => self.browser.set_links(key, links),
                    Message::Pointing(pointing) => self.browser.set_pointing(pointing),
                    Message::Prompt(prompt) => self.browser.set_prompt(prompt, now),
                    Message::Show(key) => self.browser.show(key, now),
                    Message::Aliases { channel, numbers } => self.browser.set_aliases(channel, numbers),
                }
                self.act(event_loop);
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

    fn window_input(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
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
                #[cfg(feature = "interaction")]
                {
                    let held = self.browser_modifier();
                    self.browser.set_browser_modifier(held);
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
                // Command-click (Control-click elsewhere) opens in a new tab
                // what opens a channel; anywhere else it is a press.
                let changed = (self.modifiers().command && self.browser.new_tab_pressed())
                    || self.browser.pointer_pressed(&mut self.painter, self.modifiers.shift_key(), now);
                self.act(event_loop);
                changed
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                self.browser.pointer_released();
                self.act(event_loop);
                true
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Middle, .. } => self.middle_pressed(event_loop),
            // A secondary click: Back's or Forward's list (requirement 20.10).
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                let changed = self.browser.secondary_pressed();
                self.act(event_loop);
                changed
            }
            WindowEvent::MouseWheel { delta, .. } => self.wheel(event_loop, delta, now),
            #[cfg(feature = "interaction")]
            WindowEvent::KeyboardInput { ref event, .. } if self.browser.typing_target().is_some() && !self.browser_modifier() => {
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
            WindowEvent::Ime(Ime::Commit(text)) if self.browser.typing_target().is_some() => {
                let changed = self.browser.terminal_text(&text, now);
                self.act(event_loop);
                changed
            }
            // The input method's preedit goes to the channel in the COMPOSE
            // mode (INPUT.md section 9.1), its cursor as a code-point
            // offset.
            #[cfg(feature = "interaction")]
            WindowEvent::Ime(Ime::Preedit(text, cursor)) if self.browser.composes_for_channel() => {
                let at = cursor.map_or(text.len(), |(start, _)| start);
                let offset = text.get(..at).map_or(0, |before| before.chars().count());
                let changed = self.browser.terminal_compose(u32::try_from(offset).unwrap_or(u32::MAX), &text);
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
}

/// Reads the trusted core's messages until the pipe ends. Page images are
/// read on the window's thread, against the frame of the page it holds: a
/// whole page in runs takes about 45 microseconds, and a page's changes 5
/// (measured 22 October 2026).
fn read_core(proxy: &EventLoopProxy<Incoming>) {
    let mut input = BufReader::new(io::stdin().lock());
    let end = loop {
        let incoming = match read_from_core(&mut input) {
            Ok(Some(ToInterface::Tabs { tabs, active })) => Incoming::Tabs(tabs, active),
            Ok(Some(ToInterface::Strip(strip))) => Incoming::Strip(strip),
            Ok(Some(ToInterface::Channel(channel))) => Incoming::Channel(channel),
            Ok(Some(ToInterface::Page { channel, page, title })) => Incoming::Arrived { channel, page, title },
            Ok(Some(ToInterface::Updated { channel, page })) => Incoming::Updated { channel, page },
            Ok(Some(ToInterface::Frame { key, version, image })) => Incoming::Page { key, version, image },
            Ok(Some(ToInterface::Sounding(channels))) => Incoming::Sounding(channels.into_iter().collect()),
            #[cfg(feature = "links")]
            Ok(Some(ToInterface::Link(message))) => Incoming::Link(message),
            #[cfg(feature = "interaction")]
            Ok(Some(ToInterface::Typing(typing))) => Incoming::Typing(typing),
            #[cfg(feature = "phone-keyboard")]
            Ok(Some(ToInterface::Phone(view))) => Incoming::Phone(view),
            #[cfg(feature = "phone-keyboard")]
            Ok(Some(ToInterface::PhoneKey(key))) => Incoming::PhoneKey(key),
            Ok(Some(ToInterface::Settings(view))) => Incoming::Settings(view),
            Ok(Some(ToInterface::Console(update))) => Incoming::Console(update),
            Ok(Some(ToInterface::History(view))) => Incoming::Library(Library::History(view)),
            Ok(Some(ToInterface::Cookies(view))) => Incoming::Library(Library::Cookies(view)),
            Ok(Some(ToInterface::Suggestions(suggestions))) => Incoming::Library(Library::Suggestions(suggestions)),
            Ok(Some(ToInterface::TabList(list))) => Incoming::Library(Library::TabList(list)),
            Ok(Some(ToInterface::Bookmarks(view))) => Incoming::Library(Library::Bookmarks(view)),
            Ok(Some(ToInterface::Star(view))) => Incoming::Library(Library::Star(view)),
            Ok(Some(ToInterface::Bar(view))) => Incoming::Library(Library::Bar(view)),
            Ok(Some(ToInterface::Window(window))) => Incoming::Window(window),
            Ok(Some(ToInterface::PageControls(controls))) => Incoming::Library(Library::PageControls(controls)),
            Ok(Some(ToInterface::Remote(place))) => Incoming::Library(Library::Remote(place)),
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
    // First, before any thread and before the window system is reached:
    // the window, fonts, text input, the pasteboard and the sound output,
    // no network (requirement 3.6).
    match confine::apply(confine::Role::Interface, None) {
        Ok(report) => eprintln!("neoteletext-interface: confined: {report}"),
        Err(error) => {
            eprintln!("neoteletext-interface: {error}");
            return std::process::ExitCode::FAILURE;
        }
    }
    let Ok(event_loop) = EventLoop::<Incoming>::with_user_event().build() else {
        eprintln!("neoteletext-interface: cannot open the event loop");
        return std::process::ExitCode::FAILURE;
    };
    let proxy = event_loop.create_proxy();
    // The application's delegate exists once the event loop does; AppKit
    // first asks it for the Dock menu as the application launches, so the
    // menu is installed before then.
    let dock_proxy = proxy.clone();
    dock_menu::install(move |choice| {
        let _ = dock_proxy.send_event(Incoming::Dock(choice));
    });
    std::thread::spawn(move || read_core(&proxy));
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
        tabs: Vec::new(),
        active: 0,
        exit: None,
        ime: InputMethod::default(),
        // NEOTELETEXT_FRAME_TRACE reports every frame drawn and its cost.
        trace_frames: std::env::var_os("NEOTELETEXT_FRAME_TRACE").is_some(),
        held: HashMap::new(),
        resumed: false,
        heard: Heard::Nothing,
        reported: None,
        normal: None,
    };
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("neoteletext-interface: {error}");
        return std::process::ExitCode::FAILURE;
    }
    if let Some(reason) = app.exit {
        eprintln!("neoteletext-interface: {reason}");
        return std::process::ExitCode::FAILURE;
    }
    let _ = app.output.flush();
    std::process::ExitCode::SUCCESS
}

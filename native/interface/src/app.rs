//! The interface's state and its event loop's handler: the window at hand
//! and the others set aside, what the trusted core said, and what the user
//! does, sent on as intents and strip input.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufWriter, Write};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dock_menu::{Choice, DockTab, EditAction, MenuAction, MenuItem, ShortcutKey};
use draw_vello::{Family, Painter};
use interface_core::browser::{Browser, Cursor, Key as BrowserKey, Modifiers, Output, TabInfo, WindowAction};
use interface_core::canvas::PageKey;
use interface_core::controls::FastextKey;
use interface_core::library::WindowGeometry;
use protocol::frame::{self, FrameError};
use protocol::{ToCore, write_to_core};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalPosition;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

use crate::incoming::{Incoming, Library};
use crate::window::{FRAME, RESIZE_MARGIN, TORN_OFFSET, WINDOW_TITLE, attributes, geometry, on_a_display};

/// The shortest time between frames that only show what the trusted core
/// sent: ten a second.
const CHANGE_FRAME_MILLISECONDS: u64 = 100;
/// One frame of a 60 hertz display: what a channel being typed into
/// pushes is drawn at the next (`interaction.md` section 10).
#[cfg(feature = "interaction")]
const TYPING_FRAME_MILLISECONDS: u64 = 16;

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

/// One window and what it shows (`multiple-windows.md`): the interface
/// holds the window at hand in [`App::view`] and sets the others aside.
struct View {
    graphics: Option<Graphics>,
    browser: Browser,
    /// Exists before the window, so page images that arrive first are kept.
    painter: Painter,
    /// The pointer, in logical pixels.
    cursor: (f64, f64),
    /// When the last frame was drawn, in milliseconds from `start`.
    last_frame: u64,
    /// Whether the trusted core changed something not yet drawn.
    dirty: bool,
    /// The pointer shape last set on the window.
    pointer_shape: Cursor,
    /// The page last reported as opened.
    opened: Option<PageKey>,
    /// The tabs as the trusted core last sent them, for the window's title
    /// and the Dock menu.
    tabs: Vec<TabInfo>,
    active: usize,
    /// The window's input method: whether the address is being edited, so
    /// the method is the field's, and whether the method is enabled now,
    /// which it is while the address is edited and while keys go to a
    /// channel (INPUT.md section 9.1).
    ime: InputMethod,
    /// The version and indices of the last frame of each page shown, which
    /// the next frame of the page may be sent as changes against.
    held: HashMap<PageKey, (u32, Vec<u8>)>,
    /// The place and size last reported to the trusted core, and the last
    /// before the window was maximised, which it returns to.
    reported: Option<WindowGeometry>,
    normal: Option<WindowGeometry>,
}

impl View {
    fn new() -> Self {
        Self {
            graphics: None,
            browser: Browser::new(FRAME),
            painter: Painter::new(Family::Roboto),
            cursor: (0.0, 0.0),
            last_frame: 0,
            dirty: false,
            pointer_shape: Cursor::Arrow,
            opened: None,
            tabs: Vec::new(),
            active: 0,
            ime: InputMethod::default(),
            held: HashMap::new(),
            reported: None,
            normal: None,
        }
    }
}

/// The interface: its windows, what each shows, and its pipe to the
/// trusted core.
pub struct App {
    start: Instant,
    /// The window at hand.
    view: View,
    /// Its number, as the trusted core numbers windows.
    window: u32,
    /// The other windows, set aside.
    parked: BTreeMap<u32, View>,
    /// Each window's number, by the system's identifier.
    windows: HashMap<WindowId, u32>,
    /// The window the trusted core's messages are for, as it last said.
    target: u32,
    /// The window the trusted core was last told the intents are from.
    from: u32,
    /// The window with the keyboard focus, which the Dock menu acts on.
    focused: u32,
    /// Where a tab was let go to make a window of its own, and the size of
    /// the window it left: that window opens there, at that size.
    torn_at: Option<WindowGeometry>,
    modifiers: ModifiersState,
    output: BufWriter<io::Stdout>,
    exit: Option<String>,
    /// Reports each frame and what it cost, for measuring the drawing.
    trace_frames: bool,
    /// When the first frame of any window reached the screen, in
    /// milliseconds from `start`; the trusted core is told it once.
    first_frame: Option<u64>,
    /// The window opens once the event loop has resumed and the trusted
    /// core's first message has come, which is the window's kept place when
    /// a session is restored (`history-and-bookmarks.md` section 15.3).
    resumed: bool,
    heard: Heard,
    /// The system clipboard, opened when first used and then held: on
    /// Linux, what was copied is served only while it is open.
    clipboard: Option<arboard::Clipboard>,
}

/// The edit a clipboard key makes with the browser's modifier; Select All
/// is the interface core's own shortcut.
fn clipboard_key(character: &str) -> Option<EditAction> {
    match character.to_ascii_lowercase().as_str() {
        "x" => Some(EditAction::Cut),
        "c" => Some(EditAction::Copy),
        "v" => Some(EditAction::Paste),
        _ => None,
    }
}

impl App {
    /// The interface before its first window: the window opens once the
    /// event loop has resumed and the trusted core has spoken.
    /// `trace_frames` reports each frame and what it cost.
    #[must_use]
    pub fn new(trace_frames: bool) -> Self {
        Self {
            start: Instant::now(),
            view: View::new(),
            window: protocol::FIRST_WINDOW,
            parked: BTreeMap::new(),
            windows: HashMap::new(),
            target: protocol::FIRST_WINDOW,
            from: protocol::FIRST_WINDOW,
            focused: protocol::FIRST_WINDOW,
            torn_at: None,
            modifiers: ModifiersState::empty(),
            output: BufWriter::new(io::stdout()),
            exit: None,
            trace_frames,
            first_frame: None,
            resumed: false,
            heard: Heard::Nothing,
            clipboard: None,
        }
    }

    /// Why the interface ended, when not at the trusted core's word; when
    /// it ended at its word, what is still buffered for it is written.
    #[must_use]
    pub fn ended(mut self) -> Option<String> {
        if self.exit.is_some() {
            return self.exit;
        }
        let _ = self.output.flush();
        None
    }

    fn now(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn scale(&self) -> f64 {
        self.view.graphics.as_ref().map_or(1.0, |graphics| graphics.window.scale_factor())
    }

    /// The window's title and the Dock menu follow the tabs: the title is
    /// the selected tab's, marked when it plays sound as a web browser
    /// marks it, so the Dock's window list names the selected tab.
    fn show_tabs(&self) {
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

    fn request_redraw(&mut self) {
        if let Some(graphics) = &mut self.view.graphics {
            graphics.asked = true;
            graphics.window.request_redraw();
        }
    }

    fn modifiers(&self) -> Modifiers {
        let command = if cfg!(target_os = "macos") { self.modifiers.super_key() } else { self.modifiers.control_key() };
        Modifiers { shift: self.modifiers.shift_key(), command, alt: self.modifiers.alt_key(), repeat: false }
    }

    /// Sends `message` from the window at hand, telling the trusted core
    /// first when the window changed (`multiple-windows.md`).
    fn send(&mut self, event_loop: &ActiveEventLoop, message: &ToCore) {
        let from = (self.window != self.from).then_some(ToCore::From(self.window));
        for message in from.iter().chain([message]) {
            if let Err(error) = write_to_core(&mut self.output, message) {
                self.exit = Some(format!("cannot write to the trusted core: {error}"));
                event_loop.exit();
                return;
            }
        }
        self.from = self.window;
    }

    /// Makes window `id` the one at hand; false when there is none.
    fn switch_view(&mut self, id: u32) -> bool {
        if id == self.window {
            return true;
        }
        let Some(mut other) = self.parked.remove(&id) else { return false };
        std::mem::swap(&mut self.view, &mut other);
        self.parked.insert(self.window, other);
        self.window = id;
        true
    }

    /// Every window's number, the one at hand among them.
    fn window_ids(&self) -> Vec<u32> {
        self.parked.keys().copied().chain([self.window]).collect()
    }

    /// Opens window `id`, as the trusted core asked: at its kept place, or
    /// where a tab was let go to make it, or where the system puts it.
    fn open_another(&mut self, event_loop: &ActiveEventLoop, id: u32, geometry: Option<WindowGeometry>) {
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
    fn close_view(&mut self, id: u32) {
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
    fn tab_out(&mut self, index: usize, x: i32, y: i32) -> Option<ToCore> {
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

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
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
    fn update_cursor(&mut self) {
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

    /// Sends on or carries out what the user did.
    fn act(&mut self, event_loop: &ActiveEventLoop) {
        self.update_cursor();
        for output in self.view.browser.take_outputs() {
            let message = match output {
                Output::Visible(visible) => {
                    // Images of pages no longer shown are dropped (requirement 9.5).
                    self.view.painter.retain_pages(|key| visible.contains(key));
                    self.view.held.retain(|key, _| visible.contains(key));
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
                Output::NewWindow { private } => ToCore::NewWindow { private },
                Output::TabToNewWindow => {
                    let Some(message) = self.tab_to_new_window() else { continue };
                    message
                }
                Output::TabOut { index, x, y } => {
                    let Some(message) = self.tab_out(index, x, y) else { continue };
                    message
                }
                Output::Setting(change) => ToCore::ChangeSetting(change),
                Output::InstallUpdate => ToCore::InstallUpdate,
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
                    self.view.ime.address = allowed;
                    self.sync_ime();
                    #[cfg(feature = "interaction")]
                    self.send(event_loop, &ToCore::Editing(allowed));
                    continue;
                }
            };
            self.send(event_loop, &message);
        }
        // The opened page's sounds play (the engine's REQ-AUD-012).
        let opened = self.view.browser.opened();
        if opened != self.view.opened {
            self.view.opened = opened;
            self.send(event_loop, &ToCore::Opened(opened));
        }
    }

    fn window_action(&mut self, event_loop: &ActiveEventLoop, action: WindowAction) {
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
    fn resize_from_edge(&self, x: f64, y: f64) -> bool {
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

    /// Enables the window's input method while the address is edited or
    /// keys go to a channel, whose composed text then goes to it.
    fn sync_ime(&mut self) {
        #[cfg(feature = "interaction")]
        let allowed = self.view.ime.address || self.view.browser.typing_target().is_some();
        #[cfg(not(feature = "interaction"))]
        let allowed = self.view.ime.address;
        if allowed != self.view.ime.allowed
            && let Some(graphics) = &self.view.graphics
        {
            graphics.window.set_ime_allowed(allowed);
            self.view.ime.allowed = allowed;
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

    /// Whether `event` is the browser's paste shortcut, the browser's
    /// modifier and V, while keys go to a channel.
    #[cfg(feature = "interaction")]
    fn is_paste(&self, event: &winit::event::KeyEvent) -> bool {
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        self.view.browser.typing_target().is_some()
            && self.browser_modifier()
            && matches!(event.key_without_modifiers(), Key::Character(ref key) if key.eq_ignore_ascii_case("v"))
    }

    /// The clipboard's text to the channel, as one PASTE; a clipboard that
    /// holds no text pastes nothing.
    #[cfg(feature = "interaction")]
    fn paste(&mut self) -> bool {
        self.clipboard_text().is_some_and(|text| self.view.browser.terminal_paste(&text))
    }

    /// The system clipboard, opened when first needed.
    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    /// The clipboard's text; `None` when it holds none.
    fn clipboard_text(&mut self) -> Option<String> {
        self.clipboard()?.get_text().ok()
    }

    /// Puts `text` on the clipboard; returns whether it is there.
    fn copy_text(&mut self, text: String) -> bool {
        self.clipboard().is_some_and(|clipboard| clipboard.set_text(text).is_ok())
    }

    /// Cut, Copy, Paste or Select All in the browser's field with the keys
    /// (`menu-bar.md` section 4); a paste while keys go to a channel goes
    /// to the channel (decision 49). Returns whether the window needs
    /// drawing again.
    fn edit_field(&mut self, action: EditAction) -> bool {
        let now = self.now();
        match action {
            EditAction::Copy => {
                if let Some(text) = self.view.browser.selected_text() {
                    self.copy_text(text);
                }
                false
            }
            // Removed only once it is on the clipboard.
            EditAction::Cut => {
                let copied = self.view.browser.selected_text().is_some_and(|text| self.copy_text(text));
                copied && self.view.browser.cut(now).is_some()
            }
            EditAction::Paste => {
                #[cfg(feature = "interaction")]
                if self.view.browser.typing_target().is_some() {
                    return self.paste();
                }
                self.clipboard_text().is_some_and(|text| self.view.browser.paste_text(&text, now))
            }
            EditAction::SelectAll => self.view.browser.key(BrowserKey::Text("a"), Modifiers { command: true, ..Modifiers::default() }, now),
        }
    }

    /// An item of the menu bar: the key it names given to the interface
    /// core, so a menu item and its key do the same (requirement 12.5).
    /// Returns whether the window needs drawing again.
    fn menu_chosen(&mut self, event_loop: &ActiveEventLoop, item: &MenuItem) -> bool {
        match item.action {
            MenuAction::Browser => {
                let Some(shortcut) = item.shortcut else { return false };
                let key = match shortcut.key {
                    ShortcutKey::Text(text) => BrowserKey::Text(text),
                    ShortcutKey::Left => BrowserKey::Left,
                    ShortcutKey::Right => BrowserKey::Right,
                };
                let modifiers = Modifiers { command: true, shift: shortcut.shift, alt: shortcut.alt, repeat: false };
                let now = self.now();
                self.view.browser.key(key, modifiers, now)
            }
            MenuAction::NewPrivateWindow => {
                self.send(event_loop, &ToCore::NewWindow { private: true });
                false
            }
            MenuAction::TabToNewWindow => {
                if let Some(message) = self.tab_to_new_window() {
                    self.send(event_loop, &message);
                }
                false
            }
            MenuAction::Edit(action) => self.edit_field(action),
            // The system answers its own items; they never come here.
            MenuAction::System(_) => false,
        }
    }

    /// The open tab moved to a window of its own.
    fn tab_to_new_window(&self) -> Option<ToCore> {
        let tab = u8::try_from(self.view.active).ok()?;
        Some(ToCore::TabToWindow { tab, window: protocol::NEW_WINDOW, index: 0 })
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
        if self.view.browser.typing_target().is_none() || self.browser_modifier() {
            return None;
        }
        let now = self.now();
        let action = match (event.state, event.repeat) {
            (ElementState::Pressed, false) => Action::Press,
            (ElementState::Pressed, true) => Action::Repeat,
            (ElementState::Released, _) => Action::Release,
        };
        let unmodified = event.key_without_modifiers();
        let parts = crate::keys::parts(action, &event.logical_key, &unmodified, event.physical_key, event.text.as_deref(), self.modifiers);
        let changed = match parts {
            Some(parts) => self.view.browser.terminal_key(parts.action, parts.key, parts.mods, parts.text.as_deref(), parts.base, now),
            // A key that types several characters: they go as text.
            None => match (&event.text, action) {
                (Some(text), Action::Press) => self.view.browser.terminal_text(text, now),
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
        self.view.browser.set_phone(PhoneUiView { status, qr: view.qr.map(|qr| (qr.size, qr.bits)), pin: view.pin });
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
        self.view.browser.set_typing(view, now);
        self.sync_ime();
    }

    /// A key of the paired phone: it acts as a key of the window's own
    /// keyboard, with no modifier (REQ-INT-138).
    #[cfg(feature = "phone-keyboard")]
    fn phone_key(&mut self, key: protocol::keyboard::PhoneKey) -> bool {
        use protocol::keyboard::PhoneKey;
        let now = self.now();
        let typing = self.view.browser.typing_target().is_some();
        match key {
            PhoneKey::Text(text) if typing => self.view.browser.terminal_text(&text, now),
            PhoneKey::Text(text) => self.view.browser.key(BrowserKey::Text(&text), Modifiers::default(), now),
            PhoneKey::Key { down, name } => {
                let mut characters = name.chars();
                let single = match (characters.next(), characters.next()) {
                    (Some(character), None) => Some(character),
                    _ => None,
                };
                if typing {
                    return crate::keys::phone_parts(down, &name)
                        .is_some_and(|parts| self.view.browser.terminal_key(parts.action, parts.key, parts.mods, parts.text.as_deref(), parts.base, now));
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
                self.view.browser.key(browser_key, Modifiers::default(), now)
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
}

impl ApplicationHandler<Incoming> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.resumed = true;
        self.open_window(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, incoming: Incoming) {
        if matches!(self.heard, Heard::Nothing) {
            self.heard = if let Incoming::Window(kept) = &incoming { Heard::Window(*kept) } else { Heard::Spoke };
            self.switch_view(protocol::FIRST_WINDOW);
            self.open_window(event_loop);
        }
        match incoming {
            Incoming::Target(window) => self.target = window,
            Incoming::OpenWindow { window, geometry } => self.open_another(event_loop, window, geometry),
            Incoming::CloseWindow(window) => self.close_view(window),
            // The Dock menu and the menu bar are the focused window's.
            Incoming::Dock(_) | Incoming::Menu(_) => {
                self.switch_view(self.focused);
                self.incoming(event_loop, incoming);
            }
            incoming => {
                // A message for a window not open, as one just closed, is
                // dropped.
                if self.switch_view(self.target) {
                    self.incoming(event_loop, incoming);
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        // Each window's events are its own view's.
        let Some(&id) = self.windows.get(&window_id) else { return };
        self.switch_view(id);
        if matches!(event, WindowEvent::Focused(true)) {
            self.focused = id;
            self.show_tabs();
        }
        if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
            self.report_window(event_loop);
        }
        // Resized or uncovered, the window may have lost what was
        // presented: every pixel is drawn again.
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Occluded(false)) {
            self.view.painter.redraw_whole();
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
        // Each window paces its own frames; the loop wakes for the first due.
        let here = self.window;
        let mut next: Option<Instant> = None;
        for id in self.window_ids() {
            self.switch_view(id);
            if let Some(at) = self.pace(event_loop) {
                next = Some(next.map_or(at, |next| next.min(at)));
            }
        }
        self.switch_view(here);
        event_loop.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}

impl App {
    /// Asks for the window at hand's next frame when it is due, and says
    /// when the one after is: none while nothing in it animates or waits to
    /// be drawn.
    fn pace(&mut self, event_loop: &ActiveEventLoop) -> Option<Instant> {
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
}

impl App {
    /// Reads a page's frame against the one held and shows it; false when
    /// it is not shown: changes to a page no longer held, or a broken frame,
    /// which ends the interface.
    fn show_frame(&mut self, event_loop: &ActiveEventLoop, key: PageKey, version: u32, image: &[u8]) -> bool {
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

    /// Opens the window once the event loop has resumed and the trusted
    /// core has spoken.
    fn open_window(&mut self, event_loop: &ActiveEventLoop) {
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

    /// A press of the middle button: on a tab, it closes.
    fn middle_pressed(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let changed = self.view.browser.middle_pressed();
        self.act(event_loop);
        changed
    }

    /// Tells the trusted core where the window is and how large, to keep
    /// with the session (requirement 20.21); while maximised, the place and
    /// size it returns to.
    fn report_window(&mut self, event_loop: &ActiveEventLoop) {
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

    fn incoming(&mut self, event_loop: &ActiveEventLoop, incoming: Incoming) {
        let now = self.now();
        match incoming {
            // Used when the window opens; it opens with the first message.
            // The windows' own messages are taken before they come here.
            Incoming::Window(_) | Incoming::Target(_) | Incoming::OpenWindow { .. } | Incoming::CloseWindow(_) => return,
            Incoming::Tabs(tabs, active) => {
                self.view.browser.set_tabs(tabs.clone(), active);
                self.view.tabs = tabs;
                self.view.active = active;
                self.show_tabs();
            }
            // The sound helper said which channels sound, through the
            // trusted core (decision 55).
            Incoming::Sounding(channels) => {
                if self.view.browser.set_sounding(channels) {
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
            Incoming::Menu(item) => {
                self.menu_chosen(event_loop, item);
                self.act(event_loop);
            }
            Incoming::Strip(strip) => self.view.browser.set_strip(strip),
            Incoming::Settings(view) => self.view.browser.set_settings(view),
            Incoming::Update(view) => self.view.browser.set_update(view),
            Incoming::Console(update) => self.view.browser.console_update(update),
            Incoming::Library(view) => match view {
                Library::History(view) => self.view.browser.set_history(view),
                Library::Cookies(view) => self.view.browser.set_cookies(view),
                Library::PageControls(controls) => self.view.browser.set_page_controls(controls),
                Library::Remote(place) => self.view.browser.set_remote(place),
                Library::Suggestions(suggestions) => self.view.browser.set_suggestions(suggestions),
                Library::TabList(list) => self.view.browser.set_tab_list(list),
                Library::Bookmarks(view) => self.view.browser.set_bookmarks(view, now),
                Library::Star(view) => self.view.browser.set_star(view),
                Library::Bar(view) => self.view.browser.set_bar(view),
            },
            Incoming::Channel(channel) => self.view.browser.set_channel(channel),
            Incoming::Arrived { channel, page, title } => self.view.browser.add_page(channel, page, title, now),
            Incoming::Updated { channel, page } => {
                if self.trace_frames {
                    eprintln!("neoteletext-interface: at {now} ms: page {page:03X} of channel {channel} updated");
                }
                self.view.browser.page_updated(channel, page, now);
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
                    Message::Page { key, links } => self.view.browser.set_links(key, links),
                    Message::Pointing(pointing) => self.view.browser.set_pointing(pointing),
                    Message::Prompt(prompt) => self.view.browser.set_prompt(prompt, now),
                    Message::Show(key) => self.view.browser.show(key, now),
                    Message::Aliases { channel, numbers } => self.view.browser.set_aliases(channel, numbers),
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
        self.view.dirty = true;
    }

    fn window_input(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
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

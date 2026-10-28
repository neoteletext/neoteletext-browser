//! The interface's state and its event loop's handler: the window at hand
//! and the others set aside, what the trusted core said, and what the user
//! does, sent on as intents and strip input.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::rc::Rc;
use std::time::Instant;

use draw_vello::{Family, Painter};
use interface_core::browser::{Browser, Cursor, TabInfo};
use interface_core::canvas::PageKey;
use interface_core::library::WindowGeometry;
use protocol::ToCore;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::incoming::Incoming;
use crate::window::FRAME;

mod clipboard;
mod frame;
mod ime;
mod incoming;
mod input;
mod outputs;
mod pace;
#[cfg(feature = "phone-keyboard")]
mod phone;
#[cfg(feature = "interaction")]
mod typing;
mod windows;

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
    /// What the window has drawn and has still to draw.
    pace: pace::Pace,
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
    /// The window shows the page alone (decision 85): whether it was full
    /// screen already, the system's own, which leaving keeps; and whether
    /// the system has been seen to make it full screen since, so that its
    /// leaving is the system's ending of it.
    page_full_screen: Option<(bool, bool)>,
    /// The pointer is hidden, as full screen hides a still one.
    pointer_hidden: bool,
}

impl View {
    fn new() -> Self {
        Self {
            graphics: None,
            browser: Browser::new(FRAME),
            painter: Painter::new(Family::Roboto),
            cursor: (0.0, 0.0),
            pace: pace::Pace::default(),
            pointer_shape: Cursor::Arrow,
            opened: None,
            tabs: Vec::new(),
            active: 0,
            ime: InputMethod::default(),
            held: HashMap::new(),
            reported: None,
            normal: None,
            page_full_screen: None,
            pointer_hidden: false,
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
    output: outputs::CorePipe<io::Stdout>,
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
            output: outputs::CorePipe::new(io::stdout()),
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
        let _ = self.output.handled();
        None
    }

    fn now(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn scale(&self) -> f64 {
        self.view.graphics.as_ref().map_or(1.0, |graphics| graphics.window.scale_factor())
    }

    /// Sends `message` from the window at hand, telling the trusted core
    /// first when the window changed (`multiple-windows.md`).
    fn send(&mut self, event_loop: &ActiveEventLoop, message: &ToCore) {
        let from = (self.window != self.from).then_some(ToCore::From(self.window));
        for message in from.iter().chain([message]) {
            if let Err(error) = self.output.send(message) {
                self.exit = Some(format!("cannot write to the trusted core: {error}"));
                event_loop.exit();
                return;
            }
        }
        self.from = self.window;
    }

    /// Writes what is buffered for the trusted core, once an event has
    /// been handled (decision 53: once per event, not per message); false,
    /// with the loop ending, when it cannot be written.
    fn flush(&mut self, event_loop: &ActiveEventLoop) -> bool {
        match self.output.handled() {
            Ok(()) => true,
            Err(error) => {
                self.exit = Some(format!("cannot write to the trusted core: {error}"));
                event_loop.exit();
                false
            }
        }
    }

    /// Every window's number, the one at hand among them.
    fn window_ids(&self) -> Vec<u32> {
        self.parked.keys().copied().chain([self.window]).collect()
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
        // What it sent, a phone's key or a menu's choice, goes before any
        // frame it asked for is drawn (REQ-RCH-003).
        self.flush(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        // Each window's events are its own view's.
        let Some(&id) = self.windows.get(&window_id) else { return };
        self.switch_view(id);
        if matches!(event, WindowEvent::Focused(true)) {
            self.focused = id;
            self.show_tabs();
        }
        if matches!(event, WindowEvent::Resized(_)) {
            self.full_screen_resized();
        }
        // Full screen's size is not the window's, to keep.
        if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) && self.view.page_full_screen.is_none() {
            self.report_window(event_loop);
        }
        // Resized or uncovered, the window may have lost what was
        // presented: every pixel is drawn again.
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Occluded(false)) {
            self.view.painter.redraw_whole();
            self.request_redraw();
        }
        self.window_input(event_loop, window_id, event);
        // The input goes to the trusted core as soon as it has been
        // handled, before the frame it asked for is drawn: winit on macOS
        // draws the frames asked for before `about_to_wait` (REQ-RCH-003).
        self.flush(event_loop);
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device: winit::event::DeviceId, _event: winit::event::DeviceEvent) {}

    /// Redraws while something animates; otherwise sleeps until the next
    /// event.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // What the events produced goes to the trusted core before waiting.
        if !self.flush(event_loop) {
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

//! The compositor's state: only the protocols a window needs.

use std::ffi::OsString;
use std::process::Child;
use std::sync::Arc;

use smithay::desktop::{Space, Window, WindowSurfaceType};
use smithay::input::keyboard::XkbConfig;
use smithay::input::{Seat, SeatState};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::compositor::{CompositorClientState, CompositorState};
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::socket::ListeningSocketSource;

use crate::accessibility::Accessibility;
use crate::text_input::{self, TextInputs};

use crate::strip::{STRIP_HEIGHT, Strip};

/// Key repeat as most desktops default to it.
const REPEAT_DELAY_MILLISECONDS: i32 = 200;
const REPEAT_RATE_PER_SECOND: i32 = 25;

pub struct HostState {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub loop_signal: LoopSignal,
    pub space: Space<Window>,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub data_device_state: DataDeviceState,
    pub accessibility: Option<Accessibility>,
    pub text_inputs: TextInputs,
    /// Keys whose press was passed on; a release is passed on only for these,
    /// since the input method keeps the presses it composes with.
    pub keys_down: Vec<u32>,
    pub seat_state: SeatState<HostState>,
    pub seat: Seat<HostState>,
    pub strip: Strip,
    /// Where hosted windows are shown: everything below the strip.
    pub nested_area: Rectangle<i32, Logical>,
    pub child: Option<Child>,
}

impl HostState {
    pub fn new(event_loop: &mut EventLoop<Self>, display: Display<Self>) -> Result<Self, Box<dyn std::error::Error>> {
        let display_handle = display.handle();
        // Deliberately absent: virtual keyboard and pointer, screen copy,
        // layer shell, clipboard control without focus, the input method
        // protocol (which would let a client be the input method),
        // activation. Text input is present: through it the hosted window
        // receives text composed by the user's input method, which the
        // display host itself talks to (decision 32). The data device is present because GTK 3 sets up no
        // input without it; it gives a client the clipboard only while that
        // client has the keyboard, and this clipboard never leaves the nested
        // display.
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        text_input::create_global(&display_handle);
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&display_handle, "user");
        seat.add_keyboard(XkbConfig::default(), REPEAT_DELAY_MILLISECONDS, REPEAT_RATE_PER_SECOND)?;
        seat.add_pointer();

        let listening = ListeningSocketSource::new_auto()?;
        let socket_name = listening.socket_name().to_os_string();
        let handle = event_loop.handle();
        handle.insert_source(listening, |stream, (), state| {
            if let Err(error) = state.display_handle.insert_client(stream, Arc::new(ClientState::default())) {
                eprintln!("display-host-spike: refused a client: {error}");
            }
        })?;
        handle.insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
            // SAFETY: the display is owned by this event source and is not
            // dropped while it dispatches.
            #[allow(unsafe_code, reason = "calloop gives the display only through an unsafe accessor")]
            let display = unsafe { display.get_mut() };
            display.dispatch_clients(state)?;
            Ok(PostAction::Continue)
        })?;

        Ok(Self {
            start_time: std::time::Instant::now(),
            socket_name,
            display_handle,
            loop_signal: event_loop.get_signal(),
            space: Space::default(),
            compositor_state,
            xdg_shell_state,
            shm_state,
            data_device_state,
            accessibility: None,
            text_inputs: TextInputs::default(),
            keys_down: Vec::new(),
            seat_state,
            seat,
            strip: Strip::new(),
            nested_area: Rectangle::default(),
            child: None,
        })
    }

    /// The whole window is `size`; the strip takes the top, windows the rest.
    pub fn set_window_size(&mut self, size: Size<i32, Logical>) {
        self.nested_area = Rectangle::new((0, STRIP_HEIGHT).into(), (size.w, (size.h - STRIP_HEIGHT).max(1)).into());
        self.strip.set_width(size.w);
        let windows: Vec<Window> = self.space.elements().cloned().collect();
        for window in windows {
            self.place(&window);
        }
    }

    /// Every window fills the nested area, wherever it asked to be.
    pub fn place(&mut self, window: &Window) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|pending| pending.size = Some(self.nested_area.size));
            if toplevel.is_initial_configure_sent() {
                toplevel.send_pending_configure();
            }
        }
        self.space.map_element(window.clone(), self.nested_area.loc, false);
    }

    /// The screen reader's view follows the strip.
    pub fn accessibility_changed(&mut self) {
        let (text, focused) = (self.strip.text().to_owned(), self.strip.focused());
        let width = f64::from(self.nested_area.size.w);
        let height = f64::from(self.nested_area.loc.y + self.nested_area.size.h);
        if let Some(accessibility) = self.accessibility.as_mut() {
            accessibility.update(&text, focused, (width, height), f64::from(STRIP_HEIGHT));
        }
    }

    pub fn window_focus_changed(&mut self, focused: bool) {
        if let Some(accessibility) = self.accessibility.as_mut() {
            accessibility.window_focused(focused);
        }
    }

    pub fn surface_under(&self, position: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        if !self.nested_area.to_f64().contains(position) {
            return None;
        }
        self.space.element_under(position).and_then(|(window, location)| {
            window
                .surface_under(position - location.to_f64(), WindowSurfaceType::ALL)
                .map(|(surface, point)| (surface, (point + location).to_f64()))
        })
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

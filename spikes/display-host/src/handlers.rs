//! Protocol handlers. Requests that would let a window place itself, cover
//! others or take focus are answered by placing it in the nested area.

use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::desktop::Window;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::reexports::wayland_server::protocol::{wl_buffer, wl_output, wl_seat, wl_surface::WlSurface};
use smithay::utils::Serial;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    CompositorClientState, CompositorHandler, CompositorState, get_parent, is_sync_subsurface,
};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler, set_data_device_focus,
};
use smithay::wayland::shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm, delegate_xdg_shell,
};

use crate::state::{ClientState, HostState};

impl CompositorHandler for HostState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Every client is inserted with a ClientState (state.rs).
        &client.get_data::<ClientState>().expect("client inserted without its state").compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if is_sync_subsurface(surface) {
            return;
        }
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let window = self.space.elements().find(|window| window.toplevel().is_some_and(|toplevel| toplevel.wl_surface() == &root)).cloned();
        if let Some(window) = window {
            window.on_commit();
            if let Some(toplevel) = window.toplevel()
                && !toplevel.is_initial_configure_sent()
            {
                toplevel.send_configure();
            }
        }
    }
}

impl BufferHandler for HostState {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for HostState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for HostState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    /// The clipboard follows the keyboard: only the focused client may use it.
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|surface| self.display_handle.get_client(surface.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client);
        self.text_inputs.set_focus(focused.cloned());
    }
}

impl SelectionHandler for HostState {
    type SelectionUserData = ();
}

impl DataDeviceHandler for HostState {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for HostState {}
impl ServerDndGrabHandler for HostState {}

impl OutputHandler for HostState {}

impl XdgShellHandler for HostState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    /// A new window never takes the keyboard: focus moves only when the user
    /// clicks (input.rs).
    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        self.place(&window);
    }

    // Pop-ups are not hosted in the proof; a real host would keep them
    // inside the nested area too.
    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn reposition_request(&mut self, _surface: PopupSurface, _positioner: PositionerState, _token: u32) {}

    /// Full screen means the nested area, never the strip.
    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<wl_output::WlOutput>) {
        let window = self.space.elements().find(|window| window.toplevel() == Some(&surface)).cloned();
        if let Some(window) = window {
            self.place(&window);
        }
    }
}

delegate_compositor!(HostState);
delegate_shm!(HostState);
delegate_seat!(HostState);
delegate_data_device!(HostState);
delegate_output!(HostState);
delegate_xdg_shell!(HostState);

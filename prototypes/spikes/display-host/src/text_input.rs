//! The text-input protocol (text-input-unstable-v3), served by the display
//! host itself. Smithay's implementation only relays to an input method
//! client connected to the compositor, which the display host never allows;
//! here the display host is the input method's side: it delivers text the
//! user's input method composed, and nothing else, to the focused window.

use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_manager_v3::{
    self, ZwpTextInputManagerV3,
};
use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{self, ZwpTextInputV3};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::ClientId,
};

use crate::state::HostState;

/// The protocol version served.
const VERSION: u32 = 1;

struct Instance {
    object: ZwpTextInputV3,
    /// Commit requests received; `done` must carry this count (protocol,
    /// `zwp_text_input_v3.done`).
    commits: u32,
    pending_enable: Option<bool>,
    enabled: bool,
    entered: Option<WlSurface>,
}

#[derive(Default)]
pub struct TextInputs {
    instances: Vec<Instance>,
    focus: Option<WlSurface>,
}

pub fn create_global(display_handle: &DisplayHandle) {
    display_handle.create_global::<HostState, ZwpTextInputManagerV3, ()>(VERSION, ());
}

fn same_client(object: &ZwpTextInputV3, surface: &WlSurface) -> bool {
    object.id().same_client_as(&surface.id())
}

impl TextInputs {
    /// Follows the keyboard: `leave` for the old focus, `enter` for the new.
    pub fn set_focus(&mut self, focus: Option<WlSurface>) {
        for instance in &mut self.instances {
            if let Some(surface) = instance.entered.take()
                && surface.is_alive()
            {
                instance.object.leave(&surface);
            }
        }
        self.focus = focus;
        for instance in &mut self.instances {
            Self::enter(instance, self.focus.as_ref());
        }
    }

    fn enter(instance: &mut Instance, focus: Option<&WlSurface>) {
        if let Some(surface) = focus
            && same_client(&instance.object, surface)
        {
            instance.object.enter(surface);
            instance.entered = Some(surface.clone());
        }
    }

    /// Composed text, or text still being composed, for the focused window's
    /// enabled text inputs.
    pub fn deliver(&mut self, preedit: Option<(&str, i32, i32)>, commit: Option<&str>) {
        for instance in self.instances.iter().filter(|instance| instance.enabled && instance.entered.is_some()) {
            match preedit {
                Some((text, begin, end)) => instance.object.preedit_string(Some(text.to_owned()), begin, end),
                None => instance.object.preedit_string(None, 0, 0),
            }
            if let Some(text) = commit {
                instance.object.commit_string(Some(text.to_owned()));
            }
            instance.object.done(instance.commits);
        }
    }
}

impl GlobalDispatch<ZwpTextInputManagerV3, ()> for HostState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwpTextInputManagerV3>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwpTextInputManagerV3, ()> for HostState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZwpTextInputManagerV3,
        request: zwp_text_input_manager_v3::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let zwp_text_input_manager_v3::Request::GetTextInput { id, .. } = request {
            let mut instance =
                Instance { object: data_init.init(id, ()), commits: 0, pending_enable: None, enabled: false, entered: None };
            // Toolkits create these after they gain the keyboard.
            TextInputs::enter(&mut instance, state.text_inputs.focus.as_ref());
            state.text_inputs.instances.push(instance);
        }
    }
}

impl Dispatch<ZwpTextInputV3, ()> for HostState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ZwpTextInputV3,
        request: zwp_text_input_v3::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let Some(instance) = state.text_inputs.instances.iter_mut().find(|instance| &instance.object == resource) else {
            return;
        };
        match request {
            zwp_text_input_v3::Request::Enable => instance.pending_enable = Some(true),
            zwp_text_input_v3::Request::Disable => instance.pending_enable = Some(false),
            zwp_text_input_v3::Request::Commit => {
                instance.commits = instance.commits.wrapping_add(1);
                if let Some(enable) = instance.pending_enable.take() {
                    instance.enabled = enable;
                }
            }
            zwp_text_input_v3::Request::Destroy => {
                state.text_inputs.instances.retain(|instance| &instance.object != resource);
            }
            // Surrounding text, content type and cursor position are not
            // needed to deliver composed text.
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &ZwpTextInputV3, _data: &()) {
        state.text_inputs.instances.retain(|instance| &instance.object != resource);
    }
}

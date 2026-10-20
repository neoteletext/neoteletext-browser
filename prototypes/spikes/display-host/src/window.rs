//! The display host's own window on the user's desktop, through winit, drawn
//! with Smithay's OpenGL ES renderer: hosted windows first, the strip last,
//! above everything. Smithay's own winit back end does not pass on input
//! method events, which the display host needs (decision 32), so the window
//! is set up here.

use std::sync::Arc;
use std::time::Duration;

use smithay::backend::egl::context::{GlAttributes, PixelFormatRequirements};
use smithay::backend::egl::native::XlibWindow;
use smithay::backend::egl::{EGLContext, EGLDisplay, EGLSurface};
use smithay::backend::renderer::Bind;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::winit::application::ApplicationHandler;
use smithay::reexports::winit::event::WindowEvent;
use smithay::reexports::winit::event_loop::{ActiveEventLoop, EventLoop};
use smithay::reexports::winit::platform::pump_events::EventLoopExtPumpEvents;
use smithay::reexports::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use smithay::reexports::winit::window::{Window, WindowAttributes, WindowId};
use smithay::utils::{Physical, Size, Transform};

use crate::state::HostState;

/// Millihertz, as Wayland outputs report refresh rates.
const REFRESH_MILLIHERTZ: i32 = 60_000;
const BACKGROUND: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WINDOW_TITLE: &str = "NeoTeletext";
/// OpenGL ES 3.0, as Smithay's renderer requires.
const GL_VERSION: (u8, u8) = (3, 0);

smithay::backend::renderer::element::render_elements! {
    pub HostElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
}

pub struct HostWindow {
    pub window: Arc<Window>,
    // Kept alive for the surface and renderer, never used directly.
    _display: EGLDisplay,
    surface: EGLSurface,
    renderer: GlesRenderer,
    output: Output,
    damage_tracker: OutputDamageTracker,
    on_wayland: bool,
}

impl HostWindow {
    #[allow(unsafe_code, reason = "OpenGL set-up on a native window is unsafe in Smithay's interface; the window outlives every object made from it")]
    pub fn new(event_loop: &EventLoop<()>, state: &mut HostState) -> Result<Self, Box<dyn std::error::Error>> {
        #[allow(deprecated, reason = "the window is needed before the event loop first runs, as Smithay's own winit back end does")]
        let window = Arc::new(event_loop.create_window(WindowAttributes::default().with_title(WINDOW_TITLE))?);
        // Composed text from the user's input method arrives as Ime events.
        window.set_ime_allowed(true);

        // SAFETY: the window is kept alive by the Arc held in this struct.
        let display = unsafe { EGLDisplay::new(window.clone())? };
        let attributes = GlAttributes { version: GL_VERSION, profile: None, debug: false, vsync: false };
        let context = EGLContext::new_with_config(&display, attributes, PixelFormatRequirements::_8_bit())?;
        let pixel_format = context.pixel_format().ok_or("no pixel format")?;
        let size = window.inner_size();
        let (width, height) = (i32::try_from(size.width)?, i32::try_from(size.height)?);
        let (surface, on_wayland) = match window.window_handle()?.as_raw() {
            RawWindowHandle::Wayland(handle) => {
                // SAFETY: the surface pointer comes from the live window.
                let native = unsafe { wayland_egl::WlEglSurface::new_from_raw(handle.surface.as_ptr().cast(), width, height)? };
                // SAFETY: the native surface is moved into the EGL surface.
                (unsafe { EGLSurface::new(&display, pixel_format, context.config_id(), native)? }, true)
            }
            RawWindowHandle::Xlib(handle) => {
                // SAFETY: the X window belongs to the live window.
                (unsafe { EGLSurface::new(&display, pixel_format, context.config_id(), XlibWindow(handle.window))? }, false)
            }
            _ => return Err("the display host runs only on Wayland or X11 desktops".into()),
        };
        let _ = context.unbind();
        // SAFETY: the context is current only through this renderer.
        let renderer = unsafe { GlesRenderer::new(context)? };

        let mode = Mode { size: (width, height).into(), refresh: REFRESH_MILLIHERTZ };
        let output = Output::new(
            "neoteletext-display-host".into(),
            PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "NeoTeletext".into(), model: "display host".into() },
        );
        let _global = output.create_global::<HostState>(&state.display_handle);
        // The window's framebuffer is upside down, as in Smithay's own back end.
        output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
        output.set_preferred(mode);
        state.space.map_output(&output, (0, 0));
        state.set_window_size(mode.size.to_logical(1));
        let damage_tracker = OutputDamageTracker::from_output(&output);
        Ok(Self { window, _display: display, surface, renderer, output, damage_tracker, on_wayland })
    }

    fn resized(&mut self, state: &mut HostState, size: Size<i32, Physical>) {
        self.output.change_current_state(Some(Mode { size, refresh: REFRESH_MILLIHERTZ }), None, None, None);
        state.set_window_size(size.to_logical(1));
        self.report_bounds(state);
    }

    /// Tells the screen reader where the window is on the user's screen.
    pub fn report_bounds(&self, state: &mut HostState) {
        let size = self.window.inner_size();
        let inner_origin = self.window.inner_position().unwrap_or_default();
        let outer_origin = self.window.outer_position().unwrap_or_default();
        let outer_size = self.window.outer_size();
        let rectangle = |x: i32, y: i32, width: u32, height: u32| accesskit::Rect {
            x0: f64::from(x),
            y0: f64::from(y),
            x1: f64::from(x) + f64::from(width),
            y1: f64::from(y) + f64::from(height),
        };
        if let Some(accessibility) = state.accessibility.as_mut() {
            accessibility.window_bounds(
                rectangle(outer_origin.x, outer_origin.y, outer_size.width, outer_size.height),
                rectangle(inner_origin.x, inner_origin.y, size.width, size.height),
            );
        }
        state.accessibility_changed();
    }

    pub fn draw(&mut self, state: &mut HostState) {
        let size = self.window.inner_size();
        let (Ok(width), Ok(height)) = (i32::try_from(size.width), i32::try_from(size.height)) else { return };
        if self.on_wayland {
            self.surface.resize(width, height, 0, 0);
        }
        {
            let Ok(mut framebuffer) = self.renderer.bind(&mut self.surface) else { return };
            let strip = [HostElement::Solid(state.strip.render_element())];
            if let Err(error) = smithay::desktop::space::render_output::<_, HostElement, _, _>(
                &self.output,
                &mut self.renderer,
                &mut framebuffer,
                1.0,
                0,
                [&state.space],
                &strip,
                &mut self.damage_tracker,
                BACKGROUND,
            ) {
                eprintln!("display-host-spike: drawing failed: {error}");
            }
        }
        self.window.pre_present_notify();
        if let Err(error) = self.surface.swap_buffers(None) {
            eprintln!("display-host-spike: presenting failed: {error}");
        }
        for window in state.space.elements() {
            window.send_frame(&self.output, state.start_time.elapsed(), Some(Duration::ZERO), |_, _| Some(self.output.clone()));
        }
        state.space.refresh();
        let _ = state.display_handle.flush_clients();
    }
}

/// Runs winit's pending events through the host, without waiting.
pub fn pump(event_loop: &mut EventLoop<()>, window: &mut HostWindow, state: &mut HostState) {
    let _ = event_loop.pump_app_events(Some(Duration::ZERO), &mut Pump { window, state });
}

struct Pump<'a> {
    window: &'a mut HostWindow,
    state: &'a mut HostState,
}

impl ApplicationHandler for Pump<'_> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) => {
                if let (Ok(width), Ok(height)) = (i32::try_from(size.width), i32::try_from(size.height)) {
                    self.window.resized(self.state, (width, height).into());
                }
            }
            WindowEvent::CloseRequested => self.state.loop_signal.stop(),
            WindowEvent::Focused(focused) => self.state.window_focus_changed(focused),
            other => self.state.window_input(other),
        }
    }
}

//! The React prototype's tab with no interface framework. Our own code holds
//! the state, lays out the header and the wall, hit-tests the pointer,
//! scrolls and animates; tiny-skia draws every pixel on the CPU; softbuffer
//! shows the result in a winit window. Text uses bitmap fonts, the same a
//! microcontroller build would draw.

mod draw;
mod pages;
mod size;
mod tab;
mod text;
mod tween;

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const WINDOW_TITLE: &str = "NeoTeletext: tiny-skia page wall, no framework";
const WINDOW_WIDTH: f64 = 1280.0;
const WINDOW_HEIGHT: f64 = 820.0;
/// Logical pixels scrolled for one wheel line.
const LINE_HEIGHT: f32 = 21.0;

struct Graphics {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    /// Kept for the surface's lifetime.
    _context: Context<Rc<Window>>,
}

struct App {
    graphics: Option<Graphics>,
    tab: tab::Tab,
    pixmap: Option<tiny_skia::Pixmap>,
}

impl App {
    fn redraw(&mut self) {
        let Some(graphics) = self.graphics.as_mut() else { return };
        let size = graphics.window.inner_size();
        let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if graphics.surface.resize(width, height).is_err() {
            return;
        }
        let pixmap = match self.pixmap.take() {
            Some(pixmap) if pixmap.width() == size.width && pixmap.height() == size.height => pixmap,
            _ => match tiny_skia::Pixmap::new(size.width, size.height) {
                Some(pixmap) => pixmap,
                None => return,
            },
        };
        let mut pixmap = pixmap;
        #[allow(clippy::cast_possible_truncation, reason = "display scale factors are small")]
        let scale = graphics.window.scale_factor() as f32;
        self.tab.draw(&mut pixmap, scale, Instant::now());

        let Ok(mut buffer) = graphics.surface.buffer_mut() else { return };
        // softbuffer wants 0RGB words; every pixel drawn is opaque, so the
        // premultiplied RGBA bytes are the colour itself.
        for (word, pixel) in buffer.iter_mut().zip(pixmap.data().chunks_exact(4)) {
            *word = u32::from(pixel[0]) << 16 | u32::from(pixel[1]) << 8 | u32::from(pixel[2]);
        }
        graphics.window.pre_present_notify();
        let _ = buffer.present();
        self.pixmap = Some(pixmap);
    }

    fn scale(&self) -> f32 {
        #[allow(clippy::cast_possible_truncation, reason = "display scale factors are small")]
        self.graphics.as_ref().map_or(1.0, |graphics| graphics.window.scale_factor() as f32)
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }
        let attributes =
            Window::default_attributes().with_title(WINDOW_TITLE).with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
        let Ok(window) = event_loop.create_window(attributes) else {
            event_loop.exit();
            return;
        };
        let window = Rc::new(window);
        let Ok(context) = Context::new(window.clone()) else {
            event_loop.exit();
            return;
        };
        let Ok(surface) = Surface::new(&context, window.clone()) else {
            event_loop.exit();
            return;
        };
        self.graphics = Some(Graphics { window, surface, _context: context });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        let scale = self.scale();
        let now = Instant::now();
        let changed = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                false
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                false
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => true,
            #[allow(clippy::cast_possible_truncation, reason = "window coordinates fit easily in f32")]
            WindowEvent::CursorMoved { position, .. } => {
                self.tab.pointer_moved(position.x as f32 / scale, position.y as f32 / scale, now)
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => self.tab.pointer_pressed(now),
                ElementState::Released => self.tab.pointer_released(),
            },
            WindowEvent::MouseWheel { delta, .. } => {
                #[allow(clippy::cast_possible_truncation, reason = "scroll distances fit easily in f32")]
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * LINE_HEIGHT,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 / scale,
                };
                self.tab.scroll_by(-lines)
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => match event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => self.tab.step(-1, now),
                Key::Named(NamedKey::ArrowRight) => self.tab.step(1, now),
                _ => false,
            },
            _ => false,
        };
        if changed && let Some(graphics) = &self.graphics {
            graphics.window.request_redraw();
        }
    }

    /// Redraws while something animates or a layout switch is waiting out
    /// its dwell; otherwise sleeps until the next event.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self.tab.settle(now) && let Some(graphics) = &self.graphics {
            graphics.window.request_redraw();
        }
        if self.tab.animating(now) {
            if let Some(graphics) = &self.graphics {
                graphics.window.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::Poll);
        } else if let Some(deadline) = self.tab.next_deadline() {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn main() {
    let Ok(event_loop) = EventLoop::new() else {
        eprintln!("skia-wall-spike: cannot open the event loop");
        return;
    };
    let mut app = App { graphics: None, tab: tab::Tab::new(), pixmap: None };
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("skia-wall-spike: {error}");
    }
}

//! The React prototype's tab with no interface framework and no GPU. Our
//! own code holds the state, lays out the header and the wall, hit-tests the
//! pointer, scrolls and animates; `vello_cpu` draws every pixel on the CPU;
//! Parley lays out and shapes the text; softbuffer shows the result in a
//! winit window.

mod pages;
mod paint;
mod pose;
mod size;
mod tab;
mod tween;

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;

use softbuffer::{Context, Surface};
use vello_cpu::Pixmap;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const WINDOW_TITLE: &str = "T42 Browser: vello_cpu and Parley page wall, no framework, no GPU";
const WINDOW_WIDTH: f64 = 1280.0;
const WINDOW_HEIGHT: f64 = 820.0;
/// Logical pixels scrolled for one wheel line.
const LINE_HEIGHT: f64 = 21.0;

struct Graphics {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    /// Kept for the surface's lifetime.
    _context: Context<Rc<Window>>,
}

struct App {
    graphics: Option<Graphics>,
    tab: tab::Tab,
    painter: Option<paint::Painter>,
    target: Option<Pixmap>,
}

impl App {
    fn scale(&self) -> f64 {
        self.graphics.as_ref().map_or(1.0, |graphics| graphics.window.scale_factor())
    }

    fn redraw(&mut self) {
        let Some(graphics) = self.graphics.as_mut() else { return };
        let size = graphics.window.inner_size();
        let (Ok(width), Ok(height)) = (u16::try_from(size.width), u16::try_from(size.height)) else { return };
        let (Some(surface_width), Some(surface_height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if graphics.surface.resize(surface_width, surface_height).is_err() {
            return;
        }
        let painter = self.painter.get_or_insert_with(|| paint::Painter::new(width, height));
        if painter.context.width() != width || painter.context.height() != height {
            painter.context.reset_and_resize(width, height);
        } else {
            painter.context.reset();
        }
        painter.scale = graphics.window.scale_factor();
        let target = match self.target.take() {
            Some(target) if target.width() == width && target.height() == height => target,
            _ => Pixmap::new(width, height),
        };
        let mut target = target;

        let logical = (f64::from(width) / painter.scale, f64::from(height) / painter.scale);
        self.tab.draw(painter, logical.0, logical.1, Instant::now());
        painter.context.flush();
        painter.context.render(&mut target, &mut painter.resources);

        let Ok(mut buffer) = graphics.surface.buffer_mut() else { return };
        // softbuffer wants 0RGB words; every pixel drawn is opaque, so the
        // premultiplied colour is the colour itself.
        for (word, pixel) in buffer.iter_mut().zip(target.data()) {
            *word = u32::from(pixel.r) << 16 | u32::from(pixel.g) << 8 | u32::from(pixel.b);
        }
        graphics.window.pre_present_notify();
        let _ = buffer.present();
        self.target = Some(target);
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
            WindowEvent::CursorMoved { position, .. } => self.tab.pointer_moved(position.x / scale, position.y / scale, now),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => self.tab.pointer_pressed(now),
                ElementState::Released => self.tab.pointer_released(),
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let distance = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y) * LINE_HEIGHT,
                    MouseScrollDelta::PixelDelta(position) => position.y / scale,
                };
                self.tab.scroll_by(-distance)
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
        eprintln!("vello-wall-spike: cannot open the event loop");
        return;
    };
    let mut app = App { graphics: None, tab: tab::Tab::new(), painter: None, target: None };
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("vello-wall-spike: {error}");
    }
}

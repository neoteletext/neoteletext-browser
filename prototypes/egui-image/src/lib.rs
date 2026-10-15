//! An egui app drawn into a `<canvas>` so the host page can size, scale and
//! CSS-transform it like an image.
//!
//! The host owns the DOM: it sizes the canvas, maps pointer events into the
//! canvas's own (untransformed) coordinate space, and calls [`EguiImage::frame`]
//! when egui asks for a repaint. That split is what lets an arbitrarily
//! transformed canvas still receive pointer input at the right spot.

mod picture;

use std::sync::Arc;

use egui::{Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, Rect, TouchPhase, Vec2, ViewportId};
use egui_glow::glow;
use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

use picture::{Picture, PictureApp};

// Behind the picture while its texture uploads; matches the host's frame
// placeholder so the swap is invisible.
const CLEAR_COLOR: [f32; 4] = [0.11, 0.125, 0.16, 1.0];

#[wasm_bindgen]
pub struct EguiImage {
    ctx: egui::Context,
    painter: egui_glow::Painter,
    app: PictureApp,
    events: Vec<Event>,
    css_size: Vec2,
    pixel_size: [u32; 2],
    cursor: egui::CursorIcon,
}

#[wasm_bindgen]
impl EguiImage {
    /// `rgba` is unmultiplied 8-bit RGBA, `width * height * 4` bytes.
    ///
    /// # Errors
    /// Fails when the picture size does not match its data, or when the
    /// canvas cannot provide a WebGL 2 context.
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: &HtmlCanvasElement, title: &str, width: u32, height: u32, rgba: Vec<u8>) -> Result<EguiImage, JsError> {
        let picture = Picture::new(title, width, height, rgba).map_err(|message| JsError::new(&message))?;
        let webgl = canvas
            .get_context("webgl2")
            .map_err(|_| JsError::new("WebGL 2 context request failed"))?
            .ok_or_else(|| JsError::new("WebGL 2 is unavailable (too many live contexts?)"))?
            .dyn_into::<WebGl2RenderingContext>()
            .map_err(|_| JsError::new("Canvas returned a context that is not WebGL 2"))?;
        #[allow(
            clippy::arc_with_non_send_sync,
            reason = "egui_glow::Painter::new takes Arc<glow::Context>; wasm is single-threaded, so it never crosses threads"
        )]
        let gl = Arc::new(glow::Context::from_webgl2_context(webgl));
        let painter = egui_glow::Painter::new(gl, "", None, true).map_err(|error| JsError::new(&error.to_string()))?;
        Ok(EguiImage {
            ctx: egui::Context::default(),
            painter,
            app: PictureApp::new(picture),
            events: Vec::new(),
            css_size: Vec2::ZERO,
            pixel_size: [0, 0],
            cursor: egui::CursorIcon::Default,
        })
    }

    /// `css_*` is the canvas's layout size (unaffected by CSS transforms);
    /// `pixel_*` is its backing store, normally layout size × devicePixelRatio.
    pub fn resize(&mut self, css_width: f32, css_height: f32, pixel_width: u32, pixel_height: u32) {
        self.css_size = Vec2::new(css_width, css_height);
        self.pixel_size = [pixel_width, pixel_height];
    }

    /// Positions are in canvas CSS pixels, already mapped through any transform.
    pub fn pointer_moved(&mut self, x: f32, y: f32) {
        self.events.push(Event::PointerMoved(Pos2::new(x, y)));
    }

    pub fn pointer_left(&mut self) {
        self.events.push(Event::PointerGone);
    }

    /// `button` follows `MouseEvent.button`: 0 primary, 1 middle, 2 secondary.
    pub fn pointer_button(&mut self, x: f32, y: f32, button: u8, pressed: bool) {
        let button = match button {
            1 => PointerButton::Middle,
            2 => PointerButton::Secondary,
            _ => PointerButton::Primary,
        };
        self.events.push(Event::PointerButton { pos: Pos2::new(x, y), button, pressed, modifiers: Modifiers::default() });
    }

    /// `delta_*` and `delta_mode` follow `WheelEvent`. The host forwards only
    /// pinch gestures (which browsers report as ctrl+wheel), so plain
    /// scrolling keeps scrolling the page.
    pub fn pinch(&mut self, delta_x: f32, delta_y: f32, delta_mode: u32) {
        let unit = match delta_mode {
            1 => MouseWheelUnit::Line,
            2 => MouseWheelUnit::Page,
            _ => MouseWheelUnit::Point,
        };
        // WheelEvent deltas say which way the viewport moves; egui's say
        // which way the content moves.
        self.events.push(Event::MouseWheel {
            unit,
            delta: Vec2::new(-delta_x, -delta_y),
            phase: TouchPhase::Move,
            modifiers: Modifiers { ctrl: true, command: true, ..Modifiers::default() },
        });
    }

    /// Runs one egui pass and paints it. Returns how many seconds until egui
    /// wants the next frame, or infinity when it is idle until new input.
    pub fn frame(&mut self, time_seconds: f64) -> f64 {
        if self.pixel_size.contains(&0) {
            return f64::INFINITY;
        }
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.css_size)),
            time: Some(time_seconds),
            max_texture_side: Some(self.painter.max_texture_side()),
            focused: true,
            events: std::mem::take(&mut self.events),
            ..RawInput::default()
        };
        input.viewports.entry(ViewportId::ROOT).or_default().native_pixels_per_point = Some(self.pixels_per_point());

        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        self.cursor = output.platform_output.cursor_icon;

        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.painter.clear(self.pixel_size, CLEAR_COLOR);
        self.painter.paint_and_update_textures(self.pixel_size, output.pixels_per_point, &primitives, &mut output.textures_delta);

        output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map_or(f64::INFINITY, |viewport| viewport.repaint_delay.as_secs_f64())
    }

    /// The CSS `cursor` value egui asked for on the last frame.
    #[must_use]
    pub fn cursor(&self) -> String {
        match self.cursor {
            egui::CursorIcon::Grab => "grab",
            egui::CursorIcon::Grabbing => "grabbing",
            egui::CursorIcon::Crosshair => "crosshair",
            egui::CursorIcon::ZoomIn => "zoom-in",
            _ => "default",
        }
        .to_owned()
    }

    #[must_use]
    pub fn zoom(&self) -> f32 {
        self.app.zoom()
    }

    /// Releases GL resources. Call before `free()`.
    pub fn destroy(&mut self) {
        self.painter.destroy();
    }

    fn pixels_per_point(&self) -> f32 {
        if self.css_size.x > 0.0 {
            #[allow(clippy::cast_precision_loss, reason = "canvas widths are far below f32's exact-integer range")]
            let pixel_width = self.pixel_size[0] as f32;
            pixel_width / self.css_size.x
        } else {
            1.0
        }
    }
}

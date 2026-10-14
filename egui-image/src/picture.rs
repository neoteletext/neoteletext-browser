use egui::{
    Align2, Color32, ColorImage, CornerRadius, FontId, Painter, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2,
    pos2, vec2,
};

// WebGL 2 guarantees textures up to 2048 px a side (OpenGL ES 3.0 spec
// §6.2, table 6.28, MAX_TEXTURE_SIZE minimum). Larger pictures could fail to
// upload on some GPUs, so they are refused up front.
const MAX_PICTURE_SIDE: u32 = 2048;
const BYTES_PER_PIXEL: usize = 4;
// Titles come from untrusted files later; anything longer is cut.
const MAX_TITLE_CHARS: usize = 120;

const MAX_ZOOM: f32 = 16.0;
// Zoom values this close to 1 read as "not zoomed".
const ZOOM_EPSILON: f32 = 0.01;
// Below this width the overlay chips would cover most of the picture.
const COMPACT_WIDTH: f32 = 260.0;

const CHIP_PADDING: Vec2 = vec2(7.0, 4.0);
const CHIP_INSET: f32 = 8.0;
const CHIP_RADIUS: u8 = 5;
const SWATCH_SIZE: f32 = 10.0;
const SWATCH_GAP: f32 = 6.0;
const TITLE_SIZE: f32 = 13.0;
const READOUT_SIZE: f32 = 11.5;
const CROSSHAIR_WIDTH: f32 = 1.0;

// Overlay colours sit on the picture itself, not on the page, so they are
// fixed rather than themed. Premultiplied: each channel is at most alpha.
const CHIP_BACKGROUND: Color32 = Color32::from_rgba_premultiplied(9, 11, 16, 196);
const CHIP_TEXT: Color32 = Color32::from_rgb(240, 243, 248);
const CHIP_MUTED: Color32 = Color32::from_rgb(158, 167, 184);
const CROSSHAIR: Color32 = Color32::from_rgba_premultiplied(120, 120, 120, 120);

pub struct Picture {
    title: String,
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Picture {
    pub fn new(title: &str, width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, String> {
        if width == 0 || height == 0 || width > MAX_PICTURE_SIDE || height > MAX_PICTURE_SIDE {
            return Err(format!("Picture is {width}×{height}; each side must be 1–{MAX_PICTURE_SIDE} pixels"));
        }
        let width = width as usize;
        let height = height as usize;
        let expected = width * height * BYTES_PER_PIXEL;
        if rgba.len() != expected {
            return Err(format!("Picture data is {} bytes; {width}×{height} RGBA needs {expected}", rgba.len()));
        }
        let title = title.chars().take(MAX_TITLE_CHARS).collect();
        Ok(Self { title, width, height, rgba })
    }

    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let index = (y * self.width + x) * BYTES_PER_PIXEL;
        [self.rgba[index], self.rgba[index + 1], self.rgba[index + 2]]
    }
}

pub struct PictureApp {
    picture: Picture,
    texture: Option<TextureHandle>,
    zoom: f32,
    // Centre of the visible region, in texture UV space (0–1 on both axes).
    center: Pos2,
}

impl PictureApp {
    pub fn new(picture: Picture) -> Self {
        Self { picture, texture: None, zoom: 1.0, center: pos2(0.5, 0.5) }
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let picture = &self.picture;
        let texture_id = self
            .texture
            .get_or_insert_with(|| {
                let image = ColorImage::from_rgba_unmultiplied([picture.width, picture.height], &picture.rgba);
                ui.ctx().load_texture("picture", image, TextureOptions::LINEAR)
            })
            .id();

        let response = ui.allocate_rect(rect, Sense::click_and_drag());

        // Browsers report trackpad pinches as ctrl+wheel; egui turns those
        // into zoom_delta.
        let zoom_delta = ui.input(egui::InputState::zoom_delta);
        if let Some(pointer) = response.hover_pos().filter(|_| (zoom_delta - 1.0).abs() > f32::EPSILON) {
            self.zoom_about(rect, pointer, zoom_delta);
        }
        if response.dragged() {
            let uv_per_point = self.visible_uv().size() / rect.size();
            self.center -= response.drag_delta() * uv_per_point;
            self.clamp_center();
        }
        if response.double_clicked() {
            self.zoom = 1.0;
            self.center = pos2(0.5, 0.5);
        }

        let painter = ui.painter_at(rect);
        painter.image(texture_id, rect, self.visible_uv(), Color32::WHITE);

        let hover = response.hover_pos();
        if let Some(pointer) = hover {
            let stroke = Stroke::new(CROSSHAIR_WIDTH, CROSSHAIR);
            painter.line_segment([pos2(rect.left(), pointer.y), pos2(rect.right(), pointer.y)], stroke);
            painter.line_segment([pos2(pointer.x, rect.top()), pos2(pointer.x, rect.bottom())], stroke);
        }

        if rect.width() >= COMPACT_WIDTH {
            let inset = Vec2::splat(CHIP_INSET);
            chip(&painter, rect.left_top() + inset, Align2::LEFT_TOP, &self.picture.title, FontId::proportional(TITLE_SIZE), None);
            if self.zoom > 1.0 + ZOOM_EPSILON {
                let label = format!("{:.1}×", self.zoom);
                chip(&painter, rect.right_top() + vec2(-CHIP_INSET, CHIP_INSET), Align2::RIGHT_TOP, &label, FontId::monospace(READOUT_SIZE), None);
            }
            if let Some(pointer) = hover {
                let (x, y) = self.pixel_at(rect, pointer);
                let [red, green, blue] = self.picture.pixel(x, y);
                let label = format!("x {x:<4} y {y:<4} #{red:02X}{green:02X}{blue:02X}");
                let swatch = Color32::from_rgb(red, green, blue);
                chip(&painter, rect.left_bottom() + vec2(CHIP_INSET, -CHIP_INSET), Align2::LEFT_BOTTOM, &label, FontId::monospace(READOUT_SIZE), Some(swatch));
            }
        }

        let cursor = if response.dragged() {
            egui::CursorIcon::Grabbing
        } else if self.zoom > 1.0 + ZOOM_EPSILON {
            egui::CursorIcon::Grab
        } else {
            egui::CursorIcon::Crosshair
        };
        response.on_hover_cursor(cursor);
    }

    fn visible_uv(&self) -> Rect {
        Rect::from_center_size(self.center, Vec2::splat(1.0 / self.zoom))
    }

    fn clamp_center(&mut self) {
        let half = 0.5 / self.zoom;
        self.center = pos2(self.center.x.clamp(half, 1.0 - half), self.center.y.clamp(half, 1.0 - half));
    }

    fn uv_at(&self, rect: Rect, point: Pos2) -> Pos2 {
        let uv = self.visible_uv();
        uv.min + (point - rect.min) / rect.size() * uv.size()
    }

    // Keeps the texel under the pointer fixed while the zoom changes.
    fn zoom_about(&mut self, rect: Rect, pointer: Pos2, factor: f32) {
        let anchor = self.uv_at(rect, pointer);
        self.zoom = (self.zoom * factor).clamp(1.0, MAX_ZOOM);
        let offset = (pointer - rect.center()) / rect.size() / self.zoom;
        self.center = anchor - offset;
        self.clamp_center();
    }

    fn pixel_at(&self, rect: Rect, point: Pos2) -> (usize, usize) {
        let uv = self.uv_at(rect, point);
        #[allow(clippy::cast_precision_loss, reason = "picture sides are at most 2048, exact in f32")]
        let (width, height) = (self.picture.width as f32, self.picture.height as f32);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..side-1 first, so the value is a valid non-negative index"
        )]
        let to_index = |fraction: f32, side: f32| (fraction * side).clamp(0.0, side - 1.0) as usize;
        (to_index(uv.x, width), to_index(uv.y, height))
    }
}

fn chip(painter: &Painter, anchor: Pos2, align: Align2, text: &str, font: FontId, swatch: Option<Color32>) {
    let color = if swatch.is_some() { CHIP_MUTED } else { CHIP_TEXT };
    let galley = painter.layout_no_wrap(text.to_owned(), font, color);
    let swatch_width = if swatch.is_some() { SWATCH_SIZE + SWATCH_GAP } else { 0.0 };
    let size = galley.size() + vec2(swatch_width, 0.0) + CHIP_PADDING * 2.0;
    let chip_rect = align.anchor_size(anchor, size);
    painter.rect_filled(chip_rect, CornerRadius::same(CHIP_RADIUS), CHIP_BACKGROUND);
    if let Some(swatch) = swatch {
        let swatch_rect = Rect::from_center_size(
            pos2(chip_rect.left() + CHIP_PADDING.x + SWATCH_SIZE / 2.0, chip_rect.center().y),
            Vec2::splat(SWATCH_SIZE),
        );
        painter.rect_filled(swatch_rect, CornerRadius::same(2), swatch);
    }
    painter.galley(chip_rect.min + CHIP_PADDING + vec2(swatch_width, 0.0), galley, color);
}

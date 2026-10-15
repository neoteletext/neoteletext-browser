//! The wall of pages: a centred, wrapping grid of pages with captions, or,
//! in the list layout, one page a row with its details beside it. Pages are
//! drawn as textured meshes, so the prototype's CSS transforms can be applied
//! to their corners: the fan turns each page in its own plane, and the tilt
//! leans the whole wall back with the perspective projection CSS uses.

use eframe::egui::{
    Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui, epaint::TextShape, pos2, vec2,
};

use crate::pages::Page;
use crate::size::{Layout, grid_width};
use crate::theme::{FRAME, INK, LINE, MUTED};

/// Native teletext raster, 480 by 500 dots.
const PAGE_ASPECT: f32 = 500.0 / 480.0;
const COLUMN_GAP: f32 = 14.0;
const ROW_GAP: f32 = 28.0;
const CAPTION_GAP: f32 = 8.0;
const CAPTION_HEIGHT: f32 = 18.0;
const TITLE_SIZE: f32 = 13.5;
const TEMPLATE_SIZE: f32 = 11.0;
/// The list: a column no wider than 54 rem, pages 176 pixels wide.
const LIST_MAX_WIDTH: f32 = 864.0;
const LIST_PAGE_WIDTH: f32 = 176.0;
const LIST_ROW_PADDING: f32 = 12.0;
const LIST_GAP: f32 = 20.0;
const LIST_TITLE_SIZE: f32 = 17.0;
const DETAIL_SIZE: f32 = 12.5;
/// Each page is cut into this many strips each way, so the perspective of
/// the tilt bends the texture smoothly.
const MESH_DIVISIONS: usize = 8;

/// The tilt, from the prototype's `.wall--tilt`: rotateX(22deg)
/// rotateZ(-4deg) scale(0.94), seen with `perspective: 1800px`.
const TILT_X_DEGREES: f32 = 22.0;
const TILT_Z_DEGREES: f32 = -4.0;
const TILT_SCALE: f32 = 0.94;
const PERSPECTIVE: f32 = 1800.0;
/// The fan, from `.wall--fan .egui-card:nth-child(5n + k)`: an angle in
/// degrees and a lift in pixels, repeating every five pages.
const FAN: [(f32, f32); 5] = [(-2.5, -3.0), (-3.5, 0.0), (2.0, 8.0), (-1.0, -6.0), (4.0, 4.0)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    Flat,
    Tilt,
    Fan,
}

impl Transform {
    pub const ALL: [Self; 3] = [Self::Flat, Self::Tilt, Self::Fan];

    pub fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Tilt => "Tilt",
            Self::Fan => "Fan",
        }
    }
}

/// How far each transform has been applied, from 0 to 1, as they animate.
#[derive(Clone, Copy)]
pub struct Pose {
    pub tilt: f32,
    pub fan: f32,
}

/// Places points: the fan around a page's own centre, then the tilt around
/// the wall's top centre, then perspective from the wall's centre.
struct Placement {
    pose: Pose,
    wall_origin: Pos2,
    perspective_origin: Pos2,
}

impl Placement {
    fn fan(&self, index: usize, centre: Pos2, point: Pos2) -> Pos2 {
        // `nth-child` counts from 1.
        let (degrees, lift) = FAN[(index + 1) % FAN.len()];
        let angle = (degrees * self.pose.fan).to_radians();
        let offset = point - centre;
        let (sine, cosine) = angle.sin_cos();
        centre + vec2(offset.x * cosine - offset.y * sine, offset.x * sine + offset.y * cosine + lift * self.pose.fan)
    }

    fn tilt(&self, point: Pos2) -> Pos2 {
        let scale = 1.0 + (TILT_SCALE - 1.0) * self.pose.tilt;
        let (sine_z, cosine_z) = (TILT_Z_DEGREES * self.pose.tilt).to_radians().sin_cos();
        let (sine_x, cosine_x) = (TILT_X_DEGREES * self.pose.tilt).to_radians().sin_cos();
        let local = (point - self.wall_origin) * scale;
        let turned = vec2(local.x * cosine_z - local.y * sine_z, local.x * sine_z + local.y * cosine_z);
        // rotateX: y shortens and the point comes towards the viewer (CSS z).
        let depth = turned.y * sine_x;
        let flat = self.wall_origin + vec2(turned.x, turned.y * cosine_x);
        let magnification = PERSPECTIVE / (PERSPECTIVE - depth);
        self.perspective_origin + (flat - self.perspective_origin) * magnification
    }

    fn place(&self, index: usize, centre: Pos2, point: Pos2) -> Pos2 {
        self.tilt(self.fan(index, centre, point))
    }
}

fn page_mesh(page: &Page, rect: Rect, place: impl Fn(Pos2) -> Pos2) -> Mesh {
    let mut mesh = Mesh::with_texture(page.texture.id());
    #[allow(clippy::cast_precision_loss, reason = "a handful of divisions")]
    let step = 1.0 / MESH_DIVISIONS as f32;
    for row in 0..=MESH_DIVISIONS {
        for column in 0..=MESH_DIVISIONS {
            #[allow(clippy::cast_precision_loss, reason = "a handful of divisions")]
            let uv = pos2(column as f32 * step, row as f32 * step);
            let point = rect.min + rect.size() * uv.to_vec2();
            mesh.vertices.push(eframe::egui::epaint::Vertex { pos: place(point), uv, color: Color32::WHITE });
        }
    }
    let stride = u32::try_from(MESH_DIVISIONS + 1).unwrap_or(0);
    for row in 0..stride - 1 {
        for column in 0..stride - 1 {
            let top_left = row * stride + column;
            mesh.add_triangle(top_left, top_left + 1, top_left + stride + 1);
            mesh.add_triangle(top_left, top_left + stride + 1, top_left + stride);
        }
    }
    mesh
}

fn caption(ui: &Ui, text: String, size: f32, colour: Color32, position: Pos2, angle: f32) -> Shape {
    let galley = ui.painter().layout_no_wrap(text, FontId::proportional(size), colour);
    Shape::Text(TextShape::new(position, galley, colour).with_angle(angle))
}

pub fn show(ui: &mut Ui, pages: &[Page], width: f32, layout: Layout, pose: Pose) {
    match layout {
        Layout::Grid => grid(ui, pages, grid_width(width), pose),
        Layout::List => list(ui, pages),
    }
}

fn grid(ui: &mut Ui, pages: &[Page], card_width: f32, pose: Pose) {
    let available = ui.available_width();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small, positive column count")]
    let columns = (((available + COLUMN_GAP) / (card_width + COLUMN_GAP)).floor() as usize).max(1);
    let rows = pages.len().div_ceil(columns);
    let page_height = card_width * PAGE_ASPECT;
    let card_height = page_height + CAPTION_GAP + CAPTION_HEIGHT;
    #[allow(clippy::cast_precision_loss, reason = "small counts")]
    let total_height = rows as f32 * card_height + (rows.saturating_sub(1)) as f32 * ROW_GAP;
    let (wall, _) = ui.allocate_exact_size(vec2(available, total_height), Sense::hover());
    let placement = Placement { pose, wall_origin: wall.center_top(), perspective_origin: wall.center() };
    let painter = ui.painter();

    for (index, page) in pages.iter().enumerate() {
        let (row, column) = (index / columns, index % columns);
        let in_row = columns.min(pages.len() - row * columns);
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let row_width = in_row as f32 * card_width + (in_row - 1) as f32 * COLUMN_GAP;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let origin = pos2(
            wall.center().x - row_width / 2.0 + column as f32 * (card_width + COLUMN_GAP),
            wall.top() + row as f32 * (card_height + ROW_GAP),
        );
        let page_rect = Rect::from_min_size(origin, vec2(card_width, page_height));
        let centre = Rect::from_min_size(origin, vec2(card_width, card_height)).center();
        let place = |point: Pos2| placement.place(index, centre, point);

        // The frame behind the page, then the page itself.
        let corners = [page_rect.left_top(), page_rect.right_top(), page_rect.right_bottom(), page_rect.left_bottom()];
        painter.add(Shape::convex_polygon(corners.iter().map(|&corner| place(corner)).collect(), FRAME, Stroke::NONE));
        painter.add(Shape::mesh(page_mesh(page, page_rect, place)));

        // Captions follow the page: turned with the fan, carried by the tilt.
        let angle = (FAN[(index + 1) % FAN.len()].0 * pose.fan).to_radians();
        let caption_top = page_rect.left_bottom() + vec2(0.0, CAPTION_GAP);
        painter.add(caption(ui, format!("P{:03X}", page.number), TITLE_SIZE, INK, place(caption_top), angle));
        let channel = ui.painter().layout_no_wrap(format!("channel {}", page.channel), FontId::monospace(TEMPLATE_SIZE), MUTED);
        let channel_top = page_rect.right_bottom() + vec2(-channel.size().x, CAPTION_GAP + 2.0);
        painter.add(Shape::Text(TextShape::new(place(channel_top), channel, MUTED).with_angle(angle)));
    }
}

fn list(ui: &mut Ui, pages: &[Page]) {
    let width = ui.available_width().min(LIST_MAX_WIDTH);
    let page_height = LIST_PAGE_WIDTH * PAGE_ASPECT;
    let row_height = page_height + LIST_ROW_PADDING * 2.0;
    #[allow(clippy::cast_precision_loss, reason = "small counts")]
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_height * pages.len() as f32), Sense::hover());
    let left = area.center().x - width / 2.0;
    let painter = ui.painter();
    for (index, page) in pages.iter().enumerate() {
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let top = area.top() + index as f32 * row_height;
        if index == 0 {
            painter.hline(left..=left + width, top, Stroke::new(1.0, LINE));
        }
        painter.hline(left..=left + width, top + row_height, Stroke::new(1.0, LINE));
        let page_rect = Rect::from_min_size(pos2(left, top + LIST_ROW_PADDING), vec2(LIST_PAGE_WIDTH, page_height));
        painter.rect_filled(page_rect, 0.0, FRAME);
        painter.add(Shape::mesh(page_mesh(page, page_rect, |point| point)));
        let text_left = page_rect.right() + LIST_GAP;
        let middle = page_rect.center().y;
        painter.text(pos2(text_left, middle - 10.0), Align2::LEFT_BOTTOM, format!("P{:03X}", page.number), FontId::proportional(LIST_TITLE_SIZE), INK);
        painter.text(pos2(left + width, middle - 10.0), Align2::RIGHT_BOTTOM, format!("channel {}", page.channel), FontId::monospace(TEMPLATE_SIZE), MUTED);
        let details = format!("PAGE {:03X}     PICTURE 480 × 500     RENDERER egui", page.number);
        painter.text(pos2(text_left, middle + 6.0), Align2::LEFT_TOP, details, FontId::monospace(DETAIL_SIZE), MUTED);
    }
}

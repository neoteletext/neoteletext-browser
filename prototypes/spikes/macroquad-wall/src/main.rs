//! The React prototype's tab in macroquad: the header with the thumbnail
//! width slider, its readout and the transform picker, and the wall of
//! teletext pages. Pages are drawn as subdivided textured meshes, so the
//! flat, fan and tilt transforms (the tilt with the perspective CSS uses)
//! are one drawing path, animated with the prototype's CSS timing curves.
//! The event loop blocks between events instead of redrawing like a game.

mod pages;
mod size;
mod tween;

use std::time::{Duration, Instant};

use macroquad::miniquad::conf::Platform;
use macroquad::models::{Mesh, Vertex, draw_mesh};
use macroquad::prelude::*;

use pages::{PAGE_COUNT, Page, pages};
use size::{
    DEFAULT_WIDTH, LIST_STOP, Layout, MAX_WIDTH, SWITCH_DWELL_SECONDS, clamp_width, grid_width, layout_for_width,
    max_width_for_window, next_layout,
};
use tween::{EASE_OUT, TRANSFORM_EASE, Tween};

const WINDOW_TITLE: &str = "T42 Browser: macroquad page wall";
const WINDOW_WIDTH: i32 = 1280;
const WINDOW_HEIGHT: i32 = 820;

// ---- The prototype's dark theme (src/shared/tokens.css) ----
const RAISED: Color = Color::from_rgba(0x1b, 0x1e, 0x25, 0xff);
const FIELD: Color = Color::from_rgba(0x0f, 0x11, 0x15, 0xff);
const WELL: Color = Color::from_rgba(0x0c, 0x0e, 0x12, 0xff);
const FRAME: Color = Color::from_rgba(0x11, 0x13, 0x18, 0xff);
const INK: Color = Color::from_rgba(0xe6, 0xe9, 0xef, 0xff);
const MUTED: Color = Color::from_rgba(0x93, 0x99, 0xa6, 0xff);
const LINE: Color = Color::from_rgba(0x2b, 0x2f, 0x38, 0xff);
const LINE_SOFT: Color = Color::from_rgba(0x20, 0x24, 0x2b, 0xff);
const ACCENT: Color = Color::from_rgba(0x8f, 0xa1, 0xff, 0xff);
const MOSAIC: [Color; 4] = [
    Color::from_rgba(0xff, 0x4d, 0x4d, 0xff),
    Color::from_rgba(0xff, 0xd8, 0x4d, 0xff),
    Color::from_rgba(0x4d, 0xd2, 0xff, 0xff),
    Color::from_rgba(0x4d, 0xff, 0x88, 0xff),
];

// ---- Header geometry, logical pixels ----
const HEADER_HEIGHT: f32 = 92.0;
const GUTTER: f32 = 24.0;
const LABEL_BASELINE: f32 = 24.0;
const CONTROL_TOP: f32 = 33.0;
const CONTROL_HEIGHT: f32 = 26.0;
const SCALE_BASELINE: f32 = 76.0;
const SLIDER_LEFT: f32 = GUTTER;
const SLIDER_WIDTH: f32 = 256.0;
const TRACK_HEIGHT: f32 = 6.0;
const THUMB_WIDTH: f32 = 10.0;
const THUMB_BORDER: f32 = 2.0;
const CONTROL_GAP: f32 = 22.0;
const READOUT_LEFT: f32 = SLIDER_LEFT + SLIDER_WIDTH + CONTROL_GAP;
const READOUT_WIDTH: f32 = 140.0;
const PICKER_LEFT: f32 = READOUT_LEFT + READOUT_WIDTH + CONTROL_GAP;
const PICKER_PADDING: f32 = 2.0;
const OPTION_PADDING: f32 = 12.0;
const OPTION_HEIGHT: f32 = 20.0;
const MOSAIC_SIZE: f32 = 36.0;
const IDENTITY_GAP: f32 = 12.0;
const LABEL_SIZE: u16 = 13;
const CAPTION_SIZE: u16 = 16;
const READOUT_SIZE: u16 = 28;
const TITLE_SIZE: u16 = 26;
/// One press of an arrow key moves the slider this far.
const KEY_STEP: f32 = 8.0;

// ---- Wall geometry ----
const WALL_TOP_PADDING: f32 = 28.0;
const WALL_BOTTOM_PADDING: f32 = 40.0;
const COLUMN_GAP: f32 = 14.0;
const ROW_GAP: f32 = 28.0;
const CAPTION_GAP: f32 = 8.0;
const CAPTION_HEIGHT: f32 = 13.0;
/// Native teletext raster, 480 by 500 dots.
const PAGE_ASPECT: f32 = 500.0 / 480.0;
const LIST_MAX_WIDTH: f32 = 864.0;
const LIST_PAGE_WIDTH: f32 = 176.0;
const LIST_ROW_PADDING: f32 = 12.0;
const LIST_GAP: f32 = 20.0;
const SCROLL_BAR_WIDTH: f32 = 4.0;
/// Logical pixels scrolled for one wheel step.
const WHEEL_STEP: f32 = 21.0;
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

/// The prototype's CSS transitions: width 160 ms ease-out, transforms 420 ms.
const WIDTH_TRANSITION: Duration = Duration::from_millis(160);
const TRANSFORM_TRANSITION: Duration = Duration::from_millis(420);
const CHANNEL_TITLE: &str = "Stand-in channel";

#[derive(Clone, Copy, PartialEq, Eq)]
enum WallTransform {
    Flat,
    Tilt,
    Fan,
}

impl WallTransform {
    const ALL: [Self; 3] = [Self::Flat, Self::Tilt, Self::Fan];

    fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Tilt => "Tilt",
            Self::Fan => "Fan",
        }
    }
}

/// How far each transform is applied, from 0 to 1, and where the wall is.
struct Pose {
    tilt: f32,
    fan: f32,
    wall_origin: Vec2,
    perspective_origin: Vec2,
}

impl Pose {
    fn fan_of(index: usize) -> (f32, f32) {
        // `nth-child` counts from 1.
        FAN[(index + 1) % FAN.len()]
    }

    fn place(&self, index: usize, centre: Vec2, point: Vec2) -> Vec2 {
        // The fan turns the page about its own centre.
        let (degrees, lift) = Self::fan_of(index);
        let (sine, cosine) = (degrees * self.fan).to_radians().sin_cos();
        let offset = point - centre;
        let point = centre + vec2(offset.x * cosine - offset.y * sine, offset.x * sine + offset.y * cosine + lift * self.fan);
        // The tilt leans the wall back about its top centre, in perspective.
        let scale = 1.0 + (TILT_SCALE - 1.0) * self.tilt;
        let (sine_z, cosine_z) = (TILT_Z_DEGREES * self.tilt).to_radians().sin_cos();
        let (sine_x, cosine_x) = (TILT_X_DEGREES * self.tilt).to_radians().sin_cos();
        let local = (point - self.wall_origin) * scale;
        let turned = vec2(local.x * cosine_z - local.y * sine_z, local.x * sine_z + local.y * cosine_z);
        let depth = turned.y * sine_x;
        let flat = self.wall_origin + vec2(turned.x, turned.y * cosine_x);
        self.perspective_origin + (flat - self.perspective_origin) * (PERSPECTIVE / (PERSPECTIVE - depth))
    }
}

fn draw_page(page: &Page, index: usize, rect: Rect, centre: Vec2, pose: &Pose) {
    let mut vertices = Vec::with_capacity((MESH_DIVISIONS + 1) * (MESH_DIVISIONS + 1));
    #[allow(clippy::cast_precision_loss, reason = "a handful of divisions")]
    let step = 1.0 / MESH_DIVISIONS as f32;
    for row in 0..=MESH_DIVISIONS {
        for column in 0..=MESH_DIVISIONS {
            #[allow(clippy::cast_precision_loss, reason = "a handful of divisions")]
            let (u, v) = (column as f32 * step, row as f32 * step);
            let point = pose.place(index, centre, vec2(rect.x + rect.w * u, rect.y + rect.h * v));
            vertices.push(Vertex::new(point.x, point.y, 0.0, u, v, WHITE));
        }
    }
    let stride = u16::try_from(MESH_DIVISIONS + 1).unwrap_or(0);
    let mut indices = Vec::new();
    for row in 0..stride - 1 {
        for column in 0..stride - 1 {
            let top_left = row * stride + column;
            indices.extend_from_slice(&[top_left, top_left + 1, top_left + stride + 1, top_left, top_left + stride + 1, top_left + stride]);
        }
    }
    draw_mesh(&Mesh { vertices, indices, texture: Some(page.texture.clone()) });
}

fn text_width(text: &str, size: u16) -> f32 {
    measure_text(text, None, size, 1.0).width
}

fn contains(rect: Rect, (x, y): (f32, f32)) -> bool {
    rect.contains(vec2(x, y))
}

fn option_rects() -> [Rect; 3] {
    let mut left = PICKER_LEFT + PICKER_PADDING;
    let top = CONTROL_TOP + (CONTROL_HEIGHT - OPTION_HEIGHT) / 2.0;
    WallTransform::ALL.map(|choice| {
        let width = text_width(choice.label(), LABEL_SIZE) + OPTION_PADDING * 2.0;
        let rect = Rect::new(left, top, width, OPTION_HEIGHT);
        left += width;
        rect
    })
}

struct Tab {
    pages: Vec<Page>,
    width: f32,
    max: f32,
    layout: Layout,
    pending: Option<(Layout, Instant)>,
    transform: WallTransform,
    shown_width: Tween,
    tilt: Tween,
    fan: Tween,
    scroll: f32,
    dragging: bool,
}

impl Tab {
    fn set_width(&mut self, width: f32, now: Instant) {
        self.width = clamp_width(width, self.max);
        self.shown_width.set(self.width, now);
    }

    fn width_at(&self, x: f32) -> f32 {
        let fraction = ((x - SLIDER_LEFT - THUMB_WIDTH / 2.0) / (SLIDER_WIDTH - THUMB_WIDTH)).clamp(0.0, 1.0);
        LIST_STOP + fraction * (self.max - LIST_STOP)
    }

    fn input(&mut self, now: Instant) {
        let pointer = mouse_position();
        if is_mouse_button_pressed(MouseButton::Left) {
            if contains(Rect::new(SLIDER_LEFT, CONTROL_TOP, SLIDER_WIDTH, CONTROL_HEIGHT), pointer) {
                self.dragging = true;
            }
            for (choice, rect) in WallTransform::ALL.into_iter().zip(option_rects()) {
                if contains(rect, pointer) {
                    self.transform = choice;
                    self.tilt.set(f32::from(u8::from(choice == WallTransform::Tilt)), now);
                    self.fan.set(f32::from(u8::from(choice == WallTransform::Fan)), now);
                }
            }
        }
        if !is_mouse_button_down(MouseButton::Left) {
            self.dragging = false;
        }
        if self.dragging {
            self.set_width(self.width_at(pointer.0), now);
        }
        if is_key_pressed(KeyCode::Right) {
            self.set_width(self.width + KEY_STEP, now);
        }
        if is_key_pressed(KeyCode::Left) {
            self.set_width(self.width - KEY_STEP, now);
        }
        let (_, wheel) = mouse_wheel();
        self.scroll = (self.scroll - wheel * WHEEL_STEP).max(0.0);
    }

    /// The layout trails the thumb through the hysteresis band and a short dwell.
    fn settle(&mut self, now: Instant) {
        let target = next_layout(self.layout, self.width);
        if target == self.layout {
            self.pending = None;
            return;
        }
        match self.pending {
            Some((waiting_for, since)) if waiting_for == target => {
                if now.saturating_duration_since(since) >= Duration::from_secs_f32(SWITCH_DWELL_SECONDS) {
                    self.layout = target;
                    self.pending = None;
                }
            }
            _ => self.pending = Some((target, now)),
        }
    }

    fn busy(&self, now: Instant) -> bool {
        self.pending.is_some() || self.dragging || self.shown_width.moving(now) || self.tilt.moving(now) || self.fan.moving(now)
    }

    fn draw_wall(&mut self, width: f32, height: f32, now: Instant) {
        let available = width - GUTTER * 2.0;
        let content = match self.layout {
            Layout::Grid => self.draw_grid(width, height, available, now),
            Layout::List => self.draw_list(width, height, available),
        };
        let view = height - HEADER_HEIGHT;
        let total = content + WALL_TOP_PADDING + WALL_BOTTOM_PADDING;
        let max_scroll = (total - view).max(0.0);
        self.scroll = self.scroll.min(max_scroll);
        if max_scroll > 0.0 {
            let bar = view * view / total;
            draw_rectangle(width - SCROLL_BAR_WIDTH * 2.0, HEADER_HEIGHT + (view - bar) * self.scroll / max_scroll, SCROLL_BAR_WIDTH, bar, LINE);
        }
    }

    fn draw_grid(&self, width: f32, height: f32, available: f32, now: Instant) -> f32 {
        let card_width = grid_width(self.shown_width.value(now));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small, positive column count")]
        let columns = (((available + COLUMN_GAP) / (card_width + COLUMN_GAP)).floor() as usize).max(1);
        let rows = self.pages.len().div_ceil(columns);
        let page_height = card_width * PAGE_ASPECT;
        let card_height = page_height + CAPTION_GAP + CAPTION_HEIGHT;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let content = rows as f32 * card_height + rows.saturating_sub(1) as f32 * ROW_GAP;
        let top = HEADER_HEIGHT + WALL_TOP_PADDING - self.scroll;
        let pose = Pose {
            tilt: self.tilt.value(now),
            fan: self.fan.value(now),
            wall_origin: vec2(width / 2.0, top),
            perspective_origin: vec2(width / 2.0, top + content / 2.0),
        };
        let transformed = pose.tilt > f32::EPSILON || pose.fan > f32::EPSILON;
        for (index, page) in self.pages.iter().enumerate() {
            let (row, column) = (index / columns, index % columns);
            let in_row = columns.min(self.pages.len() - row * columns);
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let row_width = in_row as f32 * card_width + (in_row - 1) as f32 * COLUMN_GAP;
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let (left, card_top) = (width / 2.0 - row_width / 2.0 + column as f32 * (card_width + COLUMN_GAP), top + row as f32 * (card_height + ROW_GAP));
            if !transformed && (card_top > height || card_top + card_height < HEADER_HEIGHT) {
                continue;
            }
            let rect = Rect::new(left, card_top, card_width, page_height);
            let centre = vec2(left + card_width / 2.0, card_top + card_height / 2.0);
            if !transformed {
                draw_rectangle(rect.x, rect.y, rect.w, rect.h, FRAME);
            }
            draw_page(page, index, rect, centre, &pose);
            // Captions are carried along by the transforms but not turned.
            let caption = pose.place(index, centre, vec2(left, card_top + page_height + CAPTION_GAP + CAPTION_HEIGHT));
            draw_text(format!("P{:03X}", page.number), caption.x, caption.y, f32::from(CAPTION_SIZE), INK);
            let channel = format!("channel {}", page.channel);
            let channel_at = pose.place(index, centre, vec2(left + card_width - text_width(&channel, LABEL_SIZE), card_top + page_height + CAPTION_GAP + CAPTION_HEIGHT));
            draw_text(&channel, channel_at.x, channel_at.y, f32::from(LABEL_SIZE), MUTED);
        }
        content
    }

    fn draw_list(&self, width: f32, height: f32, available: f32) -> f32 {
        let list_width = available.min(LIST_MAX_WIDTH);
        let left = width / 2.0 - list_width / 2.0;
        let page_height = LIST_PAGE_WIDTH * PAGE_ASPECT;
        let row_height = page_height + LIST_ROW_PADDING * 2.0;
        let top = HEADER_HEIGHT + WALL_TOP_PADDING - self.scroll;
        let flat = Pose { tilt: 0.0, fan: 0.0, wall_origin: Vec2::ZERO, perspective_origin: Vec2::ZERO };
        for (index, page) in self.pages.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let row_top = top + index as f32 * row_height;
            if row_top > height || row_top + row_height < HEADER_HEIGHT {
                continue;
            }
            if index == 0 {
                draw_rectangle(left, row_top, list_width, 1.0, LINE);
            }
            draw_rectangle(left, row_top + row_height, list_width, 1.0, LINE);
            let rect = Rect::new(left, row_top + LIST_ROW_PADDING, LIST_PAGE_WIDTH, page_height);
            draw_page(page, index, rect, Vec2::ZERO, &flat);
            let text_left = rect.right() + LIST_GAP;
            let middle = rect.y + page_height / 2.0;
            draw_text(format!("P{:03X}", page.number), text_left, middle - 8.0, f32::from(READOUT_SIZE) * 0.75, INK);
            let channel = format!("channel {}", page.channel);
            draw_text(&channel, left + list_width - text_width(&channel, LABEL_SIZE), middle - 8.0, f32::from(LABEL_SIZE), MUTED);
            draw_text(format!("PAGE {:03X}   PICTURE 480 x 500   RENDERER macroquad", page.number), text_left, middle + 16.0, f32::from(LABEL_SIZE), MUTED);
        }
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let height = row_height * self.pages.len() as f32;
        height
    }

    fn draw_header(&self, width: f32) {
        draw_rectangle(0.0, 0.0, width, HEADER_HEIGHT, RAISED);
        draw_rectangle(0.0, HEADER_HEIGHT - 1.0, width, 1.0, LINE_SOFT);

        // The slider: label, track filled up to the thumb, scale.
        draw_text("THUMBNAIL WIDTH", SLIDER_LEFT, LABEL_BASELINE, f32::from(LABEL_SIZE), MUTED);
        let fraction = (self.width - LIST_STOP) / (self.max - LIST_STOP);
        let track_top = CONTROL_TOP + (CONTROL_HEIGHT - TRACK_HEIGHT) / 2.0;
        draw_rectangle(SLIDER_LEFT, track_top, SLIDER_WIDTH, TRACK_HEIGHT, LINE);
        let thumb_left = SLIDER_LEFT + fraction * (SLIDER_WIDTH - THUMB_WIDTH);
        draw_rectangle(SLIDER_LEFT, track_top, thumb_left - SLIDER_LEFT + THUMB_WIDTH / 2.0, TRACK_HEIGHT, ACCENT);
        draw_rectangle(thumb_left, CONTROL_TOP, THUMB_WIDTH, CONTROL_HEIGHT, ACCENT);
        draw_rectangle(thumb_left + THUMB_BORDER, CONTROL_TOP + THUMB_BORDER, THUMB_WIDTH - THUMB_BORDER * 2.0, CONTROL_HEIGHT - THUMB_BORDER * 2.0, FIELD);
        draw_text("List", SLIDER_LEFT, SCALE_BASELINE, f32::from(LABEL_SIZE), if self.layout == Layout::List { ACCENT } else { MUTED });
        let max = format!("{}", self.max.round());
        draw_text(&max, SLIDER_LEFT + SLIDER_WIDTH - text_width(&max, LABEL_SIZE), SCALE_BASELINE, f32::from(LABEL_SIZE), MUTED);

        // The readout.
        let baseline = CONTROL_TOP + CONTROL_HEIGHT - 4.0;
        if self.layout == Layout::List {
            draw_text("List", READOUT_LEFT, baseline, f32::from(READOUT_SIZE), INK);
        } else {
            let value = format!("{}", grid_width(self.width).round());
            draw_text(&value, READOUT_LEFT, baseline, f32::from(READOUT_SIZE), INK);
            draw_text("px", READOUT_LEFT + text_width(&value, READOUT_SIZE) + 4.0, baseline, f32::from(LABEL_SIZE), MUTED);
        }

        // The transform picker.
        draw_text("CSS TRANSFORM", PICKER_LEFT, LABEL_BASELINE, f32::from(LABEL_SIZE), MUTED);
        let options = option_rects();
        let box_width = options.iter().map(|rect| rect.w).sum::<f32>() + PICKER_PADDING * 2.0;
        draw_rectangle(PICKER_LEFT, options[0].y - PICKER_PADDING, box_width, OPTION_HEIGHT + PICKER_PADDING * 2.0, LINE_SOFT);
        for (choice, rect) in WallTransform::ALL.into_iter().zip(options) {
            let selected = choice == self.transform;
            if selected {
                draw_rectangle(rect.x, rect.y, rect.w, rect.h, FIELD);
            }
            draw_text(choice.label(), rect.x + OPTION_PADDING, rect.y + 14.0, f32::from(LABEL_SIZE), if selected { INK } else { MUTED });
        }

        // The channel's identity, at the right edge.
        let meta = format!("{PAGE_COUNT} teletext pages");
        let text_left = width - GUTTER - text_width(CHANNEL_TITLE, TITLE_SIZE).max(text_width(&meta, LABEL_SIZE));
        let mosaic_left = text_left - IDENTITY_GAP - MOSAIC_SIZE;
        let mosaic_top = CONTROL_TOP + CONTROL_HEIGHT / 2.0 - MOSAIC_SIZE / 2.0 - 6.0;
        let corners = [vec2(mosaic_left, mosaic_top), vec2(mosaic_left + MOSAIC_SIZE, mosaic_top), vec2(mosaic_left + MOSAIC_SIZE, mosaic_top + MOSAIC_SIZE), vec2(mosaic_left, mosaic_top + MOSAIC_SIZE)];
        let vertices = corners.iter().zip(MOSAIC).map(|(corner, colour)| Vertex::new(corner.x, corner.y, 0.0, 0.0, 0.0, colour)).collect();
        draw_mesh(&Mesh { vertices, indices: vec![0, 1, 2, 0, 2, 3], texture: None });
        draw_text(CHANNEL_TITLE, text_left, mosaic_top + 18.0, f32::from(TITLE_SIZE), INK);
        draw_text(&meta, text_left, mosaic_top + 36.0, f32::from(LABEL_SIZE), MUTED);
    }
}

fn window_conf() -> macroquad::conf::Conf {
    macroquad::conf::Conf {
        miniquad_conf: Conf {
            window_title: WINDOW_TITLE.into(),
            window_width: WINDOW_WIDTH,
            window_height: WINDOW_HEIGHT,
            high_dpi: true,
            // Sleep between events, as a desktop application should.
            platform: Platform { blocking_event_loop: true, ..Default::default() },
            ..Default::default()
        },
        // With the loop blocking, a frame runs only for the input listed
        // here; without it, clicks, movement, the wheel and keys wake nothing.
        update_on: Some(macroquad::conf::UpdateTrigger {
            key_down: true,
            mouse_down: true,
            mouse_up: true,
            mouse_motion: true,
            mouse_wheel: true,
            specific_key: None,
            touch: true,
        }),
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut tab = Tab {
        pages: pages(),
        width: DEFAULT_WIDTH,
        max: MAX_WIDTH,
        layout: layout_for_width(DEFAULT_WIDTH),
        pending: None,
        transform: WallTransform::Flat,
        shown_width: Tween::new(DEFAULT_WIDTH, WIDTH_TRANSITION, EASE_OUT),
        tilt: Tween::new(0.0, TRANSFORM_TRANSITION, TRANSFORM_EASE),
        fan: Tween::new(0.0, TRANSFORM_TRANSITION, TRANSFORM_EASE),
        scroll: 0.0,
        dragging: false,
    };
    loop {
        let now = Instant::now();
        let (width, height) = (screen_width(), screen_height());
        tab.max = max_width_for_window(width);
        tab.input(now);
        tab.settle(now);
        clear_background(WELL);
        tab.draw_wall(width, height, now);
        tab.draw_header(width);
        // Keep drawing only while something moves or waits.
        if tab.busy(now) {
            macroquad::miniquad::window::schedule_update();
        }
        next_frame().await;
    }
}

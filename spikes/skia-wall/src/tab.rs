//! The tab: its state, layout, input and drawing. Because this code lays
//! everything out itself, it knows where every control is, and hit-tests the
//! pointer against the same rectangles it draws.

use std::time::{Duration, Instant};

use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_7X13, FONT_9X15, FONT_10X20};
use tiny_skia::{GradientStop, LinearGradient, Pixmap, Point, Rect, SpreadMode, Transform};

use crate::draw::{self, ACCENT, FIELD, FRAME, INK, LINE, LINE_SOFT, MUTED, Pose, RAISED, WELL, colour};
use crate::pages::{PAGE_COUNT, Page, pages};
use crate::size::{
    DEFAULT_WIDTH, LIST_STOP, Layout, MAX_WIDTH, SWITCH_DWELL_SECONDS, clamp_width, grid_width, layout_for_width,
    max_width_for_window, next_layout,
};
use crate::text;
use crate::tween::{EASE_OUT, TRANSFORM_EASE, Tween};

// ---- Header geometry, logical pixels ----
const HEADER_HEIGHT: f32 = 92.0;
const GUTTER: f32 = 24.0;
const LABEL_TOP: f32 = 14.0;
const CONTROL_TOP: f32 = 33.0;
const CONTROL_HEIGHT: f32 = 26.0;
const SCALE_TOP: f32 = 65.0;
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
/// One press of an arrow key moves the slider this far.
const KEY_STEP: f32 = 8.0;

// ---- Wall geometry ----
const WALL_TOP_PADDING: f32 = 28.0;
const WALL_BOTTOM_PADDING: f32 = 40.0;
const COLUMN_GAP: f32 = 14.0;
const ROW_GAP: f32 = 28.0;
const CAPTION_GAP: f32 = 8.0;
const CAPTION_HEIGHT: f32 = 13.0;
const FRAME_RADIUS: f32 = 7.0;
/// Native teletext raster, 480 by 500 dots.
const PAGE_ASPECT: f32 = 500.0 / 480.0;
const LIST_MAX_WIDTH: f32 = 864.0;
const LIST_PAGE_WIDTH: f32 = 176.0;
const LIST_ROW_PADDING: f32 = 12.0;
const LIST_GAP: f32 = 20.0;
const SCROLL_BAR_WIDTH: f32 = 4.0;

/// The prototype's CSS transitions: width 160 ms ease-out, transforms 420 ms.
const WIDTH_TRANSITION: Duration = Duration::from_millis(160);
const TRANSFORM_TRANSITION: Duration = Duration::from_millis(420);
const CHANNEL_TITLE: &str = "Stand-in channel";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transform3 {
    Flat,
    Tilt,
    Fan,
}

impl Transform3 {
    const ALL: [Self; 3] = [Self::Flat, Self::Tilt, Self::Fan];

    fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Tilt => "Tilt",
            Self::Fan => "Fan",
        }
    }
}

pub struct Tab {
    pages: Vec<Page>,
    width: f32,
    max: f32,
    layout: Layout,
    pending: Option<(Layout, Instant)>,
    transform: Transform3,
    shown_width: Tween,
    tilt: Tween,
    fan: Tween,
    scroll: f32,
    pointer: (f32, f32),
    dragging: bool,
}

fn option_rects() -> [Rect; 3] {
    let mut left = PICKER_LEFT + PICKER_PADDING;
    let top = CONTROL_TOP + (CONTROL_HEIGHT - OPTION_HEIGHT) / 2.0;
    Transform3::ALL.map(|choice| {
        let width = text::width(choice.label(), &FONT_7X13) + OPTION_PADDING * 2.0;
        let rect = Rect::from_xywh(left, top, width, OPTION_HEIGHT).unwrap_or(Rect::from_xywh(0.0, 0.0, 1.0, 1.0).expect("unit rectangle"));
        left += width;
        rect
    })
}

fn contains(rect: Rect, (x, y): (f32, f32)) -> bool {
    x >= rect.left() && x < rect.right() && y >= rect.top() && y < rect.bottom()
}

impl Tab {
    pub fn new() -> Self {
        Self {
            pages: pages(),
            width: DEFAULT_WIDTH,
            max: MAX_WIDTH,
            layout: layout_for_width(DEFAULT_WIDTH),
            pending: None,
            transform: Transform3::Flat,
            shown_width: Tween::new(DEFAULT_WIDTH, WIDTH_TRANSITION, EASE_OUT),
            tilt: Tween::new(0.0, TRANSFORM_TRANSITION, TRANSFORM_EASE),
            fan: Tween::new(0.0, TRANSFORM_TRANSITION, TRANSFORM_EASE),
            scroll: 0.0,
            pointer: (0.0, 0.0),
            dragging: false,
        }
    }

    fn set_width(&mut self, width: f32, now: Instant) {
        self.width = clamp_width(width, self.max);
        self.shown_width.set(self.width, now);
    }

    fn width_at(&self, x: f32) -> f32 {
        let fraction = ((x - SLIDER_LEFT - THUMB_WIDTH / 2.0) / (SLIDER_WIDTH - THUMB_WIDTH)).clamp(0.0, 1.0);
        LIST_STOP + fraction * (self.max - LIST_STOP)
    }

    pub fn pointer_moved(&mut self, x: f32, y: f32, now: Instant) -> bool {
        self.pointer = (x, y);
        if self.dragging {
            self.set_width(self.width_at(x), now);
        }
        self.dragging
    }

    pub fn pointer_pressed(&mut self, now: Instant) -> bool {
        let slider = Rect::from_xywh(SLIDER_LEFT, CONTROL_TOP, SLIDER_WIDTH, CONTROL_HEIGHT);
        if slider.is_some_and(|slider| contains(slider, self.pointer)) {
            self.dragging = true;
            self.set_width(self.width_at(self.pointer.0), now);
            return true;
        }
        for (choice, rect) in Transform3::ALL.into_iter().zip(option_rects()) {
            if contains(rect, self.pointer) {
                self.transform = choice;
                self.tilt.set(f32::from(u8::from(choice == Transform3::Tilt)), now);
                self.fan.set(f32::from(u8::from(choice == Transform3::Fan)), now);
                return true;
            }
        }
        false
    }

    pub fn pointer_released(&mut self) -> bool {
        self.dragging = false;
        false
    }

    pub fn scroll_by(&mut self, distance: f32) -> bool {
        self.scroll = (self.scroll + distance).max(0.0);
        true
    }

    pub fn step(&mut self, direction: i8, now: Instant) -> bool {
        self.set_width(self.width + f32::from(direction) * KEY_STEP, now);
        true
    }

    /// The layout trails the thumb through the hysteresis band and a short
    /// dwell, so a fast sweep across the band does not switch it. Returns
    /// whether the layout changed.
    pub fn settle(&mut self, now: Instant) -> bool {
        let target = next_layout(self.layout, self.width);
        if target == self.layout {
            self.pending = None;
            return false;
        }
        match self.pending {
            Some((waiting_for, since)) if waiting_for == target => {
                if now.saturating_duration_since(since) >= Duration::from_secs_f32(SWITCH_DWELL_SECONDS) {
                    self.layout = target;
                    self.pending = None;
                    return true;
                }
                false
            }
            _ => {
                self.pending = Some((target, now));
                false
            }
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.map(|(_, since)| since + Duration::from_secs_f32(SWITCH_DWELL_SECONDS))
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.shown_width.moving(now) || self.tilt.moving(now) || self.fan.moving(now)
    }

    pub fn draw(&mut self, pixmap: &mut Pixmap, scale: f32, now: Instant) {
        #[allow(clippy::cast_precision_loss, reason = "window sizes fit easily in f32")]
        let (width, height) = (pixmap.width() as f32 / scale, pixmap.height() as f32 / scale);
        self.max = max_width_for_window(width);
        if self.width > self.max {
            self.set_width(self.width, now);
        }
        pixmap.fill(draw::float(WELL));
        self.draw_wall(pixmap, scale, width, height, now);
        self.draw_header(pixmap, scale, width);
    }

    fn draw_wall(&mut self, pixmap: &mut Pixmap, scale: f32, width: f32, height: f32, now: Instant) {
        let available = width - GUTTER * 2.0;
        let content_height = match self.layout {
            Layout::Grid => self.draw_grid(pixmap, scale, (width, height), available, now),
            Layout::List => self.draw_list(pixmap, scale, (width, height), available),
        };
        // Clamp the scroll to the content, then show where the view is.
        let view = height - HEADER_HEIGHT;
        let total = content_height + WALL_TOP_PADDING + WALL_BOTTOM_PADDING;
        let max_scroll = (total - view).max(0.0);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
        if max_scroll > 0.0 {
            let bar_height = view * view / total;
            let bar_top = HEADER_HEIGHT + (view - bar_height) * self.scroll / max_scroll;
            draw::fill(pixmap, scale, Rect::from_xywh(width - SCROLL_BAR_WIDTH * 2.0, bar_top, SCROLL_BAR_WIDTH, bar_height), SCROLL_BAR_WIDTH / 2.0, LINE);
        }
    }

    /// Draws the grid and returns its height.
    fn draw_grid(&self, pixmap: &mut Pixmap, scale: f32, (width, height): (f32, f32), available: f32, now: Instant) -> f32 {
        let card_width = grid_width(self.shown_width.value(now));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small, positive column count")]
        let columns = (((available + COLUMN_GAP) / (card_width + COLUMN_GAP)).floor() as usize).max(1);
        let rows = self.pages.len().div_ceil(columns);
        let page_height = card_width * PAGE_ASPECT;
        let card_height = page_height + CAPTION_GAP + CAPTION_HEIGHT;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let content_height = rows as f32 * card_height + rows.saturating_sub(1) as f32 * ROW_GAP;
        let top = HEADER_HEIGHT + WALL_TOP_PADDING - self.scroll;
        let pose = Pose {
            tilt: self.tilt.value(now),
            fan: self.fan.value(now),
            wall_origin: Point::from_xy(width / 2.0, top),
            perspective_origin: Point::from_xy(width / 2.0, top + content_height / 2.0),
        };
        let transformed = pose.tilt > f32::EPSILON || pose.fan > f32::EPSILON;
        for (index, page) in self.pages.iter().enumerate() {
            let (row, column) = (index / columns, index % columns);
            let in_row = columns.min(self.pages.len() - row * columns);
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let row_width = in_row as f32 * card_width + (in_row - 1) as f32 * COLUMN_GAP;
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let (left, card_top) = (width / 2.0 - row_width / 2.0 + column as f32 * (card_width + COLUMN_GAP), top + row as f32 * (card_height + ROW_GAP));
            // Untransformed cards entirely outside the view are skipped.
            if !transformed && (card_top > height || card_top + card_height < HEADER_HEIGHT) {
                continue;
            }
            let Some(rect) = Rect::from_xywh(left, card_top, card_width, page_height) else { continue };
            let centre = Point::from_xy(left + card_width / 2.0, card_top + card_height / 2.0);
            if !transformed {
                draw::fill(pixmap, scale, Some(rect), FRAME_RADIUS, FRAME);
            }
            draw::page(pixmap, scale, page, index, rect, centre, &pose);
            // Captions are carried along by the transforms but not turned.
            let caption = pose.place(index, centre, Point::from_xy(left, card_top + page_height + CAPTION_GAP));
            text::draw(pixmap, scale, caption.x, caption.y, &format!("P{:03X}", page.number), &FONT_7X13, INK);
            let channel = format!("channel {}", page.channel);
            let channel_left = left + card_width - text::width(&channel, &FONT_6X10);
            let channel_at = pose.place(index, centre, Point::from_xy(channel_left, card_top + page_height + CAPTION_GAP + 2.0));
            text::draw(pixmap, scale, channel_at.x, channel_at.y, &channel, &FONT_6X10, MUTED);
        }
        content_height
    }

    /// Draws the list and returns its height.
    fn draw_list(&self, pixmap: &mut Pixmap, scale: f32, (width, height): (f32, f32), available: f32) -> f32 {
        let list_width = available.min(LIST_MAX_WIDTH);
        let left = width / 2.0 - list_width / 2.0;
        let page_height = LIST_PAGE_WIDTH * PAGE_ASPECT;
        let row_height = page_height + LIST_ROW_PADDING * 2.0;
        let top = HEADER_HEIGHT + WALL_TOP_PADDING - self.scroll;
        let flat = Pose { tilt: 0.0, fan: 0.0, wall_origin: Point::zero(), perspective_origin: Point::zero() };
        for (index, page) in self.pages.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let row_top = top + index as f32 * row_height;
            if row_top > height || row_top + row_height < HEADER_HEIGHT {
                continue;
            }
            if index == 0 {
                draw::fill(pixmap, scale, Rect::from_xywh(left, row_top, list_width, 1.0), 0.0, LINE);
            }
            draw::fill(pixmap, scale, Rect::from_xywh(left, row_top + row_height, list_width, 1.0), 0.0, LINE);
            let Some(rect) = Rect::from_xywh(left, row_top + LIST_ROW_PADDING, LIST_PAGE_WIDTH, page_height) else { continue };
            draw::page(pixmap, scale, page, index, rect, Point::zero(), &flat);
            let text_left = rect.right() + LIST_GAP;
            let middle = rect.top() + page_height / 2.0;
            text::draw(pixmap, scale, text_left, middle - 22.0, &format!("P{:03X}", page.number), &FONT_9X15, INK);
            let channel = format!("channel {}", page.channel);
            text::draw(pixmap, scale, left + list_width - text::width(&channel, &FONT_6X10), middle - 18.0, &channel, &FONT_6X10, MUTED);
            let details = format!("PAGE {:03X}   PICTURE 480 x 500   RENDERER tiny-skia", page.number);
            text::draw(pixmap, scale, text_left, middle + 4.0, &details, &FONT_6X10, MUTED);
        }
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let height = row_height * self.pages.len() as f32;
        height
    }

    fn draw_header(&self, pixmap: &mut Pixmap, scale: f32, width: f32) {
        draw::fill(pixmap, scale, Rect::from_xywh(0.0, 0.0, width, HEADER_HEIGHT), 0.0, RAISED);
        draw::fill(pixmap, scale, Rect::from_xywh(0.0, HEADER_HEIGHT - 1.0, width, 1.0), 0.0, LINE_SOFT);

        // The slider: label, track filled up to the thumb, scale.
        text::draw(pixmap, scale, SLIDER_LEFT, LABEL_TOP, "THUMBNAIL WIDTH", &FONT_6X10, MUTED);
        let fraction = (self.width - LIST_STOP) / (self.max - LIST_STOP);
        let track_top = CONTROL_TOP + (CONTROL_HEIGHT - TRACK_HEIGHT) / 2.0;
        draw::fill(pixmap, scale, Rect::from_xywh(SLIDER_LEFT, track_top, SLIDER_WIDTH, TRACK_HEIGHT), TRACK_HEIGHT / 2.0, LINE);
        let thumb_left = SLIDER_LEFT + fraction * (SLIDER_WIDTH - THUMB_WIDTH);
        draw::fill(pixmap, scale, Rect::from_xywh(SLIDER_LEFT, track_top, thumb_left - SLIDER_LEFT + THUMB_WIDTH / 2.0, TRACK_HEIGHT), TRACK_HEIGHT / 2.0, ACCENT);
        draw::fill(pixmap, scale, Rect::from_xywh(thumb_left, CONTROL_TOP, THUMB_WIDTH, CONTROL_HEIGHT), 3.0, ACCENT);
        draw::fill(pixmap, scale, Rect::from_xywh(thumb_left + THUMB_BORDER, CONTROL_TOP + THUMB_BORDER, THUMB_WIDTH - THUMB_BORDER * 2.0, CONTROL_HEIGHT - THUMB_BORDER * 2.0), 1.5, FIELD);
        let list_colour = if self.layout == Layout::List { ACCENT } else { MUTED };
        text::draw(pixmap, scale, SLIDER_LEFT, SCALE_TOP, "List", &FONT_6X10, list_colour);
        let max = format!("{}", self.max.round());
        text::draw(pixmap, scale, SLIDER_LEFT + SLIDER_WIDTH - text::width(&max, &FONT_6X10), SCALE_TOP, &max, &FONT_6X10, MUTED);

        // The readout.
        if self.layout == Layout::List {
            text::draw(pixmap, scale, READOUT_LEFT, CONTROL_TOP + 3.0, "List", &FONT_10X20, INK);
        } else {
            let value = format!("{}", grid_width(self.width).round());
            text::draw(pixmap, scale, READOUT_LEFT, CONTROL_TOP + 3.0, &value, &FONT_10X20, INK);
            text::draw(pixmap, scale, READOUT_LEFT + text::width(&value, &FONT_10X20) + 4.0, CONTROL_TOP + 11.0, "px", &FONT_7X13, MUTED);
        }

        // The transform picker.
        text::draw(pixmap, scale, PICKER_LEFT, LABEL_TOP, "CSS TRANSFORM", &FONT_6X10, MUTED);
        let options = option_rects();
        let box_width = options.iter().map(Rect::width).sum::<f32>() + PICKER_PADDING * 2.0;
        draw::fill(pixmap, scale, Rect::from_xywh(PICKER_LEFT, CONTROL_TOP + (CONTROL_HEIGHT - OPTION_HEIGHT) / 2.0 - PICKER_PADDING, box_width, OPTION_HEIGHT + PICKER_PADDING * 2.0), 8.0, LINE_SOFT);
        for (choice, rect) in Transform3::ALL.into_iter().zip(options) {
            let selected = choice == self.transform;
            if selected {
                draw::fill(pixmap, scale, Some(rect), 6.0, FIELD);
            }
            text::draw(pixmap, scale, rect.left() + OPTION_PADDING, rect.top() + 4.0, choice.label(), &FONT_7X13, if selected { INK } else { MUTED });
        }

        // The channel's identity, at the right edge.
        let meta = format!("{PAGE_COUNT} teletext pages");
        let text_width = text::width(CHANNEL_TITLE, &FONT_10X20).max(text::width(&meta, &FONT_7X13));
        let text_left = width - GUTTER - text_width;
        let mosaic_left = text_left - IDENTITY_GAP - MOSAIC_SIZE;
        let mosaic_top = CONTROL_TOP + CONTROL_HEIGHT / 2.0 - MOSAIC_SIZE / 2.0 - 6.0;
        if let Some(path) = Rect::from_xywh(mosaic_left, mosaic_top, MOSAIC_SIZE, MOSAIC_SIZE).and_then(|rect| draw::rounded(rect, 8.0)) {
            let stops = vec![
                GradientStop::new(0.0, draw::float(colour(0xff, 0x4d, 0x4d))),
                GradientStop::new(0.34, draw::float(colour(0xff, 0xd8, 0x4d))),
                GradientStop::new(0.67, draw::float(colour(0x4d, 0xff, 0x88))),
                GradientStop::new(1.0, draw::float(colour(0x4d, 0xd2, 0xff))),
            ];
            let start = Point::from_xy(mosaic_left, mosaic_top);
            let end = Point::from_xy(mosaic_left + MOSAIC_SIZE, mosaic_top + MOSAIC_SIZE);
            if let Some(shader) = LinearGradient::new(start, end, stops, SpreadMode::Pad, Transform::identity()) {
                let mut paint = draw::paint(INK);
                paint.shader = shader;
                pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, Transform::from_scale(scale, scale), None);
            }
        }
        text::draw(pixmap, scale, text_left, mosaic_top - 2.0, CHANNEL_TITLE, &FONT_10X20, INK);
        text::draw(pixmap, scale, text_left, mosaic_top + 22.0, &meta, &FONT_7X13, MUTED);
    }
}

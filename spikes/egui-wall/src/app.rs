//! The tab: the header with its controls, and the wall below. The tab owns
//! the width, the committed layout and the transform.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, CentralPanel, Color32, Context, CornerRadius, Frame, Layout as EguiLayout, Margin, Mesh, Panel,
    RichText, ScrollArea, Sense, Slider, Stroke, Ui, Vec2,
};

use crate::pages::{PAGE_COUNT, Page, pages};
use crate::size::{
    DEFAULT_WIDTH, LIST_STOP, Layout, SWITCH_DWELL_SECONDS, clamp_width, grid_width, layout_for_width,
    max_width_for_window, next_layout,
};
use crate::theme::{ACCENT, FIELD, INK, LINE, LINE_SOFT, MOSAIC, MUTED, RAISED, WELL};
use crate::wall::{self, Transform};

const SLIDER_WIDTH: f32 = 256.0;
const READOUT_WIDTH: f32 = 140.0;
const LABEL_SIZE: f32 = 11.0;
const READOUT_SIZE: f32 = 20.0;
const TITLE_SIZE: f32 = 21.0;
const META_SIZE: f32 = 12.0;
const MOSAIC_SIZE: f32 = 36.0;
const HEADER_MARGIN: Margin = Margin::symmetric(24, 14);
const WALL_MARGIN: Margin = Margin { left: 24, right: 24, top: 28, bottom: 40 };
const CONTROL_GAP: f32 = 22.0;
/// Room for the channel's identity at the header's right edge.
const IDENTITY_WIDTH: f32 = 320.0;
/// The prototype's CSS transitions: width 160 ms, transforms 420 ms.
const WIDTH_TRANSITION_SECONDS: f32 = 0.160;
const TRANSFORM_TRANSITION_SECONDS: f32 = 0.420;
const CHANNEL_TITLE: &str = "Stand-in channel";

pub struct TabApp {
    pages: Vec<Page>,
    width: f32,
    layout: Layout,
    pending: Option<(Layout, Instant)>,
    transform: Transform,
}

impl TabApp {
    pub fn new(context: &Context) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.selection.bg_fill = ACCENT;
        visuals.widgets.inactive.bg_fill = LINE;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, MUTED);
        visuals.widgets.hovered.bg_fill = LINE;
        visuals.widgets.active.bg_fill = ACCENT;
        visuals.panel_fill = WELL;
        context.set_visuals(visuals);
        Self {
            pages: pages(context),
            width: DEFAULT_WIDTH,
            layout: layout_for_width(DEFAULT_WIDTH),
            pending: None,
            transform: Transform::Flat,
        }
    }

    /// The layout trails the thumb through the hysteresis band and a short
    /// dwell, so a fast sweep across the band does not switch it.
    fn commit_layout(&mut self, context: &Context) {
        let target = next_layout(self.layout, self.width);
        if target == self.layout {
            self.pending = None;
            return;
        }
        let dwell = Duration::from_secs_f32(SWITCH_DWELL_SECONDS);
        match self.pending {
            Some((waiting_for, since)) if waiting_for == target => {
                let waited = since.elapsed();
                if waited >= dwell {
                    self.layout = target;
                    self.pending = None;
                } else {
                    context.request_repaint_after(dwell.saturating_sub(waited));
                }
            }
            _ => {
                self.pending = Some((target, Instant::now()));
                context.request_repaint_after(dwell);
            }
        }
    }

    /// Controls on the left, the channel's identity in a box at the right
    /// edge of the panel.
    fn header(&mut self, ui: &mut Ui, max: f32, panel: egui::Rect) {
        let controls = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CONTROL_GAP;
            self.slider(ui, max);
            self.readout(ui);
            self.transform_picker(ui);
        });
        let height = controls.response.rect.height();
        let right = egui::Rect::from_min_max(
            egui::pos2(panel.right() - IDENTITY_WIDTH, controls.response.rect.top()),
            egui::pos2(panel.right(), controls.response.rect.top() + height),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(EguiLayout::right_to_left(Align::Center)), identity);
    }

    fn slider(&mut self, ui: &mut Ui, max: f32) {
        ui.vertical(|ui| {
            ui.set_width(SLIDER_WIDTH);
            ui.label(RichText::new("THUMBNAIL WIDTH").size(LABEL_SIZE).color(MUTED));
            ui.spacing_mut().slider_width = SLIDER_WIDTH;
            ui.add(Slider::new(&mut self.width, LIST_STOP..=max).show_value(false).trailing_fill(true));
            ui.horizontal(|ui| {
                let list = if self.layout == Layout::List { ACCENT } else { MUTED };
                ui.label(RichText::new("List").size(LABEL_SIZE).color(list));
                ui.with_layout(EguiLayout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("{max}")).size(LABEL_SIZE).monospace().color(MUTED));
                });
            });
        });
    }

    fn readout(&self, ui: &mut Ui) {
        ui.allocate_ui(Vec2::new(READOUT_WIDTH, READOUT_SIZE * 2.0), |ui| {
            ui.set_width(READOUT_WIDTH);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if self.layout == Layout::List {
                    ui.label(RichText::new("List").size(READOUT_SIZE).monospace().color(INK));
                } else {
                    ui.label(RichText::new(format!("{}", grid_width(self.width).round())).size(READOUT_SIZE).monospace().color(INK));
                    ui.label(RichText::new("px").size(META_SIZE).monospace().color(MUTED));
                }
            });
        });
    }

    fn transform_picker(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.label(RichText::new("CSS TRANSFORM").size(LABEL_SIZE).color(MUTED));
            Frame::new().fill(LINE_SOFT).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(2)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for choice in Transform::ALL {
                        let selected = self.transform == choice;
                        let text = RichText::new(choice.label()).size(META_SIZE).color(if selected { INK } else { MUTED });
                        let button = egui::Button::new(text)
                            .fill(if selected { FIELD } else { Color32::TRANSPARENT })
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(6));
                        if ui.add(button).clicked() {
                            self.transform = choice;
                        }
                    }
                });
            });
        });
    }
}

/// The channel's identity: a mosaic stand-in, the title and the page count.
fn identity(ui: &mut Ui) {
    ui.vertical(|ui| {
        ui.label(RichText::new(CHANNEL_TITLE).size(TITLE_SIZE).strong().color(INK));
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{PAGE_COUNT} teletext pages")).size(META_SIZE).color(MUTED));
            Frame::new().stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(6, 1)).show(ui, |ui| {
                ui.label(RichText::new("SAMPLE DATA").size(10.5).strong().color(MUTED));
            });
        });
    });
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(MOSAIC_SIZE), Sense::hover());
    let mut mesh = Mesh::default();
    for (corner, colour) in [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].into_iter().zip(MOSAIC) {
        mesh.colored_vertex(corner, colour);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

impl eframe::App for TabApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        let max = max_width_for_window(ui.max_rect().width());
        self.width = clamp_width(self.width, max);
        self.commit_layout(&context);

        let window = ui.max_rect();
        let panel = window.shrink2(egui::vec2(f32::from(HEADER_MARGIN.left), 0.0));
        Panel::top("header").frame(Frame::new().fill(RAISED).inner_margin(HEADER_MARGIN)).show(ui, |ui| self.header(ui, max, panel));

        let shown_width = context.animate_value_with_time(egui::Id::new("width"), self.width, WIDTH_TRANSITION_SECONDS);
        let tilt = context.animate_value_with_time(egui::Id::new("tilt"), f32::from(u8::from(self.transform == Transform::Tilt)), TRANSFORM_TRANSITION_SECONDS);
        let fan = context.animate_value_with_time(egui::Id::new("fan"), f32::from(u8::from(self.transform == Transform::Fan)), TRANSFORM_TRANSITION_SECONDS);

        CentralPanel::default().frame(Frame::new().fill(WELL).inner_margin(WALL_MARGIN)).show(ui, |ui| {
            // The scroll bar's space is always kept, as the prototype's
            // `scrollbar-gutter: stable` does: otherwise the bar appearing
            // narrows the wall, which changes whether the bar is needed, and
            // the two chase each other for ever.
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    wall::show(ui, &self.pages, shown_width, self.layout, wall::Pose { tilt, fan });
                });
        });
    }
}

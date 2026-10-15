//! The tab's components: the page header with its controls, and the wall.
//! The tab owns the thumbnail size and the transform, and hands them down;
//! controls report changes up through callbacks.

use std::time::Duration;

use dioxus::prelude::*;

use crate::pages::{PAGE_COUNT, Page, pages};
use crate::size::{
    DEFAULT_WIDTH, LIST_STOP, Layout, MAX_WIDTH, SWITCH_DWELL_SECONDS, clamp_width, grid_width, layout_for_width,
    next_layout,
};

const STYLE: &str = include_str!("../assets/tab.css");
/// Where the slider's track sits in the window, fixed by the header's CSS
/// (`.page-header__inner` padding and `.size-slider` placement): Dioxus
/// native cannot measure an element, so these must agree with `tab.css`.
const TRACK_LEFT: f32 = 24.0;
const TRACK_WIDTH: f32 = 256.0;
/// One press of an arrow key moves the slider this far.
const KEY_STEP: f32 = 8.0;
/// Twice a teletext page's native size, the slider's ceiling.
const CHANNEL_TITLE: &str = "Stand-in channel";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    Flat,
    Tilt,
    Fan,
}

impl Transform {
    const ALL: [Self; 3] = [Self::Flat, Self::Tilt, Self::Fan];

    fn name(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Tilt => "tilt",
            Self::Fan => "fan",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Tilt => "Tilt",
            Self::Fan => "Fan",
        }
    }
}

fn width_at(client_x: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation, reason = "window coordinates fit easily in f32")]
    let x = client_x as f32;
    let fraction = ((x - TRACK_LEFT) / TRACK_WIDTH).clamp(0.0, 1.0);
    clamp_width(LIST_STOP + fraction * (MAX_WIDTH - LIST_STOP), MAX_WIDTH)
}

#[component]
pub fn TabApp() -> Element {
    let pages = use_hook(pages);
    let mut width = use_signal(|| DEFAULT_WIDTH);
    let mut layout = use_signal(|| layout_for_width(DEFAULT_WIDTH));
    let mut transform = use_signal(|| Transform::Flat);
    let mut dragging = use_signal(|| false);

    // The layout trails the thumb through the hysteresis band and a short
    // dwell, so a fast sweep across the band does not switch it.
    use_effect(move || {
        let target = next_layout(layout(), width());
        if target != layout() {
            spawn(async move {
                tokio::time::sleep(Duration::from_secs_f32(SWITCH_DWELL_SECONDS)).await;
                if next_layout(layout(), width()) == target {
                    layout.set(target);
                }
            });
        }
    });

    rsx! {
        style { {STYLE} }
        div {
            class: "tab",
            tabindex: 0,
            // Dragging continues wherever the pointer goes in the window.
            onmousemove: move |event| {
                if dragging() {
                    width.set(width_at(event.client_coordinates().x));
                }
            },
            onmouseup: move |_| dragging.set(false),
            onkeydown: move |event| {
                let step = match event.key() {
                    Key::ArrowRight => KEY_STEP,
                    Key::ArrowLeft => -KEY_STEP,
                    _ => return,
                };
                width.set(clamp_width(width() + step, MAX_WIDTH));
            },
            PageHeader {
                width: width(),
                layout: layout(),
                transform: transform(),
                on_transform: move |chosen| transform.set(chosen),
                on_press: move |client_x| {
                    dragging.set(true);
                    width.set(width_at(client_x));
                },
            }
            main { class: "tab-main",
                Wall { pages: pages.clone(), width: width(), layout: layout(), transform: transform() }
            }
        }
    }
}

#[component]
fn PageHeader(
    width: f32,
    layout: Layout,
    transform: Transform,
    on_transform: EventHandler<Transform>,
    on_press: EventHandler<f64>,
) -> Element {
    rsx! {
        header { class: "page-header",
            div { class: "page-header__inner",
                div { class: "page-header__controls",
                    ThumbnailSizeSlider { width, layout, on_press }
                    SizeReadout { width, layout }
                    TransformPicker { selected: transform, on_choose: on_transform }
                }
                div { class: "page-header__identity",
                    div { class: "page-header__mosaic" }
                    div { class: "page-header__text",
                        h1 { class: "page-header__title", "{CHANNEL_TITLE}" }
                        p { class: "page-header__meta",
                            span { "{PAGE_COUNT} teletext pages" }
                            span { class: "page-header__sample", "Sample data" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ThumbnailSizeSlider(width: f32, layout: Layout, on_press: EventHandler<f64>) -> Element {
    let fill = (width - LIST_STOP) / (MAX_WIDTH - LIST_STOP) * 100.0;
    let list_class = if layout == Layout::List { "size-slider__stop size-slider__stop--active" } else { "size-slider__stop" };
    rsx! {
        div { class: "size-slider",
            span { class: "size-slider__label", "Thumbnail width" }
            div {
                class: "size-slider__input",
                onmousedown: move |event| on_press.call(event.client_coordinates().x),
                div { class: "size-slider__track",
                    div { class: "size-slider__fill", style: "width: {fill}%" }
                }
                div { class: "size-slider__thumb", style: "left: calc({fill}% - 5px)" }
            }
            div { class: "size-slider__scale",
                span { class: list_class, "List" }
                span { "{MAX_WIDTH}" }
            }
        }
    }
}

#[component]
fn SizeReadout(width: f32, layout: Layout) -> Element {
    let width = grid_width(width).round();
    rsx! {
        div { class: "size-readout",
            if layout == Layout::List {
                span { class: "size-readout__value", "List" }
            } else {
                span { class: "size-readout__value size-readout__value--width", "{width}" }
                span { class: "size-readout__unit", "px" }
            }
        }
    }
}

#[component]
fn TransformPicker(selected: Transform, on_choose: EventHandler<Transform>) -> Element {
    rsx! {
        div { class: "transform-picker",
            span { class: "transform-picker__legend", "CSS transform" }
            div { class: "transform-picker__options",
                for choice in Transform::ALL {
                    span {
                        key: "{choice.name()}",
                        class: if choice == selected { "transform-picker__option transform-picker__option--checked" } else { "transform-picker__option" },
                        onclick: move |_| on_choose.call(choice),
                        "{choice.label()}"
                    }
                }
            }
        }
    }
}

#[component]
fn Wall(pages: Vec<Page>, width: f32, layout: Layout, transform: Transform) -> Element {
    let card_width = if layout == Layout::Grid { grid_width(width) } else { width };
    let layout_class = if layout == Layout::List { "list" } else { "grid" };
    let transform_class = transform.name();
    rsx! {
        div { class: "wall-stage wall-stage--{transform_class}",
            section { class: "wall wall--{layout_class} wall--{transform_class}",
                for page in pages {
                    PageCard { key: "{page.number}", page, width: card_width, layout }
                }
            }
        }
    }
}

#[component]
fn PageCard(page: Page, width: f32, layout: Layout) -> Element {
    let card_style = if layout == Layout::Grid { format!("width: {width}px") } else { String::new() };
    let kilobytes = format!("{:.1}", f64::from(u32::try_from(page.bytes).unwrap_or(u32::MAX)) / 1024.0);
    rsx! {
        figure { class: "page-card", style: "{card_style}",
            div { class: "page-card__frame",
                img { class: "page-card__image", src: "{page.address}", alt: "Page {page.number:03X}" }
            }
            figcaption { class: "page-card__caption",
                span { class: "page-card__title", "P{page.number:03X}" }
                span { class: "page-card__template", "channel {page.channel}" }
                dl { class: "page-card__details",
                    div { dt { "Page" } dd { "{page.number:03X}" } }
                    div { dt { "Picture" } dd { "480 × 500" } }
                    div { dt { "Image" } dd { "{kilobytes} KB" } }
                }
            }
        }
    }
}

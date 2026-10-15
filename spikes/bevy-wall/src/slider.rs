//! The header: the thumbnail width slider, its scale and its readout. The
//! slider is Bevy's standard headless slider, styled like the prototype's:
//! a thin track, filled in the accent colour up to a tall, narrow thumb.
//! It owns the width, commits the layout after the hysteresis and dwell of
//! `size.rs`, and announces both in a `ThumbnailSize` message.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Slider, SliderRange, SliderThumb, SliderValue, TrackClick, ValueChange, observe};

use crate::ThumbnailSize;
use crate::size::{
    DEFAULT_WIDTH, LIST_STOP, Layout, MAX_WIDTH, SWITCH_DWELL_SECONDS, clamp_width, grid_width, layout_for_width,
    max_width_for_window, next_layout,
};
use crate::theme::{ACCENT, FIELD, LABEL_SIZE, LINE, MUTED, READOUT_SIZE, TEXT};

const HEADER_PADDING: f32 = 16.0;
const SLIDER_WIDTH: f32 = 256.0;
const TRACK_HEIGHT: f32 = 6.0;
const THUMB_WIDTH: f32 = 10.0;
const THUMB_HEIGHT: f32 = 26.0;
const THUMB_BORDER: f32 = 2.0;
const THUMB_RADIUS: f32 = 3.0;
const STACK_GAP: f32 = 6.0;
/// One press of an arrow key moves the slider this far.
const KEY_STEP: f32 = 8.0;

pub struct SliderPlugin;

impl Plugin for SliderPlugin {
    fn build(&self, app: &mut App) {
        let layout = layout_for_width(DEFAULT_WIDTH);
        app.insert_resource(Thumbnail { width: DEFAULT_WIDTH, max: MAX_WIDTH, layout, pending: None, announced: None })
            .add_systems(Update, (follow_window, step_with_keys, commit_layout, announce, style).chain());
    }
}

/// The slider's state: where the thumb is, the window's ceiling, the
/// committed layout, and a layout waiting out its dwell.
#[derive(Resource)]
struct Thumbnail {
    width: f32,
    max: f32,
    layout: Layout,
    pending: Option<(Layout, Timer)>,
    announced: Option<ThumbnailSize>,
}

#[derive(Component)]
struct WidthSlider;
#[derive(Component)]
struct Fill;
#[derive(Component)]
struct Thumb;
#[derive(Component)]
struct ListStop;
#[derive(Component)]
struct MaxLabel;
#[derive(Component)]
struct Readout;

fn label(text: &str, size: f32, colour: Color) -> impl Bundle {
    (Text::new(text), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(colour))
}

pub fn spawn_header(commands: &mut Commands) -> Entity {
    commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(24),
                padding: UiRect::all(px(HEADER_PADDING)),
                border: UiRect::bottom(px(1)),
                ..default()
            },
            BorderColor::all(LINE),
            children![
                (
                    Node { flex_direction: FlexDirection::Column, row_gap: px(STACK_GAP), width: px(SLIDER_WIDTH), ..default() },
                    children![
                        label("THUMBNAIL WIDTH", LABEL_SIZE, MUTED),
                        slider(),
                        (
                            Node { justify_content: JustifyContent::SpaceBetween, ..default() },
                            children![(label("List", LABEL_SIZE, MUTED), ListStop), (label("", LABEL_SIZE, MUTED), MaxLabel)],
                        ),
                    ],
                ),
                (label("", READOUT_SIZE, TEXT), Readout),
            ],
        ))
        .id()
}

fn slider() -> impl Bundle {
    (
        Node { height: px(THUMB_HEIGHT), justify_content: JustifyContent::Center, flex_direction: FlexDirection::Column, ..default() },
        WidthSlider,
        Hovered::default(),
        Slider { track_click: TrackClick::Snap, ..default() },
        SliderValue(DEFAULT_WIDTH),
        SliderRange::new(LIST_STOP, MAX_WIDTH),
        observe(on_value_change),
        children![
            // The track, with the played-through part filled.
            (
                Node { height: px(TRACK_HEIGHT), border_radius: BorderRadius::all(px(TRACK_HEIGHT)), overflow: Overflow::clip(), ..default() },
                BackgroundColor(LINE),
                children![(Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(ACCENT), Fill)],
            ),
            // A track narrower by the thumb's width, so the thumb can be
            // placed by percentage without measuring it.
            (
                Node { position_type: PositionType::Absolute, left: px(0), right: px(THUMB_WIDTH), top: px(0), bottom: px(0), ..default() },
                children![(
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(THUMB_WIDTH),
                        height: px(THUMB_HEIGHT),
                        left: percent(0),
                        border: UiRect::all(px(THUMB_BORDER)),
                        border_radius: BorderRadius::all(px(THUMB_RADIUS)),
                        ..default()
                    },
                    BackgroundColor(FIELD),
                    BorderColor::all(ACCENT),
                    SliderThumb,
                    Thumb,
                )],
            ),
        ],
    )
}

/// The standard slider reports a new value; it shows what this sets.
#[allow(clippy::needless_pass_by_value, reason = "Bevy hands system parameters over by value")]
fn on_value_change(change: On<ValueChange<f32>>, mut thumbnail: ResMut<Thumbnail>) {
    thumbnail.width = clamp_width(change.value, thumbnail.max);
}

/// A narrower window lowers the ceiling and pulls the width down with it.
/// The slider's value and range are replaced, never edited: Bevy keeps them
/// immutable so the widget always sees the change.
fn follow_window(
    windows: Query<&Window>,
    mut thumbnail: ResMut<Thumbnail>,
    sliders: Query<Entity, With<WidthSlider>>,
    mut commands: Commands,
) {
    let Ok(window) = windows.single() else { return };
    let max = max_width_for_window(window.width());
    if (max - thumbnail.max).abs() > f32::EPSILON {
        thumbnail.max = max;
        thumbnail.width = clamp_width(thumbnail.width, max);
        for slider in &sliders {
            commands.entity(slider).insert(SliderRange::new(LIST_STOP, max));
        }
    }
}

#[allow(clippy::needless_pass_by_value, reason = "Bevy hands system parameters over by value")]
fn step_with_keys(keys: Res<ButtonInput<KeyCode>>, mut thumbnail: ResMut<Thumbnail>) {
    let step = if keys.just_pressed(KeyCode::ArrowRight) {
        KEY_STEP
    } else if keys.just_pressed(KeyCode::ArrowLeft) {
        -KEY_STEP
    } else {
        return;
    };
    thumbnail.width = clamp_width(thumbnail.width + step, thumbnail.max);
}

/// The layout trails the thumb through the hysteresis band and the dwell.
#[allow(clippy::needless_pass_by_value, reason = "Bevy hands system parameters over by value")]
fn commit_layout(time: Res<Time>, mut thumbnail: ResMut<Thumbnail>) {
    let target = next_layout(thumbnail.layout, thumbnail.width);
    if target == thumbnail.layout {
        // Written only when something was pending, so an idle slider is not
        // marked as changed every frame.
        if thumbnail.pending.is_some() {
            thumbnail.pending = None;
        }
        return;
    }
    let restart = thumbnail.pending.as_ref().is_none_or(|(waiting_for, _)| *waiting_for != target);
    if restart {
        thumbnail.pending = Some((target, Timer::from_seconds(SWITCH_DWELL_SECONDS, TimerMode::Once)));
    }
    let finished = thumbnail.pending.as_mut().is_some_and(|(_, timer)| timer.tick(time.delta()).is_finished());
    if finished {
        thumbnail.layout = target;
        thumbnail.pending = None;
    }
}

fn announce(mut thumbnail: ResMut<Thumbnail>, mut sizes: MessageWriter<ThumbnailSize>) {
    let current = ThumbnailSize { width: thumbnail.width, layout: thumbnail.layout };
    if thumbnail.announced != Some(current) {
        sizes.write(current);
        thumbnail.announced = Some(current);
    }
}

type ThumbQuery<'w, 's> = Query<'w, 's, &'static mut Node, (With<Thumb>, Without<Fill>)>;
type FillQuery<'w, 's> = Query<'w, 's, &'static mut Node, (With<Fill>, Without<Thumb>)>;

type ReadoutQuery<'w, 's> = Query<'w, 's, &'static mut Text, (With<Readout>, Without<MaxLabel>, Without<ListStop>)>;
type MaxLabelQuery<'w, 's> = Query<'w, 's, &'static mut Text, (With<MaxLabel>, Without<Readout>, Without<ListStop>)>;

#[allow(clippy::too_many_arguments, reason = "a Bevy system takes each query it uses as an argument")]
#[allow(clippy::needless_pass_by_value, reason = "Bevy hands system parameters over by value")]
fn style(
    thumbnail: Res<Thumbnail>,
    sliders: Query<(Entity, &SliderValue), With<WidthSlider>>,
    mut commands: Commands,
    mut thumbs: ThumbQuery,
    mut fills: FillQuery,
    mut readouts: ReadoutQuery,
    mut max_labels: MaxLabelQuery,
    mut list_stops: Query<&mut TextColor, With<ListStop>>,
) {
    if !thumbnail.is_changed() {
        return;
    }
    for (slider, value) in &sliders {
        if (value.0 - thumbnail.width).abs() > f32::EPSILON {
            commands.entity(slider).insert(SliderValue(thumbnail.width));
        }
        let position = SliderRange::new(LIST_STOP, thumbnail.max).thumb_position(thumbnail.width) * 100.0;
        for mut thumb in &mut thumbs {
            thumb.left = percent(position);
        }
        for mut fill in &mut fills {
            fill.width = percent(position);
        }
    }
    let is_list = thumbnail.layout == Layout::List;
    for mut readout in &mut readouts {
        readout.0 = if is_list { "List".into() } else { format!("{} px wide", grid_width(thumbnail.width).round()) };
    }
    for mut max_label in &mut max_labels {
        max_label.0 = format!("{}", thumbnail.max.round());
    }
    for mut colour in &mut list_stops {
        colour.0 = if is_list { ACCENT } else { MUTED };
    }
}

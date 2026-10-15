//! The wall of pages: a centred, wrapping grid of page images with captions,
//! or, in the list layout, one page a row with its details beside it. It
//! sizes itself only from `ThumbnailSize` messages.

use bevy::prelude::*;

use crate::ThumbnailSize;
use crate::pages::Page;
use crate::size::{Layout, PAGE_ASPECT, grid_width};
use crate::theme::{CAPTION_SIZE, LINE, MUTED, TEXT};

const WALL_PADDING: f32 = 24.0;
const WALL_GAP: f32 = 14.0;
const ROW_GAP: f32 = WALL_GAP * 2.0;
const CARD_GAP: f32 = 8.0;
const LIST_GAP: f32 = 16.0;
/// The list is no wider than this, as in the prototype (54 rem).
const LIST_MAX_WIDTH: f32 = 864.0;
const FRAME_RADIUS: f32 = 7.0;

pub struct WallPlugin;

impl Plugin for WallPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, resize);
    }
}

#[derive(Component)]
struct Wall;
#[derive(Component)]
struct Card;
#[derive(Component)]
struct Frame;
#[derive(Component)]
struct Details;

fn text(content: String, colour: Color) -> impl Bundle {
    (Text::new(content), TextFont { font_size: FontSize::Px(CAPTION_SIZE), ..default() }, TextColor(colour))
}

fn card(page: &Page, image: Handle<Image>) -> impl Bundle {
    (
        Node { flex_direction: FlexDirection::Column, row_gap: px(CARD_GAP), ..default() },
        Card,
        children![
            (
                Node { aspect_ratio: Some(PAGE_ASPECT), border_radius: BorderRadius::all(px(FRAME_RADIUS)), overflow: Overflow::clip(), ..default() },
                BackgroundColor(LINE),
                Frame,
                children![(Node { width: percent(100), height: percent(100), ..default() }, ImageNode::new(image))],
            ),
            (
                Node { flex_direction: FlexDirection::Column, row_gap: px(2), ..default() },
                Details,
                children![
                    text(format!("P{:03X}", page.number), TEXT),
                    text(format!("channel {}", page.channel), MUTED),
                ],
            ),
        ],
    )
}

pub fn spawn_wall(commands: &mut Commands, images: &mut Assets<Image>, pages: Vec<Page>) -> Entity {
    let wall = commands
        .spawn((
            Node {
                flex_grow: 1.0,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                align_content: AlignContent::FlexStart,
                column_gap: px(WALL_GAP),
                row_gap: px(ROW_GAP),
                padding: UiRect::all(px(WALL_PADDING)),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            ScrollPosition::default(),
            Wall,
        ))
        .id();
    for page in pages {
        let image = images.add(page.image.clone());
        let card = commands.spawn(card(&page, image)).id();
        commands.entity(wall).add_child(card);
    }
    wall
}

type WallQuery<'w, 's> = Query<'w, 's, &'static mut Node, (With<Wall>, Without<Card>, Without<Frame>)>;
type CardQuery<'w, 's> = Query<'w, 's, &'static mut Node, (With<Card>, Without<Wall>, Without<Frame>)>;
type FrameQuery<'w, 's> = Query<'w, 's, &'static mut Node, (With<Frame>, Without<Wall>, Without<Card>)>;

fn resize(
    mut sizes: MessageReader<ThumbnailSize>,
    mut walls: WallQuery,
    mut cards: CardQuery,
    mut frames: FrameQuery,
) {
    let Some(size) = sizes.read().last().copied() else { return };
    let list = size.layout == Layout::List;
    for mut wall in &mut walls {
        // The list is one centred column; the grid wraps rows of pages.
        wall.flex_direction = if list { FlexDirection::Column } else { FlexDirection::Row };
        wall.align_items = if list { AlignItems::Center } else { AlignItems::FlexStart };
        wall.row_gap = px(if list { 0.0 } else { ROW_GAP });
    }
    for mut card in &mut cards {
        card.flex_direction = if list { FlexDirection::Row } else { FlexDirection::Column };
        card.column_gap = px(LIST_GAP);
        card.width = if list { percent(100) } else { px(grid_width(size.width)) };
        card.max_width = if list { px(LIST_MAX_WIDTH) } else { Val::Auto };
        card.padding = UiRect::vertical(px(if list { CARD_GAP } else { 0.0 }));
    }
    for mut frame in &mut frames {
        frame.width = if list { px(size.width) } else { percent(100) };
        frame.flex_shrink = 0.0;
    }
}

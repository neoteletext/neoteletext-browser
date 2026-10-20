//! The React prototype's page wall and thumbnail width slider, drawn with
//! Bevy instead of a web engine: a header with the slider, and a scrolling
//! wall of teletext pages that resizes, and switches between a grid and a
//! list, as the slider moves.

mod pages;
mod scroll;
mod size;
mod slider;
mod theme;
mod wall;

use bevy::prelude::*;
use bevy::window::WindowResolution;
use bevy::winit::WinitSettings;

const WINDOW_TITLE: &str = "NeoTeletext: Bevy page wall";
const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 820;

/// The width the slider has set, and the layout committed for it. The slider
/// and the wall agree through the `ThumbnailSize` message, as the React
/// components agree through one event, and never read each other.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct ThumbnailSize {
    pub width: f32,
    pub layout: size::Layout,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: WINDOW_TITLE.into(),
                resolution: WindowResolution::new(WINDOW_WIDTH, WINDOW_HEIGHT),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(theme::BACKGROUND))
        // Redraw only on input or change, as a desktop application should,
        // not every frame as a game does.
        .insert_resource(WinitSettings::desktop_app())
        .add_message::<ThumbnailSize>()
        .add_plugins((slider::SliderPlugin, wall::WallPlugin, scroll::ScrollPlugin))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.spawn(Camera2d);
    let pages = pages::pages();
    let root = commands
        .spawn(Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            ..default()
        })
        .id();
    let header = slider::spawn_header(&mut commands);
    let wall = wall::spawn_wall(&mut commands, &mut images, pages);
    commands.entity(root).add_children(&[header, wall]);
}

//! Scrolling the wall with the mouse wheel or trackpad, as in Bevy's own
//! scrolling example: wheel movement is sent to whatever is under the pointer
//! and travels up the hierarchy until a scrollable node takes it.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;

/// Logical pixels scrolled for one wheel line.
const LINE_HEIGHT: f32 = 21.0;

pub struct ScrollPlugin;

impl Plugin for ScrollPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, send_scroll).add_observer(on_scroll);
    }
}

#[derive(EntityEvent, Debug)]
#[entity_event(propagate, auto_propagate)]
struct Scroll {
    entity: Entity,
    delta: Vec2,
}

#[allow(clippy::needless_pass_by_value, reason = "Bevy hands system parameters over by value")]
fn send_scroll(mut wheel: MessageReader<MouseWheel>, hover_map: Res<HoverMap>, mut commands: Commands) {
    for movement in wheel.read() {
        let mut delta = -Vec2::new(movement.x, movement.y);
        if movement.unit == MouseScrollUnit::Line {
            delta *= LINE_HEIGHT;
        }
        for pointer in hover_map.values() {
            for entity in pointer.keys().copied() {
                commands.trigger(Scroll { entity, delta });
            }
        }
    }
}

fn on_scroll(mut scroll: On<Scroll>, mut nodes: Query<(&mut ScrollPosition, &Node, &ComputedNode)>) {
    let Ok((mut position, node, computed)) = nodes.get_mut(scroll.entity) else { return };
    if node.overflow.y != OverflowAxis::Scroll || scroll.delta.y == 0.0 {
        return;
    }
    let max = (computed.content_size().y - computed.size().y) * computed.inverse_scale_factor();
    position.y = (position.y + scroll.delta.y).clamp(0.0, max.max(0.0));
    scroll.propagate(false);
}

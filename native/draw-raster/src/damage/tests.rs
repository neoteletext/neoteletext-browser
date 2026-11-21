//! The damage's tests: a call's box of pixels, boxes joined or kept apart
//! within their bound, and the common ends of two lists of calls.

use super::*;

const SURFACE: DeviceBox = DeviceBox::whole(100, 50);

#[test]
fn a_rect_covers_every_pixel_it_touches_and_a_margin() {
    let rect = Rect { x0: 10.25, y0: 4.5, x1: 20.75, y1: 5.0 };
    assert_eq!(DeviceBox::covering(rect, 2.0, SURFACE), DeviceBox { x0: 19, y0: 8, x1: 43, y1: 11 });
}

#[test]
fn a_rect_is_held_inside_the_surface() {
    let rect = Rect { x0: -30.0, y0: -1.0, x1: 400.0, y1: 20.0 };
    assert_eq!(DeviceBox::covering(rect, 1.0, SURFACE), DeviceBox { x0: 0, y0: 0, x1: 100, y1: 21 });
    let outside = Rect { x0: 200.0, y0: 0.0, x1: 300.0, y1: 10.0 };
    assert!(DeviceBox::covering(outside, 1.0, SURFACE).is_empty());
}

#[test]
fn an_empty_box_adds_nothing_to_a_union() {
    let one = DeviceBox { x0: 5, y0: 5, x1: 6, y1: 7 };
    assert_eq!(one.union(DeviceBox::default()), one);
    assert_eq!(DeviceBox::default().union(one), one);
    assert_eq!(one.union(DeviceBox { x0: 1, y0: 9, x1: 2, y1: 10 }), DeviceBox { x0: 1, y0: 5, x1: 6, y1: 10 });
    assert!(!one.meets(DeviceBox { x0: 6, y0: 5, x1: 8, y1: 7 }), "boxes that only touch share no pixel");
}

#[test]
fn damage_joins_boxes_that_meet_and_keeps_apart_those_that_do_not() {
    let mut damage = Damage::default();
    damage.add(DeviceBox { x0: 0, y0: 0, x1: 4, y1: 4 });
    damage.add(DeviceBox { x0: 50, y0: 0, x1: 54, y1: 4 });
    damage.add(DeviceBox::default());
    assert_eq!(damage.boxes().len(), 2);
    // Meets both: all three become one.
    damage.add(DeviceBox { x0: 2, y0: 1, x1: 52, y1: 2 });
    assert_eq!(damage.boxes(), [DeviceBox { x0: 0, y0: 0, x1: 54, y1: 4 }]);
}

#[test]
fn past_the_most_boxes_the_nearest_are_joined() {
    let mut damage = Damage::default();
    let far = u16::try_from(MAX_BOXES).unwrap() * 10;
    for step in 0..=u16::try_from(MAX_BOXES).unwrap() {
        damage.add(DeviceBox { x0: step * 10, y0: 0, x1: step * 10 + 2, y1: 2 });
    }
    assert_eq!(damage.boxes().len(), MAX_BOXES);
    let covered: u32 = damage.boxes().iter().map(|area| area.area()).sum();
    assert!(covered < 2 * 2 * u32::try_from(MAX_BOXES).unwrap() + 12 * 2, "only two neighbours joined: {covered}");
    assert!(damage.boxes().iter().any(|area| area.x1 == far + 2), "the last box is kept");
}

#[test]
fn the_calls_that_changed_lie_between_the_common_ends() {
    let same = |a: &u8, b: &u8| a == b;
    assert_eq!(common_ends(&[1, 2, 3, 4], &[1, 2, 3, 4], same), (4, 0));
    assert_eq!(common_ends(&[1, 2, 3, 4], &[1, 9, 3, 4], same), (1, 2));
    assert_eq!(common_ends(&[1, 2, 4], &[1, 2, 3, 4], same), (2, 1));
    assert_eq!(common_ends(&[1, 1], &[1, 1, 1], same), (2, 0), "the ends never overlap");
    assert_eq!(common_ends(&[], &[5], same), (0, 0));
}

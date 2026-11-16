#![allow(clippy::unwrap_used, reason = "a test")]

use super::*;

/// Decision 124: every window is given the browser's icon, the art's own
/// pixels as the interface was built with them, where the platform shows
/// a window's icon.
#[cfg(not(target_os = "macos"))]
#[test]
fn every_window_carries_the_browsers_icon() {
    let art = icon::source().unwrap();
    assert_eq!(WINDOW_ICON, art.rgba());
    assert_eq!(WINDOW_ICON_SIDE, art.width());
    assert!(attributes(None).window_icon.is_some());
    assert!(icon().is_some());
}

/// Decision 124: outside a bundle the Dock is given the art at
/// `DOCK_SIDE`, by nearest neighbour.
#[cfg(target_os = "macos")]
#[test]
fn the_dock_is_given_the_art() {
    let art = icon::source().unwrap();
    assert_eq!(icon::png::decode(DOCK_ICON).unwrap(), art.scaled(icon::DOCK_SIDE).unwrap());
}

/// REQ-6396: a press within the margin of an edge resizes by that edge;
/// within the margin and within the corner's reach along it, by the
/// corner, whose zone is larger than the margin; further in
/// than the margin by none, however near a corner.
#[test]
fn an_edge_within_the_margin_resizes_and_a_corner_reaches_along_it() {
    use winit::window::ResizeDirection;
    let (width, height) = (1_000.0, 640.0);
    let inside = RESIZE_MARGIN - 1.0;
    let (right, bottom) = (width - inside, height - inside);
    let reach = RESIZE_CORNER - 1.0;
    let edge = |x: f64, y: f64| resize_edge(x, y, width, height);
    assert_eq!(edge(inside, 300.0), Some(ResizeDirection::West));
    assert_eq!(edge(right, 300.0), Some(ResizeDirection::East));
    assert_eq!(edge(500.0, inside), Some(ResizeDirection::North));
    assert_eq!(edge(500.0, bottom), Some(ResizeDirection::South));
    assert_eq!(edge(inside, inside), Some(ResizeDirection::NorthWest));
    assert_eq!(edge(right, inside), Some(ResizeDirection::NorthEast));
    assert_eq!(edge(inside, bottom), Some(ResizeDirection::SouthWest));
    assert_eq!(edge(right, bottom), Some(ResizeDirection::SouthEast));
    // The corner's reach along each edge, beyond the margin's own square.
    assert_eq!(edge(reach, inside), Some(ResizeDirection::NorthWest), "along the top from the left corner");
    assert_eq!(edge(inside, reach), Some(ResizeDirection::NorthWest), "down the left from the top corner");
    assert_eq!(edge(width - reach, bottom), Some(ResizeDirection::SouthEast));
    assert_eq!(edge(right, height - reach), Some(ResizeDirection::SouthEast));
    assert_eq!(edge(RESIZE_CORNER, inside), Some(ResizeDirection::North), "past the corner's reach, the edge");
    assert_eq!(edge(inside, RESIZE_CORNER), Some(ResizeDirection::West));
    assert_eq!(edge(reach, reach), None, "near a corner but in no margin");
    assert_eq!(edge(RESIZE_MARGIN, 300.0), None, "at the margin itself, the window's own");
    assert_eq!(edge(500.0, 300.0), None);
}

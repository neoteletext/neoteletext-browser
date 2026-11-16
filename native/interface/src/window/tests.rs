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

/// A press within the margin of an edge resizes by that edge, of two
/// edges by their corner, and further in by none: the pointer's shape at
/// the edge and the resize a press starts there come from this one answer.
#[test]
fn an_edge_within_the_margin_resizes_and_a_corner_wins() {
    use winit::window::ResizeDirection;
    let (width, height) = (1_000.0, 640.0);
    let inside = RESIZE_MARGIN - 1.0;
    let (right, bottom) = (width - inside, height - inside);
    let edge = |x: f64, y: f64| resize_edge(x, y, width, height);
    assert_eq!(edge(inside, 300.0), Some(ResizeDirection::West));
    assert_eq!(edge(right, 300.0), Some(ResizeDirection::East));
    assert_eq!(edge(500.0, inside), Some(ResizeDirection::North));
    assert_eq!(edge(500.0, bottom), Some(ResizeDirection::South));
    assert_eq!(edge(inside, inside), Some(ResizeDirection::NorthWest));
    assert_eq!(edge(right, inside), Some(ResizeDirection::NorthEast));
    assert_eq!(edge(inside, bottom), Some(ResizeDirection::SouthWest));
    assert_eq!(edge(right, bottom), Some(ResizeDirection::SouthEast));
    assert_eq!(edge(RESIZE_MARGIN, 300.0), None, "at the margin itself, the window's own");
    assert_eq!(edge(500.0, 300.0), None);
    assert_eq!(edge(width - RESIZE_MARGIN, height - RESIZE_MARGIN), None);
}

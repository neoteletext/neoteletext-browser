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

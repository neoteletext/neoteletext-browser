#![allow(clippy::unwrap_used, reason = "a test")]

use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!("neoteletext-icon-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

/// The iconset written is the one the check accepts, and a file changed,
/// missing or extra is refused.
#[test]
fn an_iconset_is_written_and_checked() {
    let folder = scratch("iconset").with_extension("iconset");
    write_iconset(&folder).unwrap();
    assert!(write_iconset(&folder).is_err(), "never into a folder that exists");
    check_iconset(&folder).unwrap();
    let small = folder.join("icon_16x16.png");
    let other = png::encode(&crate::source().unwrap().scaled(16).unwrap().scaled(8).unwrap().scaled(16).unwrap()).unwrap();
    std::fs::write(&small, other).unwrap();
    assert!(check_iconset(&folder).is_err(), "a different picture");
    std::fs::remove_file(&small).unwrap();
    assert!(check_iconset(&folder).is_err(), "a missing size");
    std::fs::remove_dir_all(&folder).unwrap();
    write_iconset(&folder).unwrap();
    std::fs::write(folder.join("extra.png"), b"").unwrap();
    assert!(check_iconset(&folder).is_err(), "an extra file");
    std::fs::remove_dir_all(&folder).unwrap();
}

/// By hand on macOS: Apple's `iconutil` makes an icon file of the iconset
/// and gives back the same pixels at every size.
#[test]
#[ignore = "runs Apple's iconutil; by hand on macOS"]
fn iconutil_keeps_every_pixel() {
    let folder = scratch("iconutil");
    std::fs::create_dir_all(&folder).unwrap();
    let iconset = folder.join("NeoTeletext.iconset");
    write_iconset(&iconset).unwrap();
    let icns = folder.join("NeoTeletext.icns");
    let made = std::process::Command::new("/usr/bin/iconutil").arg("-c").arg("icns").arg("-o").arg(&icns).arg(&iconset).status().unwrap();
    assert!(made.success());
    let back = folder.join("back.iconset");
    let read = std::process::Command::new("/usr/bin/iconutil").arg("-c").arg("iconset").arg("-o").arg(&back).arg(&icns).status().unwrap();
    assert!(read.success());
    check_iconset(&back).unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
}

//! The browser's icon in the interface (decision 124), made from the art
//! when the interface is built, since the confined interface reads no file
//! (requirement 3.6): on Windows and Linux the art's pixels for every
//! window's icon; on macOS a large copy of it, as a PNG, for the Dock when
//! the browser runs outside its bundle. On Windows the program also
//! carries the icon as a resource, as the browser's other programs do.

use std::path::{Path, PathBuf};

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let art = icon::source().expect("the art is read");
    let code = if os == "macos" {
        let dock = art.scaled(icon::DOCK_SIDE).and_then(|image| icon::png::encode(&image)).expect("the Dock's icon is made");
        std::fs::write(out.join("dock-icon.png"), dock).expect("the Dock's icon is written");
        format!(
            "/// The Dock's icon for a program run outside a bundle: the art at {side} pixels, a PNG.\n\
             pub(crate) static DOCK_ICON: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/dock-icon.png\"));\n",
            side = icon::DOCK_SIDE
        )
    } else {
        std::fs::write(out.join("window-icon.rgba"), art.rgba()).expect("the windows' icon is written");
        format!(
            "/// The windows' icon: the art's pixels, red, green, blue and alpha.\n\
             pub(crate) static WINDOW_ICON: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/window-icon.rgba\"));\n\
             /// Its width and height.\n\
             pub(crate) const WINDOW_ICON_SIDE: u32 = {side};\n",
            side = icon::SOURCE_SIDE
        )
    };
    std::fs::write(out.join("icon.rs"), code).expect("the icon's code is written");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|environment| environment == "msvc");
    if os == "windows" && msvc {
        let resource = icon::windows::write_resource_file(Path::new(&out)).expect("the icon's resource file is written");
        println!("cargo::rustc-link-arg-bin=neoteletext-interface={}", resource.display());
    }
}

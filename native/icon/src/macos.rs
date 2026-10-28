//! The macOS bundle's icon file is made by Apple's `iconutil` from an
//! iconset, a folder of PNG files named by size (`iconutil(1)`). This
//! writes that folder from the art and checks one `iconutil` wrote back
//! from the icon file, so the release tool knows the file holds the art at
//! every size, pixel for pixel (measured on 28 October 2026: `iconutil`
//! keeps PNG images as PNG and stores 16 and 32 as its own ARGB, each
//! with the pixels it was given).

use std::io::Read as _;
use std::path::Path;

use crate::{Image, MACOS_ICONSET, png};

/// The largest file of an iconset read back: any PNG [`png::decode`]
/// takes is smaller.
const MAX_READ: u64 = 16 * 1024 * 1024;

/// Every image of the iconset, made from the art: its name and picture.
///
/// # Errors
/// When the art cannot be read.
pub fn iconset() -> Result<Vec<(&'static str, Image)>, String> {
    let art = crate::source().map_err(|refused| refused.to_string())?;
    MACOS_ICONSET.iter().map(|(name, side)| art.scaled(*side).map(|image| (*name, image)).map_err(|refused| refused.to_string())).collect()
}

/// Writes the iconset in `folder`, a new folder whose name ends
/// `.iconset`, as `iconutil` requires.
///
/// # Errors
/// When the folder exists or a file cannot be written.
pub fn write_iconset(folder: &Path) -> Result<(), String> {
    std::fs::create_dir(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    for (name, image) in iconset()? {
        let bytes = png::encode(&image).map_err(|refused| refused.to_string())?;
        let path = folder.join(name);
        std::fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

/// Checks that `folder`, an iconset `iconutil` wrote from an icon file,
/// holds exactly the iconset's files, each the art at its size.
///
/// # Errors
/// What is missing, extra or different.
pub fn check_iconset(folder: &Path) -> Result<(), String> {
    let expected = iconset()?;
    let mut found = Vec::new();
    for entry in std::fs::read_dir(folder).map_err(|error| format!("{}: {error}", folder.display()))? {
        let entry = entry.map_err(|error| format!("{}: {error}", folder.display()))?;
        found.push(entry.file_name().to_string_lossy().into_owned());
    }
    found.sort();
    let mut names: Vec<String> = expected.iter().map(|(name, _)| (*name).to_owned()).collect();
    names.sort();
    if found != names {
        return Err(format!("the icon file holds {found:?}, not {names:?}"));
    }
    for (name, image) in expected {
        let path = folder.join(name);
        let mut bytes = Vec::new();
        std::fs::File::open(&path).and_then(|file| file.take(MAX_READ).read_to_end(&mut bytes)).map_err(|error| format!("{}: {error}", path.display()))?;
        let read = png::decode(&bytes).map_err(|refused| format!("{name}: {refused}"))?;
        if read != image {
            return Err(format!("{name} is not the art at {} pixels", image.width()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

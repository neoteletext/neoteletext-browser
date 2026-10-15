//! Colours and sizes from the prototype's dark theme (`src/shared/tokens.css`).

use bevy::prelude::*;

pub const BACKGROUND: Color = Color::srgb_u8(0x0b, 0x0d, 0x11);
pub const FIELD: Color = Color::srgb_u8(0x0f, 0x11, 0x15);
pub const MUTED: Color = Color::srgb_u8(0x93, 0x99, 0xa6);
pub const LINE: Color = Color::srgb_u8(0x2b, 0x2f, 0x38);
pub const ACCENT: Color = Color::srgb_u8(0x8f, 0xa1, 0xff);
pub const TEXT: Color = Color::srgb_u8(0xe6, 0xe8, 0xee);

pub const LABEL_SIZE: f32 = 11.0;
pub const CAPTION_SIZE: f32 = 13.0;
pub const READOUT_SIZE: f32 = 15.0;

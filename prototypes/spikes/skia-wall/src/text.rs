//! Text in bitmap fonts from `embedded-graphics` (the public-domain X11
//! fixed fonts), drawn straight into the tiny-skia image. Each font dot
//! becomes one logical pixel, so on a high-resolution display it covers
//! `scale` by `scale` device pixels.

use embedded_graphics::Drawable;
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::{DrawTarget, OriginDimensions, Point, RgbColor, Size};
use embedded_graphics::text::{Baseline, Text};
use tiny_skia::{ColorU8, Pixmap};

struct Target<'a> {
    pixmap: &'a mut Pixmap,
    origin: (i32, i32),
    scale: i32,
}

impl OriginDimensions for Target<'_> {
    fn size(&self) -> Size {
        Size::new(self.pixmap.width(), self.pixmap.height())
    }
}

impl DrawTarget for Target<'_> {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I: IntoIterator<Item = embedded_graphics::Pixel<Rgb888>>>(&mut self, pixels: I) -> Result<(), Self::Error> {
        let width = i32::try_from(self.pixmap.width()).unwrap_or(0);
        let height = i32::try_from(self.pixmap.height()).unwrap_or(0);
        let data = self.pixmap.data_mut();
        for embedded_graphics::Pixel(point, colour) in pixels {
            let left = self.origin.0 + point.x * self.scale;
            let top = self.origin.1 + point.y * self.scale;
            for y in top.max(0)..(top + self.scale).min(height) {
                for x in left.max(0)..(left + self.scale).min(width) {
                    let Ok(at) = usize::try_from((y * width + x) * 4) else { continue };
                    data[at..at + 4].copy_from_slice(&[colour.r(), colour.g(), colour.b(), u8::MAX]);
                }
            }
        }
        Ok(())
    }
}

fn rgb(colour: ColorU8) -> Rgb888 {
    Rgb888::new(colour.red(), colour.green(), colour.blue())
}

/// The width of `text` in logical pixels.
pub fn width(text: &str, font: &MonoFont<'_>) -> f32 {
    let advance = font.character_size.width + font.character_spacing;
    let characters = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    #[allow(clippy::cast_precision_loss, reason = "short strings")]
    let width = characters.saturating_mul(advance) as f32;
    width
}

/// Draws `text` with its top-left corner at a logical position.
pub fn draw(pixmap: &mut Pixmap, scale: f32, x: f32, y: f32, text: &str, font: &MonoFont<'_>, colour: ColorU8) {
    #[allow(clippy::cast_possible_truncation, reason = "display scale factors and window positions are small")]
    let (dots, origin) = (scale.round().max(1.0) as i32, ((x * scale).round() as i32, (y * scale).round() as i32));
    let mut target = Target { pixmap, origin, scale: dots };
    let style = MonoTextStyle::new(font, rgb(colour));
    let _ = Text::with_baseline(text, Point::zero(), style, Baseline::Top).draw(&mut target);
}

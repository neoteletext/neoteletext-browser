//! Writes the browser as the microcontroller back end draws it, as a
//! portable pixmap file for inspection:
//! `cargo run --example write_screen -- <file.ppm>`.

use std::convert::Infallible;
use std::io::Write;

use decoder::Engine;
use draw_embedded::Screen;
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use interface_core::browser::{Browser, Frame, StripState, TabInfo};
use interface_core::canvas::PageKey;
use interface_core::channel::{Channel, PageInfo, Status, page_index};

const WIDTH: u32 = 1024;
const HEIGHT: u32 = 600;
const CHANNEL: u32 = 3;

struct Memory(Vec<Rgb888>);

impl OriginDimensions for Memory {
    fn size(&self) -> Size {
        Size::new(WIDTH, HEIGHT)
    }
}

impl DrawTarget for Memory {
    type Color = Rgb888;
    type Error = Infallible;
    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb888>>>(&mut self, pixels: I) -> Result<(), Infallible> {
        for Pixel(at, colour) in pixels {
            if let (Ok(x @ 0..WIDTH), Ok(y @ 0..HEIGHT)) = (u32::try_from(at.x), u32::try_from(at.y)) {
                self.0[(y * WIDTH + x) as usize] = colour;
            }
        }
        Ok(())
    }
}

/// A channel of sample pages, decoded and drawn by the teletext engine, as
/// a microcontroller build would use it.
fn channel_with_pages<D>(screen: &mut Screen<D>) -> Channel
where
    D: DrawTarget,
    D::Color: From<Rgb888>,
{
    let pages: Vec<u16> = (0..6_u16).map(|offset| 0x100 + offset).chain([0x200, 0x300, 0x450]).collect();
    let mut engine = Engine::new();
    let mut arrived = engine.feed(&decoder::sample::tti(&pages, &["Headlines", "World news", "Sport results"]));
    arrived.extend(engine.finish());
    let mut channel = Channel::empty();
    channel.channel = CHANNEL;
    for (page, title) in arrived {
        let Some(index) = page_index(page) else { continue };
        channel.pages[index] = Some(PageInfo { title, status: Status::Loaded });
        if let Some(rendered) = engine.render(page) {
            let _ = screen.set_page(PageKey { channel: CHANNEL, page }, &rendered.palette, rendered.screen, &rendered.indices);
        }
    }
    channel
}

fn main() -> std::io::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "screen.ppm".to_owned());
    let mut screen = Screen::new(Memory(vec![Rgb888::BLACK; (WIDTH * HEIGHT) as usize]));
    let channel = channel_with_pages(&mut screen);
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.set_tabs(
        vec![
            TabInfo { title: "news.invalid".into(), colour: 0, channel: 1, muted: false, private: false, address: "https://example.invalid/".into() },
            TabInfo { title: "sport.invalid".into(), colour: 1, channel: 2, muted: false, private: false, address: "https://example.invalid/".into() },
        ],
        0,
    );
    browser.set_strip(StripState { address: "https://news.invalid/".into(), identity: "news.invalid".into(), encrypted: true, ..StripState::default() });
    browser.set_channel(channel);
    browser.draw(&mut screen, f64::from(WIDTH), f64::from(HEIGHT), 0);

    let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);
    write!(file, "P6\n{WIDTH} {HEIGHT}\n255\n")?;
    for pixel in &screen.target.0 {
        file.write_all(&[pixel.r(), pixel.g(), pixel.b()])?;
    }
    println!("{path}");
    Ok(())
}

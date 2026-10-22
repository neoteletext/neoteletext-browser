//! The browser drawn by the microcontroller back end into a display in
//! memory, at a common microcontroller panel size: the same interface code
//! the desktop runs, with features cut (design section 7).

use std::convert::Infallible;

use decoder::Engine;
use draw_embedded::Screen;
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use interface_core::browser::{Browser, Frame, Output, StripState, TabInfo};
use interface_core::canvas::PageKey;
use interface_core::channel::{Channel, PageInfo, Status, page_index};
use interface_core::theme::{BLACK, WINDOW};

/// A 7-inch panel of 1024 by 600, as driven by many microcontrollers.
const WIDTH: u32 = 1024;
const HEIGHT: u32 = 600;
const CHANNEL: u32 = 3;

struct Memory {
    pixels: Vec<Rgb888>,
}

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
                self.pixels[(y * WIDTH + x) as usize] = colour;
            }
        }
        Ok(())
    }
}

fn rgb(colour: interface_core::theme::Colour) -> Rgb888 {
    Rgb888::new(colour.red, colour.green, colour.blue)
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

fn drawn() -> (Screen<Memory>, Browser) {
    let mut screen = Screen::new(Memory { pixels: vec![Rgb888::BLACK; (WIDTH * HEIGHT) as usize] });
    let channel = channel_with_pages(&mut screen);
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.set_tabs(vec![TabInfo { title: "news.invalid".into(), colour: 0, channel: 1, muted: false, private: false }], 0);
    browser.set_strip(StripState { address: "https://news.invalid/".into(), identity: "news.invalid".into(), encrypted: true, ..StripState::default() });
    browser.set_channel(channel);
    browser.draw(&mut screen, f64::from(WIDTH), f64::from(HEIGHT), 0);
    (screen, browser)
}

#[test]
fn the_browser_draws_with_its_pages() {
    let (screen, mut browser) = drawn();
    // The tab strip's background, right of the tabs and the + button.
    assert_eq!(screen.target.pixels[(2 * WIDTH + WIDTH - 2) as usize], rgb(BLACK));
    // The window behind the panels, in the margin at the bottom.
    assert_eq!(screen.target.pixels[((HEIGHT - 2) * WIDTH + WIDTH / 2) as usize], rgb(WINDOW));
    // The first bank's pages were reported visible, for the trusted core to render.
    let outputs = browser.take_outputs();
    let Some(Output::Visible(visible)) = outputs.first() else { panic!("{outputs:?}") };
    assert!(visible.contains(&PageKey { channel: CHANNEL, page: 0x100 }));
    // The first page's image is on screen: its title, in alpha red (the
    // sample's first page), is drawn in the top fifth of where the page
    // was placed.
    let (_, place) = browser.wall().shown().next().unwrap();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "positions and sizes on the panel")]
    let (left, top, width, height) = (place.x0 as u32, place.y0 as u32, place.width() as u32, place.height() as u32);
    let red = Rgb888::new(0xFF, 0, 0);
    let found = (top..(top + height / 5).min(HEIGHT))
        .flat_map(|y| (left..(left + width).min(WIDTH)).map(move |x| (y * WIDTH + x) as usize))
        .any(|at| screen.target.pixels[at] == red);
    assert!(found, "no page colours inside {place:?}");
}

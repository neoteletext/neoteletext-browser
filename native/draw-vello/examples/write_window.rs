//! Draws the browser offscreen, once in each shipped font, and writes
//! portable pixmap files for inspection:
//! `cargo run --release --example write_window -- <directory>`.
//! Prints the time each frame took to draw.

use std::io::Write;
use std::time::Instant;

use decoder::Engine;
use draw_vello::{Family, Painter};
use interface_core::browser::{Browser, Frame, StripState, TabInfo};
use interface_core::canvas::PageKey;
use interface_core::channel::{Channel, PageInfo, Status, page_index};

const WIDTH: u16 = 1440;
const HEIGHT: u16 = 900;
const CHANNEL: u32 = 3;
const TABS: [&str; 3] = ["teefax.t42", "level25", "sample.tti"];
/// Long after any transition.
const SETTLED: u64 = 10_000;
/// A third of the way through a zoom, in milliseconds.
const ZOOMING_FOR: u64 = 100;
/// A third of the way through a slide to the next page, in milliseconds.
const SLIDING_FOR: u64 = 73;
const TITLES: [&str; 6] = ["Headlines", "World news", "Sport results", "Weather", "Television", "Index"];

/// A channel of sample pages, decoded and drawn by the teletext engine.
fn browser(painter: &mut Painter) -> Result<Browser, String> {
    let pages: Vec<u16> =
        (0..8_u16).flat_map(|magazine| [0x100, 0x101, 0x104, 0x110, 0x123].map(|page| page + magazine * 0x100)).filter(|&page| page < 0x900).collect();
    let mut engine = Engine::new();
    let mut arrived = engine.feed(&decoder::sample::tti(&pages, &TITLES));
    arrived.extend(engine.finish());
    let mut channel = Channel::empty();
    channel.channel = CHANNEL;
    for (page, title) in arrived {
        let index = page_index(page).ok_or("a page outside 100 to 899")?;
        channel.pages[index] = Some(PageInfo { title, status: Status::Loaded });
        let rendered = engine.render(page).ok_or("a page that did not render")?;
        painter.set_page(PageKey { channel: CHANNEL, page }, &rendered.palette, &rendered.indices).map_err(|error| format!("{error:?}"))?;
    }
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
    let tabs = TABS
        .iter()
        .zip(0_u8..)
        .map(|(&host, colour)| TabInfo { title: host.into(), colour, channel: u32::from(colour), muted: false, private: false })
        .collect();
    browser.set_tabs(tabs, 2);
    browser.set_strip(StripState { address: "file:///samples/sample.tti".into(), identity: "sample.tti".into(), local: true, ..StripState::default() });
    browser.set_channel(channel);
    Ok(browser)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());
    let mut words = vec![0_u32; usize::from(WIDTH) * usize::from(HEIGHT)];
    // Zoom started this long before the frame: none, a third of the way
    // through the 300 millisecond zoom, and long finished.
    for (name, family, zoom) in [
        ("roboto", Family::Roboto, None),
        ("open-sans", Family::OpenSans, None),
        ("zooming", Family::Roboto, Some(ZOOMING_FOR)),
        ("zoomed", Family::Roboto, Some(SETTLED)),
        ("hover", Family::Roboto, None),
        ("sliding", Family::Roboto, Some(SETTLED)),
    ] {
        let mut painter = Painter::new(family);
        let mut browser = browser(&mut painter)?;
        if name == "hover" {
            // The pointer rests on the second page of the first frame.
            painter.begin(WIDTH, HEIGHT, 1.0);
            browser.draw(&mut painter, f64::from(WIDTH), f64::from(HEIGHT), 0);
            let (_, rect) = browser.wall().shown().nth(1).ok_or("no second page")?;
            browser.pointer_moved(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1), 0);
        }
        if let Some(before) = zoom {
            // The page's place on the wall is measured by a first frame.
            painter.begin(WIDTH, HEIGHT, 1.0);
            browser.draw(&mut painter, f64::from(WIDTH), f64::from(HEIGHT), SETTLED - before);
            browser.wall_mut().zoom(PageKey { channel: CHANNEL, page: 0x100 }, SETTLED - before);
            if name == "sliding" {
                // A step to the next page, a third of the way through its slide.
                browser.draw(&mut painter, f64::from(WIDTH), f64::from(HEIGHT), SETTLED - SLIDING_FOR);
                browser.wall_mut().step_zoom(1, SETTLED - SLIDING_FOR);
            }
        }
        // The first draw reports which pages are visible; time the second.
        painter.begin(WIDTH, HEIGHT, 1.0);
        browser.draw(&mut painter, f64::from(WIDTH), f64::from(HEIGHT), SETTLED);
        painter.finish(&mut words);
        let start = Instant::now();
        painter.begin(WIDTH, HEIGHT, 1.0);
        browser.draw(&mut painter, f64::from(WIDTH), f64::from(HEIGHT), SETTLED);
        painter.finish(&mut words);
        let elapsed = start.elapsed();
        let path = format!("{directory}/window-{name}.ppm");
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);
        write!(file, "P6\n{WIDTH} {HEIGHT}\n255\n")?;
        for word in &words {
            let [_, red, green, blue] = word.to_be_bytes();
            file.write_all(&[red, green, blue])?;
        }
        println!("{path}: drawn in {:.1} ms", elapsed.as_secs_f64() * 1000.0);
    }
    Ok(())
}

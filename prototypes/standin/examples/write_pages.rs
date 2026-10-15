//! Writes page images for inspection: `cargo run --example write_pages -- <directory>`.
use standin::page::render;
use standin::png::encode;

fn main() -> std::io::Result<()> {
    let directory = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());
    for (channel, page, flash) in [(1, 0x100, false), (1, 0x100, true), (42, 0x1FE, false), (7, 0x888, false)] {
        let bytes = encode(&render(channel, page, flash));
        let path = format!("{directory}/channel{channel}-page{page:03X}{}.png", if flash { "-flash" } else { "" });
        std::fs::write(&path, &bytes)?;
        println!("{path} {} bytes", bytes.len());
    }
    Ok(())
}

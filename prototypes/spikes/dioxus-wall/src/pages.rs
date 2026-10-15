//! Teletext pages from the stand-in engine, as Portable Network Graphics
//! images in `data:` addresses, which Blitz decodes itself.

/// How many pages the wall shows, as the prototype showed twelve frames
/// twice over.
pub const PAGE_COUNT: u16 = 24;
const FIRST_PAGE: u16 = 0x100;
/// Three stand-in channels, so neighbouring pages differ in colour.
const CHANNELS: u16 = 3;

#[derive(Clone, PartialEq)]
pub struct Page {
    pub channel: u32,
    pub number: u16,
    pub address: String,
    pub bytes: usize,
}

/// Base64 (RFC 4648 section 4), as `data:` addresses carry binary data.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const SIX_BITS: u32 = 0x3F;
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().enumerate().fold(0_u32, |group, (index, &byte)| group | u32::from(byte) << (16 - 8 * index));
        for position in 0..4 {
            if position <= chunk.len() {
                let index = (group >> (18 - 6 * position)) & SIX_BITS;
                out.push(char::from(ALPHABET[index as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn pages() -> Vec<Page> {
    (0..PAGE_COUNT)
        .map(|offset| {
            let channel = u32::from(offset % CHANNELS + 1);
            let number = FIRST_PAGE + offset;
            let png = standin::png::encode(&standin::page::render(channel, number, false));
            Page { channel, number, address: format!("data:image/png;base64,{}", base64(&png)), bytes: png.len() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_matches_the_standard_test_vectors() {
        // RFC 4648 section 10.
        for (input, expected) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }
}

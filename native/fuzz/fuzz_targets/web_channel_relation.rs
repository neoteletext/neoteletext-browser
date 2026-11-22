//! An address's relation to a channel, the unit of isolation of a
//! script's requests (neoteletext-common's DEC-7264; the script API's
//! specification section 11.4, target 9; `fetcher::web_fuzzing::
//! relation`, over `ChannelPolicy::relation`): in the channel, of its
//! origin outside its folder, or of another origin, for an arbitrary
//! channel identity (the file page's `null` among them) and an arbitrary
//! address. The opening asserts that what is in the channel lies under
//! its folder and is of its origin, and that a file page owns nothing.
#![no_main]

use libfuzzer_sys::fuzz_target;

/// What parts the channel from the address.
const SEPARATOR: u8 = 0;

fuzz_target!(|data: &[u8]| {
    let at = data.iter().position(|&byte| byte == SEPARATOR).unwrap_or(data.len());
    let (channel, url) = data.split_at(at);
    let (channel, url) = (String::from_utf8_lossy(channel), String::from_utf8_lossy(url.get(1..).unwrap_or_default()));
    let channel = if channel.is_empty() { "https://a.example/news/" } else { &channel };
    core::hint::black_box(fetcher::web_fuzzing::relation(channel, &url));
});

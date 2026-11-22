//! A script's WebSocket address as the web client turns it into the
//! address its handshake goes to and the request's head (the `WebSockets`
//! standard's `new WebSocket(url)` steps and RFC 6455 section 4.1;
//! TODO-7390 step 5 item 3; `fetcher::web_fuzzing::socket_address`):
//! the scheme changed, the fragment cleared, the port and mixed content
//! checked, the subprotocols' tokens and the head built. The opening
//! asserts the address and every line of the head; this target gives it
//! arbitrary addresses and subprotocols from each kind of channel.
#![no_main]

use libfuzzer_sys::fuzz_target;

/// The channels a page runs in: a web channel's folder over https and
/// over http, and the file page (`null`).
const CHANNELS: [&str; 3] = ["https://a.example/news/", "http://a.example/news/", "null"];
/// What parts the address from the subprotocols, and one subprotocol
/// from the next.
const SEPARATOR: u8 = 0;
/// The most subprotocols one input offers.
const MAX_PROTOCOLS: usize = 8;

fuzz_target!(|data: &[u8]| {
    let Some((&choice, rest)) = data.split_first() else { return };
    let channel = CHANNELS[usize::from(choice) % CHANNELS.len()];
    let mut fields = rest.split(|&byte| byte == SEPARATOR);
    let url = String::from_utf8_lossy(fields.next().unwrap_or_default());
    let protocols: Vec<String> = fields.take(MAX_PROTOCOLS).map(|field| String::from_utf8_lossy(field).into_owned()).collect();
    core::hint::black_box(fetcher::web_fuzzing::socket_address(channel, &url, &protocols));
});

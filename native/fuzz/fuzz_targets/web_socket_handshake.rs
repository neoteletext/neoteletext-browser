//! A server's answer to a script's WebSocket opening handshake as the
//! web client reads it (RFC 6455 section 4.2.2 and 4.1's client
//! requirements; `fetcher::web_fuzzing::handshake`, over `socket::
//! accepted`): the whole head of the answer, against the key the
//! client sent and the subprotocols it offered. Whatever the head,
//! nothing panics, and a subprotocol the client takes is one it offered
//! (asserted inside the opening).
#![no_main]

use libfuzzer_sys::fuzz_target;

/// The sample key of RFC 6455 section 1.3, whose answer is
/// `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`.
const KEY: &[u8] = b"dGhlIHNhbXBsZSBub25jZQ==";
/// Subprotocols a script may offer.
const PROTOCOLS: [&str; 3] = ["chat", "superchat", "v2.example"];

fuzz_target!(|data: &[u8]| {
    let Some((&offered, answer)) = data.split_first() else { return };
    let protocols: Vec<String> = PROTOCOLS.iter().take(usize::from(offered) % (PROTOCOLS.len() + 1)).map(|name| (*name).to_owned()).collect();
    core::hint::black_box(fetcher::web_fuzzing::handshake(answer, KEY, &protocols));
});

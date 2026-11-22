//! The address-space rules of a script's request (neoteletext-common's
//! DEC-7401, after the Local Network Access draft's `targetAddressSpace`;
//! `fetcher::web_fuzzing::address_space`): an arbitrary address with an
//! arbitrary declared space, the peer it connected to, the page's own
//! space and whether it is a secure context. The opening asserts each
//! answer against an independent statement of DEC-7401's rules: a
//! declared space is met only by a peer of it, an undeclared request is
//! always met, only a declared, literal or `.local` address is known
//! local, and a page of the internet never reaches a local or loopback
//! address unasked.
#![no_main]

use libfuzzer_sys::fuzz_target;

/// The input's fixed part: the declared space, the page's space, the
/// secure flag and the peer's sixteen bytes.
const FIXED: usize = 3 + 16;

fuzz_target!(|data: &[u8]| {
    let Some((fixed, url)) = data.split_first_chunk::<FIXED>() else { return };
    let declared = if fixed[0] & 0x80 != 0 { None } else { Some(fixed[0] % 5) };
    let page_space = fixed[1] % 5;
    let secure = fixed[2] & 1 != 0;
    let mut peer = [0_u8; 16];
    peer.copy_from_slice(&fixed[3..]);
    // An IPv4 peer as often as not, in its mapped form.
    if fixed[2] & 2 != 0 {
        peer[..12].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
    }
    let url = String::from_utf8_lossy(url);
    core::hint::black_box(fetcher::web_fuzzing::address_space(&url, declared, peer, page_space, secure));
});

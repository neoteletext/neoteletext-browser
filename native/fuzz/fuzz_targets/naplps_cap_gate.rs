//! The trusted core's gate on NAPLPS records (neoteletext-common's
//! `naplps-raster.md` RAS-9): any pushed bytes, cut into messages anywhere,
//! never panic it, and what it passes is the input's packets in their
//! order with some left out and any partial packet kept, every record it
//! counts dropped being at least one packet left out.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::naplps_cap::NaplpsGate;

/// One teletext packet's bytes.
const PACKET: usize = 42;

fuzz_target!(|data: &[u8]| {
    let Some((&cut, rest)) = data.split_first() else { return };
    let mut gate = NaplpsGate::default();
    // Whole messages of packets, as the fetcher passes them, and a last
    // piece that may end mid-packet.
    let step = (usize::from(cut) % 8 + 1) * PACKET;
    for message in rest.chunks(step) {
        let gated = gate.pass(message);
        let whole = message.len() / PACKET * PACKET;
        let (kept, tail) = gated.bytes.split_at(gated.bytes.len() - (message.len() - whole));
        assert_eq!(tail, &message[whole..], "a partial packet passes as it is");
        let mut input = message[..whole].as_chunks::<PACKET>().0.iter();
        for packet in kept.as_chunks::<PACKET>().0 {
            assert!(input.any(|candidate| candidate == packet), "a packet passed that was not given, or out of order");
        }
        let left_out = u64::try_from((whole - kept.len()) / PACKET).unwrap_or(u64::MAX);
        assert!(gated.dropped <= left_out, "a record counted that left nothing out");
    }
});

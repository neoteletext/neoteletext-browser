//! The cookie file once opened, which a program holding the key could have
//! written (requirement 21.5): whatever it holds, reading it keeps every
//! bound, keeps no cookie of an unnamed channel, and what was read writes a
//! file that reads back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::cookies::{PER_CHANNEL, Store, TOTAL};

/// 2026-10-22, a time every saved expiry is judged against.
const NOW: u64 = 1_792_656_000;

fuzz_target!(|data: &[u8]| {
    let mut store = Store::new();
    let _ = store.import(data, NOW);
    let listed = store.listed();
    assert!(listed.iter().all(|channel| channel.count <= PER_CHANNEL));
    assert!(listed.iter().map(|channel| channel.count).sum::<usize>() <= TOTAL);
    let written = store.export(NOW);
    let mut again = Store::new();
    let dropped = again.import(&written, NOW);
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(
        again.listed().iter().map(|channel| (&channel.key, channel.count)).collect::<Vec<_>>(),
        listed.iter().map(|channel| (&channel.key, channel.count)).collect::<Vec<_>>()
    );
});

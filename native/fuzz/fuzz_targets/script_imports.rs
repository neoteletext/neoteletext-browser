//! The script container's imports (script-api.md section 11.4, target 2;
//! the security review's finding S-09 item 5, TODO-7275): `nt.call`,
//! `nt.check`, `clock_time_get`, `fd_write` and the `fd_*` stubs, each
//! called with arbitrary pointers, lengths and counts over an arbitrary
//! memory image, as a module that had been taken over could call them.
//! Each returns or traps; none panics; none takes long, whatever the
//! memory names; and the host's state stays within its bounds.
#![no_main]

use std::time::{Duration, Instant};

use libfuzzer_sys::fuzz_target;
use neoteletext_script::host::Host;
use neoteletext_script::limits::{MAX_BATCH_BYTES, MAX_HOST_STATE_BYTES, MAX_LOG_LINES};
use neoteletext_script::machine::{IMPORTS, Probe};
use neoteletext_script::probe::PAGE_BYTES;
use neoteletext_script::{Claims, ContextId, ContextOpen, Renderer};

/// The most calls one input makes.
const MAX_CALLS: usize = 64;
/// The longest any one import may take: far above what bounded work
/// takes under the fuzzer's instruments, far below an unbounded copy.
const MAX_CALL_TIME: Duration = Duration::from_secs(2);
/// The limit each call is given.
const CALL_LIMIT: Duration = Duration::from_millis(500);
/// The bytes of one call in the input: an import, then ten arguments.
const CALL_BYTES: usize = 1 + 10 * 4;

fuzz_target!(|data: &[u8]| {
    // The first bytes say how the input is cut into calls and memory.
    let Some((&shape, rest)) = data.split_first() else { return };
    let calls = usize::from(shape % 16) + 1;
    let (script, image) = rest.split_at((calls * CALL_BYTES).min(rest.len()));
    let pages = if shape & 0x80 == 0 { 1 } else { 3 };
    let mut host = Host::new();
    host.open(
        ContextOpen {
            context: ContextId::Page,
            api: 1,
            renderer: Some(Renderer::Teletext),
            page: 0x100,
            subcode: None,
            claims: Claims::default(),
            language: String::new(),
            channel_url: "https://example.test/chan/".into(),
            page_url: String::new(),
            script_url: String::new(),
            channel_title: String::new(),
            seed: [1; 32],
            epoch_seconds: 1,
            zone_minutes: 0,
            sounds: vec![1],
            sound_allowed: true,
        },
        0,
    );
    host.begin_step(0, true);
    let Ok(mut probe) = Probe::new(image, pages, host) else { panic!("the forwarding module did not start") };
    let size = i64::try_from(pages * PAGE_BYTES).unwrap_or(0);
    for call in script.chunks(CALL_BYTES).take(MAX_CALLS) {
        let Some((&import, words)) = call.split_first() else { break };
        let arguments: Vec<i64> = words
            .chunks(4)
            .map(|word| {
                let mut bytes = [0_u8; 4];
                bytes[..word.len()].copy_from_slice(word);
                let value = i64::from(i32::from_le_bytes(bytes));
                // Half the time, somewhere inside the memory or just past its end.
                if bytes[3] & 1 == 0 { value.rem_euclid(size + 16) } else { value }
            })
            .collect();
        let began = Instant::now();
        let _ = probe.call(usize::from(import) % IMPORTS.len(), &arguments, CALL_LIMIT);
        assert!(began.elapsed() < MAX_CALL_TIME, "{:?} took {:?}", IMPORTS[usize::from(import) % IMPORTS.len()], began.elapsed());
        assert!(probe.host().state_bytes() <= MAX_HOST_STATE_BYTES && probe.host().batch_bytes() <= MAX_BATCH_BYTES);
    }
    assert!(probe.host().take_batch(1, Vec::new()).log.len() <= MAX_LOG_LINES);
});

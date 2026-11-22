//! The container's imports against a module that has been taken over
//! (the security review's finding S-09, TODO-7275; script-api.md section
//! 10.1 rules 1 and 5): each is called with arguments of an attacker's
//! choosing, over a memory of an attacker's making, through the
//! forwarding module of `neoteletext_script::probe`. Each must bound its
//! work by its output's bound before reading, trap on a pointer or length
//! outside the memory, and never panic.
mod common;

use std::time::Duration;

use common::timed;
use neoteletext_script::host::Host;
use neoteletext_script::limits::{MAX_HOST_BYTES, MAX_LOG_CHARS, MAX_LOG_LINES, MAX_WRITE_VECTORS};
use neoteletext_script::machine::{IMPORTS, Probe};
use neoteletext_script::probe::PAGE_BYTES;
use neoteletext_script::{ContextId, Stop};

const CALL: usize = 0;
const CHECK: usize = 1;
const CLOCK: usize = 2;
const WRITE: usize = 3;
const LIMIT: Duration = Duration::from_secs(1);
/// 16 MiB of memory.
const PAGES: usize = 256;

fn probe(pages: usize) -> Probe {
    let mut host = Host::new();
    host.open(common::opening(ContextId::Page), 0);
    host.begin_step(0, true);
    Probe::new(&[], pages, host).unwrap()
}

fn word(memory: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(memory[at..at + 4].try_into().unwrap())
}

#[test]
fn the_probe_names_the_imports_the_container_links() {
    assert_eq!(IMPORTS.map(|(_, name)| name), ["call", "check", "clock_time_get", "fd_write", "fd_close", "fd_seek", "fd_fdstat_get"]);
}

/// TODO-7275 item 1: the spike's `fd_write` copied every buffer it was
/// given before the log's bounds applied, so a memory full of descriptors
/// each naming all of the memory made the host copy without end. Here it
/// reads at most `MAX_WRITE_VECTORS` descriptors and `MAX_WRITE_BYTES`
/// bytes, whatever it is given.
#[test]
fn fd_write_reads_no_more_than_the_log_could_keep() {
    let mut probe = probe(PAGES);
    let size = PAGES * PAGE_BYTES;
    let descriptors = size / 8 - 1;
    {
        let memory = probe.memory();
        memory.fill(b'x');
        // Every descriptor names the whole memory from its start.
        for index in 0..descriptors {
            memory[index * 8..index * 8 + 4].copy_from_slice(&0_u32.to_le_bytes());
            memory[index * 8 + 4..index * 8 + 8].copy_from_slice(&u32::try_from(size).unwrap().to_le_bytes());
        }
    }
    let written = i64::try_from(size - 4).unwrap();
    let (result, took) = timed(|| probe.call(WRITE, &[1, 0, i64::try_from(descriptors).unwrap(), written], LIMIT));
    assert_eq!(result, Ok(Some(0)));
    assert!(took.running < Duration::from_millis(50), "fd_write took {took:?}");
    // The module is told all of every buffer looked at was taken.
    let told = u32::from_le_bytes(probe.memory()[size - 4..].try_into().unwrap());
    assert_eq!(u64::from(told), (MAX_WRITE_VECTORS * size) as u64);
    let batch = probe.host().take_batch(1, Vec::new());
    assert!(batch.log.len() <= MAX_LOG_LINES && batch.log.iter().all(|line| line.text.chars().count() <= MAX_LOG_CHARS), "{} lines", batch.log.len());
    // Any other descriptor is refused; a negative count, a table or a
    // buffer outside the memory, and a count that cannot be written back
    // are traps.
    assert_eq!(probe.call(WRITE, &[3, 0, 1, written], LIMIT), Ok(Some(8)));
    for bad in [[1, 0, -1, written], [1, i64::try_from(size).unwrap() - 4, 1, written], [1, 0, 1, i64::try_from(size).unwrap() - 2]] {
        assert!(matches!(probe.call(WRITE, &bad, LIMIT), Err(Stop::Trap(_))), "{bad:?}");
    }
    probe.memory()[..8].copy_from_slice(&[0xF0, 0xFF, 0xFF, 0xFF, 0x40, 0, 0, 0]);
    assert!(matches!(probe.call(WRITE, &[1, 0, 1, written], LIMIT), Err(Stop::Trap(_))), "a buffer outside the memory");
}

/// Section 10.1 rules 1 and 5 for the host call: every pointer, count and
/// length outside what the shim would pass is a trap, and a payload past
/// `MAX_HOST_BYTES` is refused before a byte of it is read.
#[test]
fn the_host_call_bounds_before_it_reads_and_traps_outside_the_memory() {
    let mut probe = probe(8);
    let size = i64::try_from(8 * PAGE_BYTES).unwrap();
    // The arguments: context, op, ints, n_ints, payload, n_payload,
    // payload_kind, result, out, out_capacity. Operation 2 is NOW.
    let good = [2, 2, 0, 0, 0, 0, 0, 1024, 2048, 1024];
    assert_eq!(probe.call(CALL, &good, LIMIT), Ok(None));
    // Status 0 (it worked), kind 1 (a whole number), value 0, length 0.
    assert_eq!([word(probe.memory(), 1024), word(probe.memory(), 1028), word(probe.memory(), 1032), word(probe.memory(), 1036)], [0, 1, 0, 0]);
    let change = |index: usize, value: i64| {
        let mut arguments = good;
        arguments[index] = value;
        arguments
    };
    for (what, arguments) in [
        ("whole numbers outside the memory", [2, 2, size - 4, 2, 0, 0, 0, 1024, 2048, 1024]),
        ("a negative count of whole numbers", change(3, -1)),
        ("more whole numbers than the bound", change(3, 24)),
        ("a negative payload length", change(5, -1)),
        ("a payload of no known kind", change(6, 9)),
        ("result words outside the memory", change(7, size - 8)),
        ("result words at a negative address", change(7, -4)),
    ] {
        assert!(matches!(probe.call(CALL, &arguments, LIMIT), Err(Stop::Trap(_))), "{what}");
    }
    assert!(matches!(probe.call(CALL, &[2, 1, 512, 1, size - 2, 16, 1, 1024, 2048, 1024], LIMIT), Err(Stop::Trap(_))), "a payload outside the memory");
    // LOG of level 1 with a payload longer than the bound: a RangeError
    // (status 1) written back, the payload never read (it lies outside
    // the memory, and that is not a trap because it is not looked at).
    probe.memory()[512..516].copy_from_slice(&1_i32.to_le_bytes());
    let long = i64::try_from(MAX_HOST_BYTES + 1).unwrap();
    assert_eq!(probe.call(CALL, &[2, 1, 512, 1, size - 2, long, 1, 1024, 2048, 1024], LIMIT), Ok(None));
    assert_eq!(word(probe.memory(), 1024), 1);
    let length = usize::try_from(word(probe.memory(), 1036)).unwrap();
    assert!(String::from_utf8_lossy(&probe.memory()[2048..2048 + length]).starts_with("MAX_HOST_BYTES: "));
    // An error's text that does not fit the buffer it was given is cut to
    // the buffer, not written past it.
    probe.memory()[4096..4104].fill(0xAA);
    assert_eq!(probe.call(CALL, &[2, 9999, 0, 0, 0, 0, 0, 1024, 4096, 4], LIMIT), Ok(None));
    assert_eq!((word(probe.memory(), 1024), word(probe.memory(), 1036)), (2, 4));
    assert_eq!(&probe.memory()[4100..4104], [0xAA; 4]);
    // A result buffer outside the memory is a trap.
    assert!(matches!(probe.call(CALL, &[2, 9999, 0, 0, 0, 0, 0, 1024, size - 1, 64], LIMIT), Err(Stop::Trap(_))));
}

/// Section 3.2 rule 6: every import reads the clock, so a module that
/// does nothing but call the host is stopped at its deadline; the clock
/// it is given moves only with the tick; no file exists.
#[test]
fn every_import_reads_the_clock_and_the_clock_given_is_the_ticks() {
    let mut probe = probe(1);
    assert_eq!(probe.call(CHECK, &[], LIMIT), Ok(None));
    for (import, name) in IMPORTS.iter().enumerate() {
        std::thread::sleep(Duration::from_millis(1));
        assert_eq!(probe.call(import, &[2, 2, 0, 0, 0, 0, 0, 1024, 2048, 1024], Duration::ZERO), Err(Stop::Time), "{name:?} did not read the clock");
    }
    // The time of day: the context's start, 1,800,000,000 s, in nanoseconds.
    assert_eq!(probe.call(CLOCK, &[0, 0, 64], LIMIT), Ok(Some(0)));
    assert_eq!(u64::from_le_bytes(probe.memory()[64..72].try_into().unwrap()), 1_800_000_000_000_000_000);
    probe.host().begin_step(60, true);
    assert_eq!(probe.call(CLOCK, &[0, 0, 64], LIMIT), Ok(Some(0)));
    assert_eq!(u64::from_le_bytes(probe.memory()[64..72].try_into().unwrap()), 1_800_000_001_000_000_000);
    assert_eq!(probe.call(CLOCK, &[1, 0, 64], LIMIT), Ok(Some(0)));
    assert_eq!(u64::from_le_bytes(probe.memory()[64..72].try_into().unwrap()), 1_000_000_000);
    assert!(matches!(probe.call(CLOCK, &[0, 0, i64::try_from(PAGE_BYTES).unwrap() - 4], LIMIT), Err(Stop::Trap(_))));
    // fd_close, fd_seek and fd_fdstat_get: a bad descriptor (8), always.
    for import in 4..7 {
        assert_eq!(probe.call(import, &[1, 0, 0, 0], LIMIT), Ok(Some(8)));
    }
}

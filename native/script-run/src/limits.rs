//! Every bound of the runner, each defined once. A bound that is the
//! script library's, the protocol's or the engine's is used by its name
//! there and not restated.

use neoteletext_script::limits::TICKS_PER_SECOND;

/// The mixer's rate: the sound helper's, 48 kHz.
pub const SAMPLE_RATE_HZ: u32 = 48_000;
/// The samples of one tick. The rate divides evenly by the clock's, so
/// every tick is the same number of samples and a run's sound is as long
/// as its ticks.
pub const SAMPLES_PER_TICK: usize = (SAMPLE_RATE_HZ as u64 / TICKS_PER_SECOND) as usize;
const _: () = assert!((SAMPLE_RATE_HZ as u64).is_multiple_of(TICKS_PER_SECOND));
/// One tick, in microseconds: what the decoder's clock is advanced by.
pub const MICROSECONDS_PER_TICK: u64 = 1_000_000 / TICKS_PER_SECOND;

/// The ticks a run steps when `--ticks` is not given: one second.
pub const DEFAULT_TICKS: u64 = TICKS_PER_SECOND;
/// The most ticks of one run: ten minutes of the page's clock, which
/// bounds the sound file and the time a gate can be made to spend.
pub const MAX_TICKS: u64 = 10 * 60 * TICKS_PER_SECOND;
/// The most snapshots of one run.
pub const MAX_SNAPSHOTS: usize = 600;

/// A WAV file's header: the RIFF, format and data chunk headers of 16-bit
/// PCM (44 bytes).
pub const WAV_HEADER_BYTES: usize = 44;
/// The bytes of one sample: 16-bit mono.
pub const BYTES_PER_SAMPLE: usize = 2;

/// The most bytes of one file passed between the two processes, either
/// way.
pub const MAX_BLOB_BYTES: usize = 64 * 1024 * 1024;
// The longest run's sound is one such file.
const _: () = assert!(MAX_TICKS * (SAMPLES_PER_TICK * BYTES_PER_SAMPLE) as u64 + WAV_HEADER_BYTES as u64 <= MAX_BLOB_BYTES as u64);
/// The most files passed in one direction: every snapshot and the few
/// fixed files beside them.
pub const MAX_BLOBS: usize = MAX_SNAPSHOTS + 16;
/// The most bytes of all the files passed in one direction.
pub const MAX_BLOBS_BYTES: usize = 256 * 1024 * 1024;
/// The most bytes of a passed file's name.
pub const MAX_BLOB_NAME_BYTES: usize = 64;

/// The most bytes of a command-line argument passed to the worker, and
/// the most arguments.
pub const MAX_ARGUMENT_BYTES: usize = 4096;
/// See [`MAX_ARGUMENT_BYTES`].
pub const MAX_ARGUMENTS: usize = 64;

/// The most bytes of a script's manifest, from the file's start to the
/// comment's end (the specification's `MAX_MANIFEST_BYTES`, section 10.6).
pub const MAX_MANIFEST_BYTES: usize = 4096;

/// The most bytes of an input script, of one of its lines, and the most
/// events it holds.
pub const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;
/// See [`MAX_INPUT_BYTES`].
pub const MAX_INPUT_LINE_BYTES: usize = 8192;
/// See [`MAX_INPUT_BYTES`].
pub const MAX_INPUT_EVENTS: usize = 100_000;

/// The most bytes of a network file (a mock table or a recording), of
/// one of its lines, of its entries, and of the decoded bytes all its
/// entries hold together.
pub const MAX_NETWORK_FILE_BYTES: usize = 32 * 1024 * 1024;
/// See [`MAX_NETWORK_FILE_BYTES`].
pub const MAX_NETWORK_LINE_BYTES: usize = 2 * 1024 * 1024;
/// See [`MAX_NETWORK_FILE_BYTES`].
pub const MAX_NETWORK_ENTRIES: usize = 4096;
/// The most steps (lines) of one entry of a network file.
pub const MAX_NETWORK_STEPS: usize = 4096;

/// The most bytes of a channel file: the engine's bound for
/// `channel.ttx` once it holds scripts (section 12.1), 64 KiB.
pub const MAX_CHANNEL_FILE_BYTES: usize = 64 * 1024;
/// The most bytes of a page file read from a channel's folder: the
/// largest T42 file the snapshot reader takes is far smaller; a TTI file
/// of every sub-page is text of this order.
pub const MAX_PAGE_FILE_BYTES: usize = 4 * 1024 * 1024;

/// The most bytes of `expect.json`, and the most files it names.
pub const MAX_EXPECT_BYTES: usize = 256 * 1024;
/// See [`MAX_EXPECT_BYTES`].
pub const MAX_EXPECT_FILES: usize = MAX_BLOBS;

/// The most bytes of the log a run keeps: lines past it are counted and
/// dropped, so that a script that logs every tick cannot fill memory.
pub const MAX_RUN_LOG_BYTES: usize = 8 * 1024 * 1024;

/// The fixed moment a run's scripts are told it is, in seconds since
/// 1970: 1 January 2027, 00:00:00 UTC. A run never reads the real clock,
/// so that `Date` gives the same answer on every machine (departure D15).
pub const FIXED_EPOCH_SECONDS: u64 = 1_798_761_600;
/// The text the seed is made from when `--seed` is not given.
pub const DEFAULT_SEED: &str = "neoteletext-script-run";
/// The channel address a run's scripts are told when `--channel-url` is
/// not given: a name under `.test`, which RFC 6761 keeps for testing, so
/// that a run does not depend on where its folder is.
pub const DEFAULT_CHANNEL_URL: &str = "https://channel.test/";
/// The page a run opens when neither `--page` nor a channel file says.
pub const DEFAULT_PAGE: u16 = 0x100;

/// How often `--watch` looks at the files' modification times.
pub const WATCH_INTERVAL: core::time::Duration = core::time::Duration::from_millis(250);

/// The version of the kept results' form, in `expect.json`: raised by
/// any change to what the runner writes for the same run (the files'
/// names, the log's or the budget's form).
pub const EXPECT_FORM: u32 = 1;

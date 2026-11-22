//! The command line (script-api.md section 16).
//!
//! ```text
//! neoteletext-script-run run CHANNEL_OR_FILE [--page NNN[/SSSS]]
//!     [--script FILE.js] [--channel-script FILE.js]
//!     [--ticks N] [--pace] [--seed TEXT] [--input FILE]
//!     [--network none|live] [--network mock|replay|record FILE] [--serve DIR]
//!     [--out DIR] [--png-every N] [--png-at TICK,...] [--wav FILE]
//!     [--expect DIR] [--bless] [--report FILE]
//!     [--channel-url URL] [--sound-allowed] [--strict-time] [--watch] [--unconfined]
//! ```
//!
//! `CHANNEL_OR_FILE` is a channel's folder, a page file (`.tti`, `.t42`)
//! or one script (`.js`).

use std::path::PathBuf;

use crate::Failure;
use crate::limits::{DEFAULT_CHANNEL_URL, DEFAULT_SEED, DEFAULT_TICKS, MAX_ARGUMENT_BYTES, MAX_ARGUMENTS, MAX_SNAPSHOTS, MAX_TICKS};

/// What the command line is, for a message that refuses one.
pub const USAGE: &str = "usage: neoteletext-script-run run CHANNEL_OR_FILE [--page NNN[/SSSS]] [--script FILE.js] [--channel-script FILE.js] \
[--ticks N] [--pace] [--seed TEXT] [--input FILE] [--network none|live | --network mock|replay|record FILE] [--serve DIR] \
[--out DIR] [--png-every N] [--png-at TICK,...] [--wav FILE] [--expect DIR] [--bless] [--report FILE] \
[--channel-url URL] [--sound-allowed] [--strict-time] [--watch] [--unconfined]";

/// Where a run's network answers come from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NetworkMode {
    /// Every request fails.
    #[default]
    None,
    /// A table of answers.
    Mock(PathBuf),
    /// A recording, each answer given once.
    Replay(PathBuf),
    /// The real web client, with what it answered written down.
    Record(PathBuf),
    /// The real web client.
    Live,
}

impl NetworkMode {
    /// Whether the mode reaches the real network, and so cannot run
    /// confined as a decoder is.
    #[must_use]
    pub const fn is_live(&self) -> bool {
        matches!(self, Self::Live | Self::Record(_))
    }
}

/// A run, as its command line asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "the command line's switches, each one of its own")]
pub struct Options {
    /// The channel's folder, a page file or a script.
    pub target: PathBuf,
    /// The page, as three hexadecimal digits, and its sub-code.
    pub page: Option<(u16, Option<u16>)>,
    /// The page-level script, where the channel file does not name it.
    pub script: Option<PathBuf>,
    /// The channel-level script, where the channel file does not name it.
    pub channel_script: Option<PathBuf>,
    /// The ticks to step.
    pub ticks: u64,
    /// Step on the wall clock, for watching.
    pub pace: bool,
    /// What the seed is made from.
    pub seed: String,
    /// The input script.
    pub input: Option<PathBuf>,
    /// The network.
    pub network: NetworkMode,
    /// The folder a local server answers from, for the live network.
    pub serve: Option<PathBuf>,
    /// Where the snapshots are written.
    pub out: Option<PathBuf>,
    /// A snapshot every this many ticks.
    pub png_every: Option<u64>,
    /// A snapshot after each of these ticks.
    pub png_at: Vec<u64>,
    /// Where the sound is written.
    pub wav: Option<PathBuf>,
    /// The kept results to compare with.
    pub expect: Option<PathBuf>,
    /// Write the kept results instead of comparing.
    pub bless: bool,
    /// Where the report is written as JSON.
    pub report: Option<PathBuf>,
    /// The address the scripts are told their channel has.
    pub channel_url: String,
    /// Sound is allowed from the start, as if the viewer had pressed a key.
    pub sound_allowed: bool,
    /// Hold the scripts to the browser's time limits on the wall clock.
    pub strict_time: bool,
    /// Run again whenever a file read for the run changes.
    pub watch: bool,
    /// Run in this process with no confinement: for a script the person
    /// running it trusts, where the system refuses the sandbox.
    pub unconfined: bool,
}

impl Options {
    /// The ticks after whose step a snapshot is taken, rising, no tick
    /// twice: those `--png-every` and `--png-at` name, or the last tick
    /// alone when neither is given.
    #[must_use]
    pub fn snapshot_ticks(&self) -> Vec<u64> {
        let last = self.ticks.saturating_sub(1);
        let mut ticks: Vec<u64> = self.png_at.iter().copied().filter(|tick| *tick <= last).collect();
        if let Some(every) = self.png_every.filter(|every| *every > 0) {
            let mut tick = every.saturating_sub(1);
            while tick <= last {
                ticks.push(tick);
                tick = tick.saturating_add(every);
            }
        }
        if self.png_every.is_none() && self.png_at.is_empty() {
            ticks.push(last);
        }
        ticks.sort_unstable();
        ticks.dedup();
        ticks
    }
}

fn usage(why: &str) -> Failure {
    Failure::usage(format!("{why}\n{USAGE}"))
}

/// A page as `NNN` or `NNN/SSSS`, in hexadecimal.
fn page(text: &str) -> Option<(u16, Option<u16>)> {
    let (number, subcode) = text.split_once('/').map_or((text, None), |(number, subcode)| (number, Some(subcode)));
    let hexadecimal = |digits: &str, length: usize| {
        (digits.len() == length && digits.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| u16::from_str_radix(digits, 16).ok()).flatten()
    };
    let number = hexadecimal(number, 3).filter(|number| neoteletext_core::types::PageAddress::from_mpp(*number).is_some())?;
    match subcode {
        None => Some((number, None)),
        Some(subcode) => Some((number, Some(hexadecimal(subcode, 4)?))),
    }
}

/// Takes the option `flag` with its `value`, and for `--network` the
/// file after it from `rest`.
fn valued(options: &mut Options, flag: &str, value: &String, rest: &mut core::slice::Iter<'_, String>) -> Result<(), Failure> {
    let number = |most: u64| value.parse::<u64>().ok().filter(|number| (1..=most).contains(number));
    match flag {
        "--page" => options.page = Some(page(value).ok_or_else(|| usage("--page takes NNN or NNN/SSSS in hexadecimal, a page from 100 to 8FF"))?),
        "--script" => options.script = Some(value.into()),
        "--channel-script" => options.channel_script = Some(value.into()),
        "--ticks" => options.ticks = number(MAX_TICKS).ok_or_else(|| usage(&format!("--ticks takes a whole number from 1 to {MAX_TICKS}")))?,
        "--seed" => options.seed.clone_from(value),
        "--input" => options.input = Some(value.into()),
        "--network" => {
            options.network = match value.as_str() {
                "none" => NetworkMode::None,
                "live" => NetworkMode::Live,
                mode @ ("mock" | "replay" | "record") => {
                    let file: PathBuf = rest.next().ok_or_else(|| usage(&format!("--network {mode} takes a file")))?.into();
                    match mode {
                        "mock" => NetworkMode::Mock(file),
                        "replay" => NetworkMode::Replay(file),
                        _ => NetworkMode::Record(file),
                    }
                }
                _ => return Err(usage("--network takes none, live, mock FILE, replay FILE or record FILE")),
            };
        }
        "--serve" => options.serve = Some(value.into()),
        "--out" => options.out = Some(value.into()),
        "--png-every" => options.png_every = Some(number(MAX_TICKS).ok_or_else(|| usage("--png-every takes a whole number of ticks, at least 1"))?),
        "--png-at" => {
            for tick in value.split(',') {
                options.png_at.push(
                    tick.parse::<u64>()
                        .ok()
                        .filter(|tick| *tick < MAX_TICKS)
                        .ok_or_else(|| usage("--png-at takes ticks, counted from 0, with commas between"))?,
                );
            }
        }
        "--wav" => options.wav = Some(value.into()),
        "--expect" => options.expect = Some(value.into()),
        "--report" => options.report = Some(value.into()),
        "--channel-url" => {
            let plain = value.bytes().all(|byte| byte.is_ascii_graphic());
            if !plain || !(value.starts_with("https://") || value.starts_with("http://")) || !value.ends_with('/') {
                return Err(usage("--channel-url takes an http: or https: address that ends in `/`, a channel's folder"));
            }
            options.channel_url.clone_from(value);
        }
        _ => return Err(usage(&format!("`{flag}` is not an option"))),
    }
    Ok(())
}

/// Reads the arguments after the program's name.
///
/// # Errors
/// A [`Failure`] of status 4 saying what was wrong, with the usage.
pub fn parse(arguments: &[String]) -> Result<Options, Failure> {
    if arguments.len() > MAX_ARGUMENTS || arguments.iter().any(|argument| argument.len() > MAX_ARGUMENT_BYTES || argument.contains('\0')) {
        return Err(usage("too many arguments, or one too long"));
    }
    let [command, target, rest @ ..] = arguments else { return Err(usage("no command or nothing to run")) };
    if command != "run" {
        return Err(usage(&format!("`{command}` is not a command")));
    }
    let mut options = Options {
        target: target.into(),
        page: None,
        script: None,
        channel_script: None,
        ticks: DEFAULT_TICKS,
        pace: false,
        seed: DEFAULT_SEED.to_owned(),
        input: None,
        network: NetworkMode::None,
        serve: None,
        out: None,
        png_every: None,
        png_at: Vec::new(),
        wav: None,
        expect: None,
        bless: false,
        report: None,
        channel_url: DEFAULT_CHANNEL_URL.to_owned(),
        sound_allowed: false,
        strict_time: false,
        watch: false,
        unconfined: false,
    };
    let mut rest = rest.iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--pace" => options.pace = true,
            "--bless" => options.bless = true,
            "--watch" => options.watch = true,
            "--sound-allowed" => options.sound_allowed = true,
            "--strict-time" => options.strict_time = true,
            "--unconfined" => options.unconfined = true,
            _ => {
                let value = rest.next().ok_or_else(|| usage(&format!("`{flag}` needs a value, or is not an option")))?;
                valued(&mut options, flag, value, &mut rest)?;
            }
        }
    }
    if options.bless && options.expect.is_none() {
        return Err(usage("--bless writes the kept results --expect names, and needs it"));
    }
    if options.serve.is_some() && !options.network.is_live() {
        return Err(usage("--serve starts a local server for --network live or --network record"));
    }
    if options.snapshot_ticks().len() > MAX_SNAPSHOTS {
        return Err(usage(&format!("MAX_SNAPSHOTS: the run would take more than {MAX_SNAPSHOTS} snapshots")));
    }
    if options.pace && options.expect.is_some() {
        return Err(usage("--pace is for watching, and a paced run is not compared with kept results"));
    }
    Ok(options)
}

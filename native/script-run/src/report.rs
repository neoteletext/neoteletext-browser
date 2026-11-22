//! What a run reports (script-api.md section 16): the budget at the end
//! and its maxima, the time of each step (median, ninth decile, worst),
//! the bytes of output for the page's renderer, the requests made and the
//! events dropped, as text and as JSON.
//!
//! Times are measured on the wall clock and differ from run to run, so
//! they are in the report and never in what is kept and compared. The
//! budget is in both: the report has all of it, and the kept
//! `budget.json` leaves out the entries that are shares of a time limit
//! ([`TIMED_ENTRIES`]) and those that measure the library's own footprint
//! ([`FOOTPRINT_ENTRIES`]).

use core::fmt::Write as _;
use core::time::Duration;

use crate::Status;
use crate::json::{self, Value, quote};
use crate::network::Counts;

/// The budget's entries that are shares of a time limit (section 3.7):
/// they follow the wall clock, so they are reported and not kept.
pub const TIMED_ENTRIES: [&str; 4] = ["callbackPercent", "callbackPercentMax", "stepPercentMax", "secondPercent"];
/// The budget's entries that measure the script library's own footprint
/// with the page's: the engine's heap and the host's bytes hold the
/// prelude, which grows with every API added to it. They are reported
/// and not kept, so that a kept run changes when its page's behaviour
/// does and not whenever the library gains an operation.
pub const FOOTPRINT_ENTRIES: [&str; 2] = ["heapBytes", "hostBytes"];
/// The most values a budget report holds: its entries, each an object of
/// two numbers, for the shared core, a renderer and the components.
const MAX_BUDGET_VALUES: usize = 512;

/// One entry of the budget, with the most it was at any step of the run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetEntry {
    /// Its name, the bound's in lower camel case.
    pub name: String,
    /// What is used at the run's end.
    pub used: i64,
    /// The bound.
    pub limit: i64,
    /// The most that was used at the end of any step.
    pub most: i64,
}

/// A context's budget through a run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    /// The entries, in the order `nt.budget()` gives them.
    pub entries: Vec<BudgetEntry>,
}

impl Budget {
    /// Takes in the budget report of one moment: the JSON text
    /// `nt.budget()` is given. Text that is not such a report changes
    /// nothing.
    pub fn observe(&mut self, text: &str) {
        let Ok(document) = json::parse(text.as_bytes(), MAX_BUDGET_VALUES) else { return };
        let Some(members) = document.members() else { return };
        for (name, entry) in members {
            let (Some(used), Some(limit)) = (entry.member("used").and_then(Value::number), entry.member("limit").and_then(Value::number)) else { continue };
            match self.entries.iter_mut().find(|held| held.name == *name) {
                Some(held) => {
                    held.used = used;
                    held.limit = limit;
                    held.most = held.most.max(used);
                }
                None => self.entries.push(BudgetEntry { name: name.clone(), used, limit, most: used }),
            }
        }
    }

    /// The entries as a JSON object, with those of [`TIMED_ENTRIES`] or
    /// without.
    fn json(&self, timed: bool) -> String {
        let mut out = String::from("{");
        let mut first = true;
        for entry in
            self.entries.iter().filter(|entry| timed || !(TIMED_ENTRIES.contains(&entry.name.as_str()) || FOOTPRINT_ENTRIES.contains(&entry.name.as_str())))
        {
            if !first {
                out.push(',');
            }
            first = false;
            let _ = write!(out, "\n    {}: {{\"used\": {}, \"limit\": {}, \"most\": {}}}", quote(&entry.name), entry.used, entry.limit, entry.most);
        }
        out.push_str("\n  }");
        out
    }
}

/// The kept budget file: each context's budget without its timed
/// entries.
#[must_use]
pub fn budget_file(budgets: &[(&'static str, Budget)]) -> Vec<u8> {
    let mut out = String::from("{");
    for (index, (context, budget)) in budgets.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "\n  {}: {}", quote(context), budget.json(false));
    }
    out.push_str("\n}\n");
    out.into_bytes()
}

/// The times of a run's steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Times {
    /// How many steps ran.
    pub steps: u64,
    /// The median, in microseconds.
    pub median: u64,
    /// The ninth decile.
    pub ninth_decile: u64,
    /// The worst.
    pub worst: u64,
}

impl Times {
    /// The figures of `times`, one for each step.
    #[must_use]
    pub fn of(times: &mut [Duration]) -> Self {
        times.sort_unstable();
        let micros = |duration: Duration| u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        // The value at a tenth of the way through the sorted steps.
        let at = |tenths: usize| times.get(times.len().saturating_sub(1).saturating_mul(tenths).checked_div(10).unwrap_or(0)).copied().map_or(0, micros);
        Self { steps: u64::try_from(times.len()).unwrap_or(u64::MAX), median: at(5), ninth_decile: at(9), worst: at(10) }
    }
}

/// A run's report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// How the run ended, before any comparison with kept results.
    pub status: Status,
    /// Why, when it did not simply run.
    pub message: String,
    /// What the seed was made from.
    pub seed: String,
    /// The ticks stepped.
    pub ticks: u64,
    /// The renderer of the page.
    pub renderer: &'static str,
    /// The time of each step.
    pub times: Times,
    /// The time to compile the engine's module, make the container and
    /// open the contexts, in microseconds.
    pub start_micros: u64,
    /// The bytes of packets and records the batches became for the
    /// page's renderer.
    pub output_bytes: u64,
    /// The sections of batches that were not applied to the page.
    pub left_out: Vec<&'static str>,
    /// The network's counts.
    pub network: Counts,
    /// Whether the network mode gives the same answers every run.
    pub deterministic: bool,
    /// The input events given to the scripts.
    pub events: u64,
    /// The input events and reports that did not fit their step and were
    /// dropped.
    pub events_dropped: u64,
    /// The log's lines, and those dropped past its bound.
    pub log_lines: u64,
    /// See `log_lines`.
    pub log_dropped: u64,
    /// The sound operations given to the mixer, and the loudest sample.
    pub sound_operations: u64,
    /// See `sound_operations`.
    pub sound_peak: u16,
    /// The container's linear memory and the engine's heap in it, in
    /// bytes, at the end.
    pub memory: (usize, usize),
    /// Each context's budget.
    pub budgets: Vec<(&'static str, Budget)>,
    /// Each kept file's name and SHA-256.
    pub files: Vec<(String, String)>,
}

impl Report {
    /// The report as text, for a person.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "status {}: {}", self.status.code(), if self.message.is_empty() { "ran" } else { &self.message });
        let _ = writeln!(out, "renderer {}, {} ticks, seed \"{}\"", self.renderer, self.ticks, self.seed);
        let _ = writeln!(
            out,
            "steps: {} run, median {} us, ninth decile {} us, worst {} us; start {} us",
            self.times.steps, self.times.median, self.times.ninth_decile, self.times.worst, self.start_micros
        );
        let _ = writeln!(out, "output: {} bytes of {} packets and records", self.output_bytes, self.renderer);
        if !self.left_out.is_empty() {
            let _ = writeln!(out, "NOT DRAWN: the batches held sections this runner does not yet apply to the page: {}", self.left_out.join(", "));
        }
        let _ = writeln!(
            out,
            "network: {} requests, {} unanswered, {} answers{}",
            self.network.requests,
            self.network.unanswered,
            self.network.answers,
            if self.deterministic { "" } else { " (live: not the same every run)" }
        );
        let _ = writeln!(out, "events: {} given, {} dropped; log: {} lines, {} dropped", self.events, self.events_dropped, self.log_lines, self.log_dropped);
        let _ = writeln!(out, "sound: {} operations, peak {}", self.sound_operations, self.sound_peak);
        let _ = writeln!(out, "memory: {} bytes of linear memory, {} of heap", self.memory.0, self.memory.1);
        for (context, budget) in &self.budgets {
            let _ = writeln!(out, "budget of the {context} context (used / limit, most):");
            for entry in &budget.entries {
                let _ = writeln!(out, "  {:<22} {:>10} / {:<10} {:>10}", entry.name, entry.used, entry.limit, entry.most);
            }
        }
        out
    }

    /// The report as JSON.
    #[must_use]
    pub fn json(&self) -> String {
        let mut out = String::from("{\n");
        let _ = writeln!(out, "  \"status\": {},", self.status.code());
        let _ = writeln!(out, "  \"message\": {},", quote(&self.message));
        let _ = writeln!(out, "  \"seed\": {},", quote(&self.seed));
        let _ = writeln!(out, "  \"ticks\": {},", self.ticks);
        let _ = writeln!(out, "  \"renderer\": {},", quote(self.renderer));
        let _ = writeln!(
            out,
            "  \"stepMicroseconds\": {{\"steps\": {}, \"median\": {}, \"ninthDecile\": {}, \"worst\": {}}},",
            self.times.steps, self.times.median, self.times.ninth_decile, self.times.worst
        );
        let _ = writeln!(out, "  \"startMicroseconds\": {},", self.start_micros);
        let _ = writeln!(out, "  \"outputBytes\": {{{}: {}}},", quote(self.renderer), self.output_bytes);
        let left_out: Vec<String> = self.left_out.iter().map(|name| quote(name)).collect();
        let _ = writeln!(out, "  \"sectionsNotApplied\": [{}],", left_out.join(", "));
        let _ = writeln!(
            out,
            "  \"network\": {{\"requests\": {}, \"unanswered\": {}, \"answers\": {}, \"deterministic\": {}}},",
            self.network.requests, self.network.unanswered, self.network.answers, self.deterministic
        );
        let _ = writeln!(out, "  \"events\": {{\"given\": {}, \"dropped\": {}}},", self.events, self.events_dropped);
        let _ = writeln!(out, "  \"log\": {{\"lines\": {}, \"dropped\": {}}},", self.log_lines, self.log_dropped);
        let _ = writeln!(out, "  \"sound\": {{\"operations\": {}, \"peak\": {}}},", self.sound_operations, self.sound_peak);
        let _ = writeln!(out, "  \"memory\": {{\"linearBytes\": {}, \"heapBytes\": {}}},", self.memory.0, self.memory.1);
        out.push_str("  \"budget\": {");
        for (index, (context, budget)) in self.budgets.iter().enumerate() {
            let _ = write!(out, "{}\n  {}: {}", if index > 0 { "," } else { "" }, quote(context), budget.json(true));
        }
        out.push_str("\n  },\n  \"files\": {");
        for (index, (name, digest)) in self.files.iter().enumerate() {
            let _ = write!(out, "{}\n    {}: {}", if index > 0 { "," } else { "" }, quote(name), quote(digest));
        }
        out.push_str("\n  }\n}\n");
        out
    }
}

/// The SHA-256 of `bytes`, in hexadecimal.
#[must_use]
pub fn sha256(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    let mut out = String::with_capacity(digest.as_ref().len().saturating_mul(2));
    for byte in digest.as_ref() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

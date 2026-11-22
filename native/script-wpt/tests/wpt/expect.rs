//! The expectations: every file's counts, and every case that does not
//! pass with the reason kept for it. A reason is a departure of the
//! specification (`D8`), of a decision (`DEC-7369/5`), or a to-do that
//! owes the fix (`TODO-7372/14b`); `expectations/reasons.txt` gives each
//! reason to the cases it explains, by rules read in order.

use std::fmt::Write as _;
use std::path::Path;

use super::page::FileResult;
use super::tree::Case;

/// The reason written for a failing case no rule explains.
pub const UNLISTED: &str = "UNLISTED";

/// One rule: a reason for the cases of files whose path begins with
/// `path` (all of it, when it ends with `$`) whose name and message
/// together hold `holds`.
struct Rule {
    reason: String,
    path: String,
    holds: String,
}

/// The rules of `reasons.txt`.
#[derive(Default)]
pub struct Reasons {
    rules: Vec<Rule>,
}

impl Reasons {
    /// Reads the rules: lines of a reason, a path and what a case holds,
    /// tabs between; `#` begins a comment.
    pub fn read(path: &Path) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let rules = text
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .map(|line| {
                let mut fields = line.split('\t');
                Rule {
                    reason: fields.next().unwrap_or_default().trim().to_owned(),
                    path: fields.next().unwrap_or_default().trim().to_owned(),
                    holds: fields.next().unwrap_or_default().to_owned(),
                }
            })
            .collect();
        Self { rules }
    }

    /// The reason for a case of `file` that says `said`.
    fn of(&self, file: &str, said: &str) -> &str {
        self.rules
            .iter()
            .find(|rule| {
                let path =
                    rule.path.strip_suffix('$').map_or_else(|| file.starts_with(&rule.path), |whole| file == whole || file.split('?').next() == Some(whole));
                path && (rule.holds.is_empty() || said.contains(&rule.holds))
            })
            .map_or(UNLISTED, |rule| rule.reason.as_str())
    }
}

/// What a suite's run is kept as.
#[derive(Default)]
pub struct Kept {
    files: Vec<String>,
    failing: Vec<String>,
    skipped: Vec<String>,
    unlisted: Vec<String>,
    /// Each failing case with what it said, for whoever writes the rules.
    said: Vec<String>,
    passed: usize,
    departures: usize,
    faults: usize,
    not_run: usize,
}

impl Kept {
    fn count(&mut self, reason: &str, line: &str) {
        if reason == UNLISTED {
            self.unlisted.push(line.to_owned());
        }
        if reason.starts_with('D') {
            self.departures += 1;
        } else {
            self.faults += 1;
        }
    }

    /// Keeps what `case` reported.
    pub fn ran(&mut self, case: &Case, result: &FileResult, reasons: &Reasons) {
        let name = case.name();
        let passed = result.tests.iter().filter(|(_, status, _)| status == "PASS").count();
        self.passed += passed;
        self.files.push(format!("FILE\t{name}\t{}\t{passed}\t{}", result.harness, result.tests.len() - passed));
        if result.harness != "OK" {
            // The file as a whole did not run to its end: one line for it.
            let said = format!("{} {}", result.harness, result.detail);
            let reason = reasons.of(&name, &said).to_owned();
            let line = format!("HARNESS\t{reason}\t{name}\t{}\t{}", result.harness, result.detail.replace('\t', " "));
            self.said.push(line.clone());
            self.count(&reason, &line);
            self.failing.push(line);
        }
        let mut seen: Vec<&str> = Vec::new();
        for (test, status, message) in &result.tests {
            if status == "PASS" {
                continue;
            }
            // A name a file gives twice is told apart by its place.
            let again = seen.iter().filter(|earlier| **earlier == test.as_str()).count();
            seen.push(test);
            let test_name = if again == 0 { test.clone() } else { format!("{test} ({})", again + 1) };
            let reason = reasons.of(&name, &format!("{test} ||| {message}")).to_owned();
            let line = format!("{status}\t{reason}\t{name}\t{test_name}");
            self.said.push(format!("{line}\t{message}"));
            self.count(&reason, &line);
            self.failing.push(line);
        }
    }

    /// Keeps that `case` was not run, and why.
    pub fn skipped(&mut self, case: &Case, why: &str, reasons: &Reasons) {
        let name = case.name();
        let reason = reasons.of(&name, why);
        self.not_run += 1;
        self.skipped.push(format!("SKIP\t{why}\t{}\t{name}", if reason == UNLISTED { "-" } else { reason }));
    }

    /// The failing cases no rule explains.
    pub fn unlisted(&self) -> &[String] {
        &self.unlisted
    }

    /// Every failing case with its message, a line each: not kept, since
    /// a message may say more than is stable, but what a rule is written from.
    pub fn said(&self) -> String {
        self.said.join("\n")
    }

    /// The counts, for the gate to print.
    pub fn counts(&self) -> String {
        format!(
            "{} files run, {} not run here; cases: {} pass, {} fail as a listed departure, {} fail as a fault with its to-do ({} of them unlisted)",
            self.files.len(),
            self.not_run,
            self.passed,
            self.departures,
            self.faults,
            self.unlisted.len()
        )
    }

    /// The expectations file's text.
    pub fn text(&self, suite: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# The web-platform-tests' {suite} suite at commit {}, as run in the standalone runner.", script_wpt::COMMIT);
        let _ = writeln!(out, "# Written by the harness (NEOTELETEXT_WPT_BLESS=1); reasons come from reasons.txt.");
        let _ = writeln!(out, "# {}", self.counts());
        let _ = writeln!(out, "# FILE <file> <harness status> <cases passed> <cases not passed>");
        let _ = writeln!(out, "# <status> <reason> <file> <case>     for each case that does not pass");
        let _ = writeln!(out, "# HARNESS <reason> <file> <status> <what was said>     for a file that did not run to its end");
        let _ = writeln!(out, "# SKIP <why> <reason> <file>     for a file not run here");
        for line in self.files.iter().chain(&self.failing).chain(&self.skipped) {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

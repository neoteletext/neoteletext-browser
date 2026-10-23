//! Fault points (`installation-and-updates.md` section 13): a test build
//! of the updater or the launcher, given `NEOTELETEXT_FAULT=<point>`,
//! ends at that point at once, as if killed or the power cut there, so a
//! test can show that every step leaves a version that starts. Only debug
//! builds have them: a release build reads no such variable.

/// The variable that names the point to end at.
pub const VARIABLE: &str = "NEOTELETEXT_FAULT";
/// The exit status of a process ended at a fault point, which a test
/// tells apart from a crash.
pub const EXIT_CODE: i32 = 86;

/// Whether the fault point `name` is the one asked for.
#[must_use]
pub fn armed(name: &str) -> bool {
    #[cfg(debug_assertions)]
    {
        std::env::var(VARIABLE).is_ok_and(|asked| asked == name)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = name;
        false
    }
}

/// Ends the process here if `name` is the fault point asked for.
pub fn point(name: &str) {
    if armed(name) {
        eprintln!("neoteletext: ended at the fault point {name}");
        std::process::exit(EXIT_CODE);
    }
}

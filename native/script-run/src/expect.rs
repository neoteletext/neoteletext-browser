//! The kept results of a run (`--expect DIR`, `--bless`; script-api.md
//! section 16): every snapshot, the sound, the log and the budget, as
//! files in one folder, with `expect.json` beside them:
//!
//! ```json
//! {
//!   "form": 1,
//!   "version": 3,
//!   "files": {
//!     "budget.json": "<SHA-256>",
//!     "log.txt": "<SHA-256>",
//!     "sound.wav": "<SHA-256>",
//!     "tick-000059.png": "<SHA-256>"
//!   }
//! }
//! ```
//!
//! `form` is the runner's: [`EXPECT_FORM`], the version of what it writes
//! for the same run. `version` is the kept set's format version
//! (DEVELOPMENT.md section 8): `--bless` raises it whenever a kept file's
//! bytes change, in the commit that changes them, and leaves it when
//! nothing changed. A comparison fails on any difference: a file whose
//! bytes differ, one the run did not make, one it made that is not kept,
//! or a kept file that no longer has the digest `expect.json` gives it.

use core::fmt::Write as _;

use crate::blobs::Blob;
use crate::json::{self, Value, quote};
use crate::limits::{EXPECT_FORM, MAX_EXPECT_BYTES, MAX_EXPECT_FILES};
use crate::report::sha256;
use crate::run::is_kept;

/// The kept set's own file.
pub const EXPECT_FILE: &str = "expect.json";
/// The characters of a SHA-256 in hexadecimal.
const DIGEST_CHARACTERS: usize = 64;

/// What `expect.json` says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    /// The form the set was written in.
    pub form: u32,
    /// The set's format version.
    pub version: u32,
    /// Each kept file's name and SHA-256, in the names' order.
    pub files: Vec<(String, String)>,
}

/// Reads `expect.json`.
///
/// # Errors
/// What is wrong with it: a member it does not have or does not know, a
/// name that is no kept file's, a digest that is not one.
pub fn parse(bytes: &[u8]) -> Result<Kept, String> {
    if bytes.len() > MAX_EXPECT_BYTES {
        return Err(format!("{EXPECT_FILE} is longer than {MAX_EXPECT_BYTES} bytes"));
    }
    let document = json::parse(bytes, MAX_EXPECT_FILES.saturating_mul(2)).map_err(|what| format!("{EXPECT_FILE} is not one JSON object: {what}"))?;
    let members = document.members().ok_or_else(|| format!("{EXPECT_FILE} is not a JSON object"))?;
    if let Some((name, _)) = members.iter().find(|(name, _)| !matches!(name.as_str(), "form" | "version" | "files")) {
        return Err(format!("{EXPECT_FILE} has a member this runner does not know: `{name}`"));
    }
    let number = |name: &str| {
        document
            .member(name)
            .and_then(Value::number)
            .and_then(|number| u32::try_from(number).ok())
            .ok_or_else(|| format!("{EXPECT_FILE} has no whole number `{name}`"))
    };
    let listed = document.member("files").and_then(Value::members).ok_or_else(|| format!("{EXPECT_FILE} has no object `files`"))?;
    if listed.len() > MAX_EXPECT_FILES {
        return Err(format!("{EXPECT_FILE} names more than {MAX_EXPECT_FILES} files"));
    }
    let mut files = Vec::with_capacity(listed.len());
    for (name, digest) in listed {
        let digest = digest
            .text()
            .filter(|digest| digest.len() == DIGEST_CHARACTERS && digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        let digest = digest.ok_or_else(|| format!("{EXPECT_FILE}: `{name}` has no SHA-256 in lower-case hexadecimal"))?;
        if !is_kept(name) {
            return Err(format!("{EXPECT_FILE}: `{name}` is not the name of a file a run keeps"));
        }
        files.push((name.clone(), digest.to_owned()));
    }
    Ok(Kept { form: number("form")?, version: number("version")?, files })
}

/// `expect.json` for `kept`.
#[must_use]
pub fn write(kept: &Kept) -> Vec<u8> {
    let mut out = String::new();
    let _ = write!(out, "{{\n  \"form\": {},\n  \"version\": {},\n  \"files\": {{", kept.form, kept.version);
    for (index, (name, digest)) in kept.files.iter().enumerate() {
        let _ = write!(out, "{}\n    {}: {}", if index > 0 { "," } else { "" }, quote(name), quote(digest));
    }
    out.push_str("\n  }\n}\n");
    out.into_bytes()
}

/// The files of a run that are kept, in the order of their names.
#[must_use]
pub fn kept_files(produced: &[Blob]) -> Vec<&Blob> {
    let mut files: Vec<&Blob> = produced.iter().filter(|(name, _)| is_kept(name)).collect();
    files.sort_by(|one, other| one.0.cmp(&other.0));
    files
}

/// What `--bless` writes for a run: its kept files' digests, under the
/// old set's version when none of them changed, the next when any did,
/// and 1 for a first set.
#[must_use]
pub fn blessed(old: Option<&Kept>, produced: &[Blob]) -> Kept {
    let files: Vec<(String, String)> = kept_files(produced).into_iter().map(|(name, bytes)| (name.clone(), sha256(bytes))).collect();
    let version = match old {
        Some(old) if old.files == files && old.form == EXPECT_FORM => old.version,
        Some(old) => old.version.saturating_add(1),
        None => 1,
    };
    Kept { form: EXPECT_FORM, version, files }
}

/// Every difference between a run and the kept set, one a line; none
/// when they match. `read` gives a kept file's bytes.
pub fn differences(kept: &Kept, produced: &[Blob], read: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Vec<String> {
    let mut found = Vec::new();
    if kept.form != EXPECT_FORM {
        found.push(format!("{EXPECT_FILE}: kept in form {}, and this runner writes form {EXPECT_FORM}: look at the difference, then --bless", kept.form));
        return found;
    }
    let made = kept_files(produced);
    for (name, digest) in &kept.files {
        let Some((_, bytes)) = made.iter().find(|(made, _)| made == name) else {
            found.push(format!("{name}: kept, and this run did not make it"));
            continue;
        };
        match read(name) {
            None => found.push(format!("{name}: named in {EXPECT_FILE}, and not in the folder")),
            Some(held) if sha256(&held) != *digest => found.push(format!("{name}: the kept file is not the one {EXPECT_FILE} was written for")),
            Some(held) if held != *bytes => {
                found.push(format!("{name}: differs ({} bytes kept, {} made; version {} of the kept set)", held.len(), bytes.len(), kept.version));
            }
            Some(_) => {}
        }
    }
    for (name, _) in made {
        if !kept.files.iter().any(|(kept, _)| kept == name) {
            found.push(format!("{name}: made by this run, and not kept"));
        }
    }
    found
}

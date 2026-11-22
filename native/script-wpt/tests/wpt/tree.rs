//! The suite's test files: which there are, and what each one's `META`
//! lines ask for (the suite's `docs/writing-tests/testharness.md`,
//! "Auto-generated test boilerplate").

use std::path::Path;

/// Where a test file can run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// In a worker's scope, which a page's script is nearest to: no
    /// document.
    #[default]
    Worker,
    /// Not here, and why.
    Skipped(&'static str),
}

/// One test to run: a file, and one of its variants.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Case {
    /// The file, from the suite's root, with `/` between its parts.
    pub path: String,
    /// The variant: empty, or a query beginning with `?`.
    pub variant: String,
    /// The scripts its `META` lines name, from the suite's root.
    pub scripts: Vec<String>,
    /// Whether it asks for the long timeout.
    pub long: bool,
    /// Where it can run.
    pub scope: Scope,
}

impl Case {
    /// The case's name in the expectations: its file and its variant.
    pub fn name(&self) -> String {
        format!("{}{}", self.path, self.variant)
    }
}

/// A path named by a file at `from`: from the root when it begins with
/// `/`, else beside the file; `.` and `..` parts resolved.
pub fn resolve(from: &str, named: &str) -> String {
    let mut parts: Vec<&str> = if named.starts_with('/') {
        Vec::new()
    } else {
        from.split('/').collect::<Vec<_>>().split_last().map(|(_, folder)| folder.to_vec()).unwrap_or_default()
    };
    for part in named.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// The scopes a `global=` line names that have no document.
const WORKER_SCOPES: [&str; 5] = ["worker", "dedicatedworker", "sharedworker", "serviceworker", "dedicatedworker-module"];

fn collect(root: &Path, folder: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(root.join(folder)) else { return };
    let mut names: Vec<_> = entries.flatten().map(|entry| entry.file_name()).collect();
    names.sort();
    for name in names {
        let path = folder.join(&name);
        if root.join(&path).is_dir() {
            collect(root, &path, out);
        } else if let Some(text) = path.to_str() {
            out.push(text.replace('\\', "/"));
        }
    }
}

/// Every test of `folder`, in the order of their paths: each `.any.js`
/// file once for each of its variants, and each file that is a page
/// (`.html`, `.window.js` and the like) once, skipped.
pub fn cases(root: &Path, folder: &str) -> Vec<Case> {
    let mut paths = Vec::new();
    collect(root, Path::new(folder), &mut paths);
    let mut cases = Vec::new();
    for path in paths {
        let file = path.rsplit('/').next().unwrap_or_default();
        // What a test's folder holds beside tests.
        if path.contains("/resources/") || path.contains("/support/") || path.contains("/tools/") || path.contains("/crashtests/") {
            continue;
        }
        let page = [".html", ".htm", ".xhtml", ".window.js", ".svg"].iter().any(|ending| file.ends_with(ending));
        if page && !file.ends_with("-ref.html") {
            cases.push(Case { path, scope: Scope::Skipped("PAGE"), ..Case::default() });
            continue;
        }
        if file.ends_with(".worker.js") && !file.ends_with(".any.worker.js") {
            cases.push(Case { path, scope: Scope::Skipped("WORKER-FILE"), ..Case::default() });
            continue;
        }
        if !file.ends_with(".any.js") {
            continue;
        }
        let source = String::from_utf8_lossy(&std::fs::read(root.join(&path)).unwrap_or_default()).into_owned();
        let mut case = Case { path: path.clone(), ..Case::default() };
        let mut variants = Vec::new();
        for line in source.lines() {
            let Some(meta) = line.strip_prefix("// META:") else {
                if line.trim().is_empty() || line.starts_with("//") {
                    continue;
                }
                break;
            };
            let Some((key, value)) = meta.trim().split_once('=') else { continue };
            match key.trim() {
                "script" => case.scripts.push(resolve(&path, value.trim())),
                "timeout" => case.long = value.trim() == "long",
                "variant" => variants.push(value.trim().to_owned()),
                "global" => {
                    let named: Vec<&str> = value.split(',').map(str::trim).collect();
                    // A scope is taken away with `!`; the default is a window and a dedicated worker.
                    let worker = named.iter().any(|scope| WORKER_SCOPES.contains(scope)) && !named.contains(&"!dedicatedworker");
                    if !worker {
                        case.scope = Scope::Skipped("PAGE");
                    }
                }
                _ => {}
            }
        }
        if variants.is_empty() {
            variants.push(String::new());
        }
        for variant in variants {
            cases.push(Case { variant, ..case.clone() });
        }
    }
    cases
}

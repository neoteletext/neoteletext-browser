#!/usr/bin/env bash
# The project's one check script (requirement 14.1).
#   scripts/check.sh fast   lint, type-check and unit tests; run before each commit
#   scripts/check.sh full   everything, including builds; run before each push
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-full}"
step() { printf '\n== %s\n' "$*"; }

# Crates that build only on Linux: they need the Wayland libraries.
linux_only=(prototypes/spikes/display-host/Cargo.toml)

rust_crates=()
for manifest in native/Cargo.toml prototypes/src-tauri/Cargo.toml prototypes/standin/Cargo.toml prototypes/spikes/*/Cargo.toml prototypes/spikes/*/src-tauri/Cargo.toml; do
    [[ -f "$manifest" ]] || continue
    if [[ "$(uname -s)" != Linux && " ${linux_only[*]} " == *" $manifest "* ]]; then
        printf '\n== Skipped on %s, checked on Linux only: %s\n' "$(uname -s)" "$manifest"
        continue
    fi
    rust_crates+=("$manifest")
done

step "TypeScript lint (prototypes)"
(cd prototypes && npx eslint . --max-warnings 0)

step "TypeScript types (prototypes)"
(cd prototypes && npx tsc -b)

for manifest in "${rust_crates[@]}"; do
    step "Clippy: $manifest"
    cargo clippy --quiet --manifest-path "$manifest" --workspace --all-targets -- -D warnings
    step "Tests: $manifest"
    cargo test --quiet --manifest-path "$manifest" --workspace
done

if [[ "$mode" == "full" ]]; then
    step "Clippy: egui-image (WebAssembly)"
    cargo clippy --quiet --manifest-path prototypes/egui-image/Cargo.toml --target wasm32-unknown-unknown --all-targets -- -D warnings
    step "WebAssembly build"
    (cd prototypes && npm run --silent wasm)
    step "Frontend build"
    (cd prototypes && npm run --silent build)
fi

step "All checks passed ($mode)"

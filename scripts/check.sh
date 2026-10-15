#!/usr/bin/env bash
# The project's one check script (requirement 14.1).
#   scripts/check.sh fast   lint, type-check and unit tests; run before each commit
#   scripts/check.sh full   everything, including builds; run before each push
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-full}"
step() { printf '\n== %s\n' "$*"; }

# Crates that build only on Linux: they need the Wayland libraries.
linux_only=(spikes/display-host/Cargo.toml)

rust_crates=()
for manifest in src-tauri/Cargo.toml standin/Cargo.toml spikes/*/Cargo.toml spikes/*/src-tauri/Cargo.toml; do
    [[ -f "$manifest" ]] || continue
    if [[ "$(uname -s)" != Linux && " ${linux_only[*]} " == *" $manifest "* ]]; then
        printf '\n== Skipped on %s, checked on Linux only: %s\n' "$(uname -s)" "$manifest"
        continue
    fi
    rust_crates+=("$manifest")
done

step "TypeScript lint"
npx eslint . --max-warnings 0

step "TypeScript types"
npx tsc -b

for manifest in "${rust_crates[@]}"; do
    step "Clippy: $manifest"
    cargo clippy --quiet --manifest-path "$manifest" --all-targets -- -D warnings
    step "Tests: $manifest"
    cargo test --quiet --manifest-path "$manifest"
done

if [[ "$mode" == "full" ]]; then
    step "Clippy: egui-image (WebAssembly)"
    cargo clippy --quiet --manifest-path egui-image/Cargo.toml --target wasm32-unknown-unknown --all-targets -- -D warnings
    step "WebAssembly build"
    npm run --silent wasm
    step "Frontend build"
    npm run --silent build
fi

step "All checks passed ($mode)"

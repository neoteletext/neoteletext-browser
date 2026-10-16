#!/usr/bin/env bash
# The project's pinned development tools, installed into tools/ (ignored by
# git) so nothing is installed for the whole machine. scripts/check.sh
# sources this file for the versions and refuses to run with any other.
#   scripts/tools.sh          install whatever is missing or at another version
#   . scripts/tools.sh        define the versions and put tools/bin on PATH
set -euo pipefail

tools_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/tools"

# Each tool and its exact version; cargo install --locked uses the tool's
# own lockfile, so its dependencies are pinned too.
readonly NEXTEST_VERSION="0.9.148"
readonly BACON_VERSION="3.26.0"
readonly DENY_VERSION="0.20.2"
readonly FUZZ_VERSION="0.13.2"
readonly MACHETE_VERSION="0.9.2"
# libFuzzer needs a nightly compiler; this one is pinned by date.
readonly FUZZ_NIGHTLY="nightly-2026-10-01"

# crate | binary | version | how the binary reports it
readonly TOOLS="
cargo-nextest|cargo-nextest|$NEXTEST_VERSION|cargo-nextest nextest --version
bacon|bacon|$BACON_VERSION|bacon --version
cargo-deny|cargo-deny|$DENY_VERSION|cargo-deny --version
cargo-fuzz|cargo-fuzz|$FUZZ_VERSION|cargo-fuzz --version
cargo-machete|cargo-machete|$MACHETE_VERSION|cargo-machete --version
"

export PATH="$tools_root/bin:$PATH"

# Prints each tool that is missing or not at its pinned version.
tools_out_of_date() {
    local crate binary version report
    while IFS='|' read -r crate binary version report; do
        [[ -z "$crate" ]] && continue
        if [[ ! -x "$tools_root/bin/$binary" ]] || ! $report 2>/dev/null | grep -qF "$version"; then
            printf '%s %s\n' "$crate" "$version"
        fi
    done <<<"$TOOLS"
    if ! rustup toolchain list | grep -q "^$FUZZ_NIGHTLY"; then
        printf '%s\n' "$FUZZ_NIGHTLY"
    fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    while IFS='|' read -r crate binary version report; do
        [[ -z "$crate" ]] && continue
        if [[ -x "$tools_root/bin/$binary" ]] && $report 2>/dev/null | grep -qF "$version"; then
            printf '%s %s: installed\n' "$crate" "$version"
            continue
        fi
        printf '%s %s: installing\n' "$crate" "$version"
        cargo install --quiet --locked --root "$tools_root" "$crate" --version "=$version"
    done <<<"$TOOLS"
    rustup toolchain install --profile minimal --component clippy "$FUZZ_NIGHTLY"
fi

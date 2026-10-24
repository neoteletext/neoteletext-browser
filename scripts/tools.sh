#!/usr/bin/env bash
# The project's pinned development tools, installed into tools/ (ignored by
# git) so nothing is installed for the whole machine. scripts/check.sh
# sources this file for the versions and refuses to run with any other.
#   scripts/tools.sh          install whatever is missing or at another version
#   . scripts/tools.sh        define the versions and put tools/bin on PATH
set -euo pipefail

tools_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/tools"

# Each tool and its exact version. Each is built from its crate as
# crates.io published it, downloaded and checked against the SHA-256
# recorded here before anything of it is built (finding F-72), so a
# changed index or download is refused; cargo install --locked then uses
# the crate's own lockfile, whose checksums pin every dependency.
readonly NEXTEST_VERSION="0.9.148"
readonly BACON_VERSION="3.26.0"
readonly DENY_VERSION="0.20.2"
readonly FUZZ_VERSION="0.13.2"
readonly MACHETE_VERSION="0.9.2"
# libFuzzer needs a nightly compiler; this one is pinned by date.
readonly FUZZ_NIGHTLY="nightly-2026-10-01"

# crate | binary | version | SHA-256 of the published crate | how the binary reports its version
readonly TOOLS="
cargo-nextest|cargo-nextest|$NEXTEST_VERSION|7023461ee82c0c2864df74ee340fb7777c13fc556f99e913646ab6ddccd05dd4|cargo-nextest nextest --version
bacon|bacon|$BACON_VERSION|2550a220971b0d2c96e0fd698ba12945bfca3e9bf53fdf26af1f5fa001e6a6e3|bacon --version
cargo-deny|cargo-deny|$DENY_VERSION|e528dfcbe739af7ce37a77d3d6df1b29dd6887b1c701d888820c0f16b864f737|cargo-deny --version
cargo-fuzz|cargo-fuzz|$FUZZ_VERSION|5acfd01930e49823e58c30dd8012d3338a620377d7c7d4cc140ca4b2169400e2|cargo-fuzz --version
cargo-machete|cargo-machete|$MACHETE_VERSION|0f134a108e6c4080e558e32421cb85c064da766937fdb5d50dfa57458edc6678|cargo-machete --version
"
# Where crates.io serves a published crate (its index's "dl" setting).
readonly CRATE_DOWNLOADS="https://static.crates.io/crates"

export PATH="$tools_root/bin:$PATH"

# Prints each tool that is missing or not at its pinned version.
tools_out_of_date() {
    local crate binary version sha256 report
    while IFS='|' read -r crate binary version sha256 report; do
        [[ -z "$crate" ]] && continue
        if [[ ! -x "$tools_root/bin/$binary" ]] || ! $report 2>/dev/null | grep -qF "$version"; then
            printf '%s %s\n' "$crate" "$version"
        fi
    done <<<"$TOOLS"
    if ! rustup toolchain list | grep -q "^$FUZZ_NIGHTLY"; then
        printf '%s\n' "$FUZZ_NIGHTLY"
    fi
}

# The SHA-256 of a file, in hexadecimal.
sha256_of() {
    if command -v sha256sum >/dev/null; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# Downloads a tool's crate, refuses it unless its SHA-256 is the one
# recorded, and builds and installs it into tools/.
install_tool() {
    local crate="$1" version="$2" sha256="$3" work found
    mkdir -p "$tools_root"
    work="$(mktemp -d "$tools_root/download.XXXXXX")" || return 1
    if ! curl --proto '=https' --tlsv1.2 --silent --show-error --fail --location \
        --output "$work/$crate.crate" "$CRATE_DOWNLOADS/$crate/$crate-$version.crate"; then
        rm -rf "$work"
        return 1
    fi
    found="$(sha256_of "$work/$crate.crate")"
    if [[ "$found" != "$sha256" ]]; then
        rm -rf "$work"
        printf '%s %s: the download has SHA-256 %s, not the %s recorded in scripts/tools.sh\n' "$crate" "$version" "$found" "$sha256" >&2
        return 1
    fi
    if ! tar -xzf "$work/$crate.crate" -C "$work" || ! cargo install --quiet --locked --root "$tools_root" --path "$work/$crate-$version"; then
        rm -rf "$work"
        return 1
    fi
    rm -rf "$work"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    while IFS='|' read -r crate binary version sha256 report; do
        [[ -z "$crate" ]] && continue
        if [[ -x "$tools_root/bin/$binary" ]] && $report 2>/dev/null | grep -qF "$version"; then
            printf '%s %s: installed\n' "$crate" "$version"
            continue
        fi
        printf '%s %s: installing\n' "$crate" "$version"
        install_tool "$crate" "$version" "$sha256"
    done <<<"$TOOLS"
    rustup toolchain install --profile minimal --component clippy "$FUZZ_NIGHTLY"
fi

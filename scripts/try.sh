#!/usr/bin/env bash
# The edit loop for native/, with no gate in the way:
#   scripts/try.sh                    watch: Clippy on every save (bacon)
#   scripts/try.sh <crate> [filter]   once: Clippy and the tests of one
#                                     crate, optionally only tests whose
#                                     names contain <filter>
# Both are as strict as the commit gate, so what passes here passes there.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/tools.sh
cd native

if [[ $# -eq 0 ]]; then
    exec bacon
fi
crate="$1"
shift
cargo clippy --quiet --package "$crate" --all-targets -- -D warnings
cargo nextest run --package "$crate" --hide-progress-bar --status-level fail --final-status-level fail "$@"

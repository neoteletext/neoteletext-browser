#!/usr/bin/env bash
# The daily job (installation-and-updates.md section 9.3), on the machine
# that holds the timestamp key alone: a new timestamp naming the current
# snapshot, valid 7 days, copied to the update host. It can say "nothing
# new"; it cannot name a release. If it misses 7 days, every user sees
# "Updates unavailable".
#   scripts/release/timestamp.sh [--test] <repository> <root.json> <timestamp key> [<rsync destination>]
set -euo pipefail
cd "$(dirname "$0")/../.."
# A test identity trusts test keys, which sign for anyone who holds them:
# a release script runs with the production identity unless its first
# argument, --test, says this is a test (finding F-24).
test_flags=()
if [[ "${1:-}" == "--test" ]]; then test_flags=(--test-identity); shift; fi
if [[ -n "${NEOTELETEXT_RELEASE_IDENTITY:-}" && ${#test_flags[@]} -eq 0 ]]; then
    echo "$(basename "$0"): NEOTELETEXT_RELEASE_IDENTITY names a test identity; give --test first for a test release" >&2
    exit 1
fi
repository="${1:?the repository folder}"
root="${2:?the root metadata}"
key="${3:?the timestamp key}"
destination="${4:-}"
release_tool="${NEOTELETEXT_RELEASE_TOOL:-native/tuf/target/release/neoteletext-release}"
if [[ ${#test_flags[@]} -eq 0 && "$key" == file:* ]]; then
    echo "timestamp: $key is a key in a file; a production release uses its timestamp key's own source" >&2
    exit 1
fi
"$release_tool" timestamp "$repository" "$root" --timestamp "$key"
if [[ -n "$destination" ]]; then
    rsync -a "$repository/metadata/timestamp.json" "$destination/metadata/timestamp.json"
fi
echo "timestamp re-signed${destination:+ and copied to $destination}"

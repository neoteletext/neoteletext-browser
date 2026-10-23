#!/usr/bin/env bash
# The daily job (installation-and-updates.md section 9.3), on the machine
# that holds the timestamp key alone: a new timestamp naming the current
# snapshot, valid 7 days, copied to the update host. It can say "nothing
# new"; it cannot name a release. If it misses 7 days, every user sees
# "Updates unavailable".
#   scripts/release/timestamp.sh <repository> <root.json> <timestamp key> [<rsync destination>]
set -euo pipefail
cd "$(dirname "$0")/../.."
repository="${1:?the repository folder}"
root="${2:?the root metadata}"
key="${3:?the timestamp key}"
destination="${4:-}"
release_tool="${NEOTELETEXT_RELEASE_TOOL:-native/tuf/target/release/neoteletext-release}"
"$release_tool" timestamp "$repository" "$root" --timestamp "$key"
if [[ -n "$destination" ]]; then
    rsync -a "$repository/metadata/timestamp.json" "$destination/metadata/timestamp.json"
fi
echo "timestamp re-signed${destination:+ and copied to $destination}"

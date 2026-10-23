#!/usr/bin/env bash
# Release steps 4 and 5 (installation-and-updates.md section 10): each
# platform's signed version is compressed into its targets; the targets
# key adds them to The Update Framework's repository and the snapshot and
# timestamp are signed; then the repository is copied to the update host,
# the targets first, then the metadata, so a client never sees metadata
# naming files that are not there yet.
#   scripts/release/publish.sh <repository> <root.json> <targets key> <snapshot key> <timestamp key> [<rsync destination>] -- <platform>:<version folder>...
# Keys are the release tool's sources; the hardware keys are plugged in
# for this step only. Without a destination nothing is copied anywhere.
set -euo pipefail
cd "$(dirname "$0")/../.."
repository="${1:?the repository folder}"
root="${2:?the root metadata}"
targets_key="${3:?the targets key}"
snapshot_key="${4:?the snapshot key}"
timestamp_key="${5:?the timestamp key}"
shift 5
destination=""
if [[ "${1:-}" != "--" ]]; then destination="${1:-}"; shift; fi
[[ "${1:-}" == "--" ]] && shift
release_tool="${NEOTELETEXT_RELEASE_TOOL:-native/tuf/target/release/neoteletext-release}"
staging="$(mktemp -d "${TMPDIR:-tmp}/neoteletext-targets.XXXXXX")"
for item in "$@"; do
    platform="${item%%:*}"
    folder="${item#*:}"
    "$release_tool" targets "$folder" "$platform" "$staging"
done
"$release_tool" publish "$repository" "$root" "$staging" --targets "$targets_key" --snapshot "$snapshot_key" --timestamp "$timestamp_key"
rm -rf "$staging"
if [[ -n "$destination" ]]; then
    rsync -a "$repository/targets/" "$destination/targets/"
    rsync -a "$repository/metadata/" "$destination/metadata/"
fi
echo "published to $repository${destination:+ and copied to $destination}; record the release in docs/releases.md"

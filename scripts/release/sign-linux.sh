#!/usr/bin/env bash
# Release step 2 for Linux (installation-and-updates.md sections 4.3 and
# 10): the release key signs every file and the manifest, the launcher's
# check is run, and the first-install tarball is made and signed.
#   scripts/release/sign-linux.sh [--test] <version> <out> <key>
# The key is the release tool's source: `command:<public key>:<program>`
# for the hardware key (its program writes the raw Ed25519 signature of
# its input), `file:<path>` only for a test key.
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
version="${1:?a version}"
out="${2:?the build-linux.sh output}"
key="${3:?the release key}"
release_tool="${NEOTELETEXT_RELEASE_TOOL:-native/tuf/target/release/neoteletext-release}"
folder="$out/linux-x86_64/$version"
"$release_tool" sign-linux "$folder" "$version" "$key"
"$release_tool" check "$folder" linux-x86_64 ${test_flags[@]+"${test_flags[@]}"} "${@:4}"
name="neoteletext-$version-linux-x86_64"
rm -rf "$out/tarball" && mkdir -p "$out/tarball/$name"
cp "$folder/neoteletext-launcher" "$out/tarball/$name/neoteletext"
cp -R "$folder" "$out/tarball/$name/$version"
tar -C "$out/tarball" -czf "$out/$name.tar.gz" "$name"
"$release_tool" sign-file "$out/$name.tar.gz" "$key"
"$release_tool" pem "$key" >"$out/neoteletext-release-key.pem"
echo "signed: $out/$name.tar.gz and .sig; users check it with"
echo "  openssl pkeyutl -verify -pubin -inkey neoteletext-release-key.pem -rawin -in $name.tar.gz -sigfile $name.tar.gz.sig"

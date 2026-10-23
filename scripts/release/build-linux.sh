#!/usr/bin/env bash
# Release step 1 for Linux (installation-and-updates.md sections 4.3 and
# 10), on the Linux build host: builds the release in an AlmaLinux 8
# container, whose C library is 2.28, so the programs start on every
# distribution since 2018, and lays out the version. Not yet run: it needs
# the AlmaLinux 8 image (`podman pull docker.io/library/almalinux:8`).
#   scripts/release/build-linux.sh [--test] <version> <out>
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
out="${2:?an output folder}"
image="docker.io/library/almalinux:8"
programs="neoteletext neoteletext-decoder neoteletext-fetcher neoteletext-interface neoteletext-keyboard neoteletext-sound neoteletext-launcher"
mkdir -p "$out"
podman run --rm -v "$PWD:/source:Z" -v "$HOME/.cargo/registry:/root/.cargo/registry:Z" -w /source \
    -e NEOTELETEXT_RELEASE_VERSION="$version" ${NEOTELETEXT_RELEASE_IDENTITY:+-e NEOTELETEXT_RELEASE_IDENTITY="$NEOTELETEXT_RELEASE_IDENTITY"} "$image" bash -euc "
    dnf install -y -q gcc gcc-c++ make perl pkgconf alsa-lib-devel git
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
    . /root/.cargo/env
    arguments=''
    for program in $programs; do arguments=\"\$arguments -p \$program\"; done
    cargo build --release --locked --manifest-path native/Cargo.toml --target-dir /source/tmp/linux-release \$arguments
    cargo build --release --locked --manifest-path native/tuf/Cargo.toml --target-dir /source/tmp/linux-release-tuf -p neoteletext-updater -p neoteletext-release
    objdump -T /source/tmp/linux-release/release/neoteletext-interface | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1"
folder="$out/programs"
mkdir -p "$folder"
for program in $programs; do cp "tmp/linux-release/release/$program" "$folder/"; done
cp tmp/linux-release-tuf/release/neoteletext-updater "$folder/"
rm -rf "$out/linux-x86_64"
tmp/linux-release-tuf/release/neoteletext-release layout linux-x86_64 "$version" "$folder" "$out/linux-x86_64/$version"
echo "built $version: $out/linux-x86_64/$version; sign it with sign-linux.sh"

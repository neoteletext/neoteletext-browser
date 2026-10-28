#!/usr/bin/env bash
# Release step 1 for Linux (installation-and-updates.md sections 4.3 and
# 10), on the Linux build host: builds the release in an AlmaLinux 8
# container, whose C library is 2.28, so the programs start on every
# distribution since 2018, and lays out the version. Everything it takes
# is pinned (finding F-73): the image by its digest, each package by its
# version, rustup-init by its version and SHA-256, and the engine by the
# commit the workspace names, fetched from its published repository. It
# refuses a tree with changes: a release is
# built from a commit. Not yet run end to end.
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
# The version named is the workspace's (decision 123): the browser shows
# NEOTELETEXT_RELEASE_VERSION on its tab strip, and its fetcher sends the
# workspace version in the user agent, so a release where the two differed
# would show one version and send another. Checked before anything is built.
workspace_version="$(sed -n '/^\[workspace.package\]$/,/^\[/s/^version = "\(.*\)"$/\1/p' native/Cargo.toml)"
[[ -n "$workspace_version" ]] || { echo "build-linux: native/Cargo.toml names no workspace version" >&2; exit 1; }
[[ "$version" == "$workspace_version" ]] || { echo "build-linux: the version $version is not the workspace's $workspace_version; set [workspace.package] version in native/Cargo.toml to it and commit first" >&2; exit 1; }
# Outside a git checkout, git status fails and prints nothing, which alone
# would read as an unchanged tree.
changes=$(git status --porcelain --untracked-files=no) || { echo "build-linux: the tree is not a git checkout; a release is built from a commit" >&2; exit 1; }
[[ -z "$changes" ]] || { echo "build-linux: the tree has changes; a release is built from a commit" >&2; exit 1; }
# AlmaLinux 8's image as pulled on 24 October 2026 (`podman image inspect`).
image="docker.io/library/almalinux:8@sha256:e6d31b01fca10ed4f914e84e1139c3a40d6acf3d8346224a32f3e5649b1a3f7d"
# Its repositories' versions on that day (`dnf repoquery --latest-limit 1`).
packages="gcc-8.5.0-29.el8_10.alma.1 gcc-c++-8.5.0-29.el8_10.alma.1 make-4.2.1-11.el8 perl-5.26.3-423.el8_10.1 pkgconf-1.4.2-1.el8 alsa-lib-devel-1.2.10-2.el8 git-2.43.7-1.el8_10"
# rustup-init 1.28.2 for x86-64 Linux, and its SHA-256 as static.rust-lang.org
# publishes it beside it; rustup then installs the toolchain
# rust-toolchain.toml pins.
rustup_version="1.28.2"
rustup_sha256="20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c"
# The engine: one commit of the repository native/Cargo.toml names, which
# Cargo fetches in the container.
engine_rev="$(sed -n 's/^neoteletext-[a-z-]* = { git = "[^"]*", rev = "\([0-9a-f]*\)".*/\1/p' native/Cargo.toml | sort -u)"
[[ -n "$engine_rev" && "$engine_rev" != *$'\n'* ]] || { echo "build-linux: native/Cargo.toml must name one engine commit" >&2; exit 1; }
programs="neoteletext neoteletext-decoder neoteletext-fetcher neoteletext-interface neoteletext-keyboard neoteletext-sound neoteletext-launcher"
mkdir -p "$out"
podman run --rm -v "$PWD:/source:Z" -v "$HOME/.cargo/registry:/root/.cargo/registry:Z" -w /source \
    -e NEOTELETEXT_RELEASE_VERSION="$version" ${NEOTELETEXT_RELEASE_IDENTITY:+-e NEOTELETEXT_RELEASE_IDENTITY="$NEOTELETEXT_RELEASE_IDENTITY"} "$image" bash -euc "
    dnf install -y -q $packages
    curl --proto '=https' --tlsv1.2 -sSf -o /tmp/rustup-init https://static.rust-lang.org/rustup/archive/$rustup_version/x86_64-unknown-linux-gnu/rustup-init
    echo '$rustup_sha256  /tmp/rustup-init' | sha256sum -c -
    chmod +x /tmp/rustup-init && /tmp/rustup-init -y --profile minimal --default-toolchain none
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

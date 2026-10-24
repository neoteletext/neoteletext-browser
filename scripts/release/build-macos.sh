#!/usr/bin/env bash
# Release step 1 on macOS (installation-and-updates.md section 10): builds
# the release at the current commit for both architectures, lays out each
# architecture's bundle for the updates, and a universal bundle for the
# disk image.
#   scripts/release/build-macos.sh [--test] <version> <out>
# The production identity is built in unless NEOTELETEXT_RELEASE_IDENTITY
# names a test one. The full gate must have passed at this commit.
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
# Outside a git checkout, git status fails and prints nothing, which alone
# would read as an unchanged tree.
changes=$(git status --porcelain --untracked-files=no) || { echo "build-macos: the tree is not a git checkout; a release is built from a commit" >&2; exit 1; }
[[ -z "$changes" ]] || { echo "build-macos: the tree has changes; a release is built from a commit" >&2; exit 1; }
programs=(neoteletext neoteletext-decoder neoteletext-fetcher neoteletext-interface neoteletext-keyboard neoteletext-sound neoteletext-launcher)
mkdir -p "$out"
cargo build --quiet --manifest-path native/tuf/Cargo.toml --release -p neoteletext-release
release_tool="native/tuf/target/release/neoteletext-release"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    arguments=()
    for program in "${programs[@]}"; do arguments+=(-p "$program"); done
    NEOTELETEXT_RELEASE_VERSION="$version" cargo build --quiet --release --locked --manifest-path native/Cargo.toml --target "$target" "${arguments[@]}"
    cargo build --quiet --release --locked --manifest-path native/tuf/Cargo.toml --target "$target" -p neoteletext-updater
    folder="$out/programs-$target"
    mkdir -p "$folder"
    for program in "${programs[@]}"; do cp "native/target/$target/release/$program" "$folder/"; done
    cp "native/tuf/target/$target/release/neoteletext-updater" "$folder/"
    case "$target" in aarch64-*) platform=macos-arm64 ;; *) platform=macos-x86_64 ;; esac
    rm -rf "$out/$platform"
    "$release_tool" layout "$platform" "$version" "$folder" "$out/$platform/$version"
done
# The disk image's bundle: each program both architectures (section 4.1).
rm -rf "$out/universal" && mkdir -p "$out/universal"
cp -R "$out/macos-arm64/$version/NeoTeletext.app" "$out/universal/"
for file in "$out/universal/NeoTeletext.app/Contents/MacOS/"*; do
    name="$(basename "$file")"
    lipo -create -output "$file" "$out/programs-aarch64-apple-darwin/$name" "$out/programs-x86_64-apple-darwin/$name"
done
echo "built $version: $out/macos-arm64, $out/macos-x86_64 (updates) and $out/universal (the disk image); sign them with sign-macos.sh"

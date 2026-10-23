#!/usr/bin/env bash
# Release step 2 on macOS (installation-and-updates.md section 10):
# signs every program and the bundle with the Developer ID Application
# certificate and the hardened runtime, notarises it with notarytool and
# staples the ticket, then lists and checks it as the launcher will.
#   scripts/release/sign-macos.sh [--test] <version> <out> "<Developer ID Application: name (TEAM)>" <notarytool profile>
# The notarytool profile is the owner's stored credentials
# (`xcrun notarytool store-credentials`); nothing here holds a secret.
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
out="${2:?the build-macos.sh output}"
identity="${3:?the signing identity}"
profile="${4:?the notarytool profile}"
release_tool="native/tuf/target/release/neoteletext-release"
sign() {
    local bundle="$1" file
    for file in "$bundle"/Contents/MacOS/*; do
        codesign --force --options runtime --timestamp --sign "$identity" "$file"
    done
    codesign --force --options runtime --timestamp --sign "$identity" "$bundle"
    ditto -c -k --keepParent "$bundle" "$bundle.zip"
    xcrun notarytool submit "$bundle.zip" --keychain-profile "$profile" --wait
    xcrun stapler staple "$bundle"
    rm "$bundle.zip"
}
for platform in macos-arm64 macos-x86_64; do
    sign "$out/$platform/$version/NeoTeletext.app"
    "$release_tool" manifest "$out/$platform/$version" "$platform" "$version"
    "$release_tool" check "$out/$platform/$version" "$platform" ${test_flags[@]+"${test_flags[@]}"} "${@:5}"
done
sign "$out/universal/NeoTeletext.app"
hdiutil create -quiet -volname NeoTeletext -srcfolder "$out/universal/NeoTeletext.app" -ov -format UDZO "$out/NeoTeletext-$version.dmg"
codesign --force --timestamp --sign "$identity" "$out/NeoTeletext-$version.dmg"
xcrun notarytool submit "$out/NeoTeletext-$version.dmg" --keychain-profile "$profile" --wait
xcrun stapler staple "$out/NeoTeletext-$version.dmg"
echo "signed and notarised: $out/NeoTeletext-$version.dmg; the updates' folders are listed and checked"

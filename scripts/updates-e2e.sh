#!/usr/bin/env bash
# The end-to-end test of installation and updates on this desktop
# (installation-and-updates.md section 13), macOS or Linux, run by hand:
#   scripts/updates-e2e.sh
# Everything is made under tmp/e2e-<system>/ (ignored): test keys and a
# test identity; the browser built with it; each release laid out, signed
# (macOS: ad hoc, the pinned requirement the bundle's identifier, with no
# certificate and no keychain touched; Linux: the test release key),
# listed, checked as the
# launcher checks, and published to a repository a local HTTPS host serves;
# a home folder of its own, where the browser is installed and run. It
# shows, with the real programs:
#   1. the first install (Linux: the signed tarball's --install; macOS: the
#      bundle copied to an Applications folder) and a healthy start;
#   2. an update checked and staged by the confined updater, accepted by
#      the launcher's pinned signer, tried and committed;
#   3. a release that crashes at start, rolled back after two starts;
#   4. a release not signed by the pinned signer, refused;
#   5. expired metadata, refused;
#   6. a full disk, failing cleanly while the current version runs;
#   7. on Linux, a launcher release replacing the launcher;
#   8. removal, asking before the settings go.
# A window opens for each start: the version's health is its window drawn.
# On Linux it opens on a virtual display (Xvfb).
set -euo pipefail
cd "$(dirname "$0")/.."
repository_root="$PWD"
system="$(uname -s | tr '[:upper:]' '[:lower:]')"
work="$repository_root/tmp/e2e-$system"
port="${NEOTELETEXT_E2E_PORT:-8443}"
case "$system" in
darwin) platform="macos-$(uname -m | sed 's/x86_64/x86_64/; s/arm64/arm64/')" ;;
linux) platform="linux-x86_64" ;;
*) echo "updates-e2e: macOS or Linux; Windows has scripts/updates-e2e.ps1" >&2; exit 1 ;;
esac

step() { printf '\n== %s\n' "$*"; }
fail() {
    printf '\nFAILED: %s\n' "$*" >&2
    exit 1
}

# Everything of a run before goes but its build folders, which only save time.
mkdir -p "$work"
find "$work" -mindepth 1 -maxdepth 1 ! -name target ! -name tuf-target ! -name tuf-tools -exec rm -rf {} +
identity="$work/identity"
home="$work/home"
mkdir -p "$home"
export NEOTELETEXT_RELEASE_IDENTITY="$identity"

# ---- The test identity ----
# macOS: ad-hoc signatures, with no certificate at all, since codesign
# signs only with an identity the user's trust settings trust, and a test
# must not change those or the login keychain. The pinned requirement is
# then the bundle's identifier: a release signed as another program, or
# changed after signing, is refused by the same codesign check a release
# with the Developer ID requirement goes through.
macos_requirement=""
if [[ "$system" == darwin ]]; then
    macos_requirement='identifier "com.neoteletext.browser"'
fi

step "the release tool, the test host and the updater's driver"
env -u NEOTELETEXT_RELEASE_IDENTITY cargo build --quiet --manifest-path native/tuf/Cargo.toml --target-dir "$work/tuf-tools" -p neoteletext-release
tools="$work/tuf-tools/debug"
if [[ -n "$macos_requirement" ]]; then
    "$tools/neoteletext-release" test-identity "$identity" --host localhost --port "$port" --macos-requirement "$macos_requirement"
else
    "$tools/neoteletext-release" test-identity "$identity" --host localhost --port "$port"
fi
# Rebuilt with the identity, now whole.
cargo build --quiet --manifest-path native/tuf/Cargo.toml --target-dir "$work/tuf-tools" -p neoteletext-release -p neoteletext-updater --bins
key() { printf 'file:%s/keys/%s.pk8' "$identity" "$1"; }

# ---- Building and releasing ----
programs_of() { printf '%s/programs-%s' "$work" "$1"; }

build() {
    local version="$1" programs
    programs="$(programs_of "$version")"
    step "the browser $version, built with the test identity"
    NEOTELETEXT_RELEASE_VERSION="$version" cargo build --quiet --release --manifest-path native/Cargo.toml --target-dir "$work/target" \
        -p neoteletext -p neoteletext-decoder -p neoteletext-fetcher -p neoteletext-interface -p neoteletext-keyboard -p neoteletext-sound -p neoteletext-launcher
    cargo build --quiet --release --manifest-path native/tuf/Cargo.toml --target-dir "$work/tuf-target" -p neoteletext-updater
    mkdir -p "$programs"
    local program
    for program in neoteletext neoteletext-decoder neoteletext-fetcher neoteletext-interface neoteletext-keyboard neoteletext-sound neoteletext-launcher; do
        cp "$work/target/release/$program" "$programs/"
    done
    cp "$work/tuf-target/release/neoteletext-updater" "$programs/"
}

# A version whose broker is the launcher's test broker, which behaves as
# its word says (crash, hang, quiet), with the version's mark.
broken() {
    local version="$1" behaviour="$2" from="$3" programs
    programs="$(programs_of "$version")"
    cargo build --quiet --release --manifest-path native/Cargo.toml --target-dir "$work/target" -p neoteletext-launcher --bin launcher-test-broker
    cp -R "$(programs_of "$from")" "$programs"
    # In place, not appended: codesign refuses a program with bytes after
    # its last segment.
    "$work/target/release/launcher-test-broker" --make "$version" "$behaviour" "$programs/neoteletext"
}

# Signs a macOS bundle ad hoc as the browser, or, with "wrong", as another
# program the pinned requirement does not name.
sign_bundle() {
    local bundle="$1" how="${2:-test}" file
    local -a identity_arguments=(--sign -)
    [[ "$how" == wrong ]] && identity_arguments=(--sign - --identifier com.example.other)
    for file in "$bundle"/Contents/MacOS/*; do
        codesign --force --options runtime --timestamp=none --sign - "$file"
    done
    codesign --force --options runtime --timestamp=none "${identity_arguments[@]}" "$bundle"
}

# Lays out, signs, lists, checks, compresses and publishes `version`.
release() {
    local version="$1" how="${2:-test}" built="$work/built/$1"
    step "release $version: laid out, signed ($how), listed, checked, published"
    rm -rf "$built"
    "$tools/neoteletext-release" layout "$platform" "$version" "$(programs_of "$version")" "$built"
    if [[ "$system" == darwin ]]; then
        sign_bundle "$built/NeoTeletext.app" "$how"
        "$tools/neoteletext-release" manifest "$built" "$platform" "$version"
    else
        local signer
        signer="$(key linux-release)"
        if [[ "$how" == wrong ]]; then
            "$tools/neoteletext-release" key "$work/wrong.pk8" 2>/dev/null || true
            signer="file:$work/wrong.pk8"
        fi
        "$tools/neoteletext-release" sign-linux "$built" "$version" "$signer"
    fi
    if [[ "$how" == test ]]; then
        "$tools/neoteletext-release" check "$built" "$platform" --test-identity
    fi
    rm -rf "$work/targets-$version"
    "$tools/neoteletext-release" targets "$built" "$platform" "$work/targets-$version"
    "$tools/neoteletext-release" publish "$work/repository" "$identity/root.json" "$work/targets-$version" \
        --targets "$(key targets)" --snapshot "$(key snapshot)" --timestamp "$(key timestamp)" "${@:3}"
}

# ---- The installation and its runs ----
if [[ "$system" == darwin ]]; then
    # Where the installed programs are, which a start's end ends.
    installed="$work/Applications/"
    bundle="$work/Applications/NeoTeletext.app"
    updates="$home/Library/Application Support/NeoTeletext Updates"
    launcher="$bundle/Contents/MacOS/neoteletext-launcher"
    updater_of() { printf '%s/Contents/MacOS/neoteletext-updater' "$bundle"; }
    installed_broker() { printf '%s/Contents/MacOS/neoteletext' "$bundle"; }
    settings="$home/Library/Application Support/NeoTeletext"
else
    installed="$home/"
    updates="$home/.local/share/neoteletext"
    launcher="$updates/bin/neoteletext"
    updater_of() { printf '%s/versions/%s/neoteletext-updater' "$updates" "$(current)"; }
    installed_broker() { printf '%s/versions/%s/neoteletext' "$updates" "$(running)"; }
    settings="$home/.config/neoteletext"
fi
state_file="$updates/state"
current() { sed -n 's/^current //p' "$state_file"; }
running() { local trial; trial="$(sed -n 's/^trial \([^ ]*\) .*/\1/p' "$state_file")"; [[ -n "$trial" ]] && echo "$trial" || current; }
mark_of() { LC_ALL=C grep -ao $'\001neoteletext-version\001[0-9.]*' "$1" | tr -d '\001' | sed 's/neoteletext-version//' | head -1; }

# Runs the browser from the launcher, as a user would, until the launcher
# commits or the browser says it is healthy, or until `$2` seconds pass,
# then ends it. The launcher's and the browser's words are in `$1`.
start() {
    local log="$1" limit="${2:-60}" launched
    : >"$log"
    if [[ "$system" == darwin ]]; then
        env -i HOME="$home" PATH=/usr/bin:/bin TMPDIR="$work/" "$launcher" 2>"$log" &
    else
        env -i HOME="$home" PATH=/usr/bin:/bin XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-}" xvfb-run -a "$launcher" 2>"$log" &
    fi
    launched=$!
    local waited=0
    until grep -q -e 'is healthy and committed' -e 'the version is healthy' -e 'rolled back' -e 'is refused' "$log" 2>/dev/null; do
        if ! kill -0 "$launched" 2>/dev/null; then break; fi
        if ((waited >= limit)); then break; fi
        sleep 1
        waited=$((waited + 1))
    done
    # The core says it is healthy a moment before the launcher commits a
    # trial; a committed version's start says no more.
    local settle=0
    while grep -q 'the version is healthy' "$log" && ! grep -q 'committed' "$log" && ((settle < 5)); do
        sleep 1
        settle=$((settle + 1))
    done
    # A rollback starts the current version, whose health ends this start.
    if grep -q -e 'rolled back' -e 'is refused' "$log"; then
        until grep -q 'the version is healthy' "$log" || ((waited >= limit)); do
            sleep 1
            waited=$((waited + 1))
        done
    fi
    # Only what runs from the installation: xvfb-run's own command line
    # names the launcher too, and ended before it has ended its Xvfb, it
    # leaves the display server running for good.
    pkill -f "^$installed" 2>/dev/null || true
    wait "$launched" 2>/dev/null || true
}

expect_state() {
    local what="$1" pattern="$2"
    grep -q -- "$pattern" "$state_file" || { cat "$state_file"; fail "$what: the state lacks \"$pattern\""; }
    echo "ok: $what"
}

serve() {
    step "the test host on localhost:$port"
    : >"$work/host.out"
    "$tools/neoteletext-test-host" "$work/repository" "$identity/host.pem" "$identity/host.key.pem" "localhost:$port" 2>"$work/host.log" >"$work/host.out" &
    host=$!
    until grep -q listening "$work/host.out"; do sleep 0.1; done
}

drive() {
    env -i HOME="$home" PATH=/usr/bin:/bin "$tools/updater-drive" "$(updater_of)" "$@"
}

trap 'pkill -f "^$installed" 2>/dev/null || true; [[ -n "${host:-}" ]] && kill "$host" 2>/dev/null || true' EXIT

# 1. The first install.
build 0.1.0
release 0.1.0
if [[ "$system" == darwin ]]; then
    step "1. first install: the bundle copied to an Applications folder"
    mkdir -p "$work/Applications"
    ditto "$work/built/0.1.0/NeoTeletext.app" "$bundle"
    codesign --verify --deep --strict "-R=$macos_requirement" "$bundle"
    hdiutil create -quiet -volname NeoTeletext -srcfolder "$work/built/0.1.0/NeoTeletext.app" -ov -format UDZO "$work/NeoTeletext-0.1.0.dmg"
    echo "ok: the disk image $work/NeoTeletext-0.1.0.dmg"
else
    step "1. first install: the signed tarball's --install"
    tarball="$work/tarball/neoteletext-0.1.0-$platform"
    mkdir -p "$tarball"
    cp "$work/built/0.1.0/neoteletext-launcher" "$tarball/neoteletext"
    cp -R "$work/built/0.1.0" "$tarball/0.1.0"
    (cd "$work/tarball" && tar -czf "neoteletext-0.1.0-$platform.tar.gz" "neoteletext-0.1.0-$platform")
    "$tools/neoteletext-release" sign-file "$work/tarball/neoteletext-0.1.0-$platform.tar.gz" "$(key linux-release)"
    "$tools/neoteletext-release" pem "$(key linux-release)" >"$work/release-key.pem"
    openssl pkeyutl -verify -pubin -inkey "$work/release-key.pem" -rawin -in "$work/tarball/neoteletext-0.1.0-$platform.tar.gz" \
        -sigfile "$work/tarball/neoteletext-0.1.0-$platform.tar.gz.sig"
    rm -rf "$work/unpacked" && mkdir -p "$work/unpacked"
    tar -xzf "$work/tarball/neoteletext-0.1.0-$platform.tar.gz" -C "$work/unpacked"
    env -i HOME="$home" PATH=/usr/bin:/bin "$work/unpacked/neoteletext-0.1.0-$platform/neoteletext" --install
    [[ -x "$launcher" && -L "$home/.local/bin/neoteletext" && -f "$home/.local/share/applications/neoteletext.desktop" ]] || fail "the install is not whole"
    echo "ok: installed, on the path, in the menu"
fi
start "$work/run-1.log"
grep -q 'the version is healthy' "$work/run-1.log" || { cat "$work/run-1.log"; fail "the first start was not healthy"; }
expect_state "1. the first install runs" "current 0.1.0"

# 2. An update.
serve
build 0.1.1
release 0.1.1
step "2. the confined updater checks and stages 0.1.1"
drive stage 0.1.1 | tee "$work/drive-2.txt"
grep -q 'Staged' "$work/drive-2.txt" || fail "0.1.1 was not staged"
start "$work/run-2.log"
grep -q 'version 0.1.1 is healthy and committed' "$work/run-2.log" || { cat "$work/run-2.log"; fail "0.1.1 was not committed"; }
expect_state "2. the update committed" "current 0.1.1"
expect_state "2. the old version kept" "previous 0.1.0"
[[ "$(mark_of "$(installed_broker)")" == 0.1.1 ]] || fail "the installed broker is not 0.1.1"

# 3. A release that crashes.
broken 0.1.2 crash 0.1.1
release 0.1.2
step "3. 0.1.2 crashes at start"
drive stage 0.1.2 >/dev/null
start "$work/run-3.log" 120
grep -q 'failed to start 2 times and is rolled back to 0.1.1' "$work/run-3.log" || { cat "$work/run-3.log"; fail "0.1.2 was not rolled back"; }
expect_state "3. rolled back" "current 0.1.1"
expect_state "3. marked bad" "bad 0.1.2"
[[ "$(mark_of "$(installed_broker)")" == 0.1.1 ]] || fail "the installed broker is not 0.1.1 after the rollback"

# 4. A release the pinned signer did not sign.
build 0.1.3
release 0.1.3 wrong
step "4. 0.1.3, not signed by the pinned signer"
drive stage 0.1.3 >/dev/null
start "$work/run-4.log"
grep -q 'version 0.1.3 is refused' "$work/run-4.log" || { cat "$work/run-4.log"; fail "0.1.3 was not refused"; }
# Refused, removed, and not marked bad: "bad" is a trial that failed
# (finding F-08).
! grep -q "bad 0.1.3" "$state_file" || fail "4. a refused version was marked bad"
[[ ! -e "$updates/staging/0.1.3" ]] || fail "4. the refused version was not removed"
echo "ok: 4. refused and removed, not marked bad"
expect_state "4. the current version still runs" "current 0.1.1"

# 5. Expired metadata.
step "5. an expired timestamp"
"$tools/neoteletext-release" timestamp "$work/repository" "$identity/root.json" --timestamp "$(key timestamp)" --timestamp-days -1
if drive check >"$work/drive-5.txt" 2>&1; then fail "expired metadata was accepted"; fi
grep -qi 'expire' "$work/drive-5.txt" || { cat "$work/drive-5.txt"; fail "not refused as expired"; }
echo "ok: 5. refused as expired"
"$tools/neoteletext-release" timestamp "$work/repository" "$identity/root.json" --timestamp "$(key timestamp)"

# 6. A full disk.
build 0.1.4
release 0.1.4
step "6. a staging folder on a disk too small for the release"
staging="$updates/staging"
rm -rf "$staging" && mkdir -p "$staging"
if [[ "$system" == darwin ]]; then
    hdiutil create -quiet -size 2m -fs HFS+ -volname Small "$work/small.dmg"
    hdiutil attach -quiet -nobrowse -mountpoint "$staging" "$work/small.dmg"
    unmount_small() { hdiutil detach -quiet "$staging"; }
else
    sudo mount -t tmpfs -o size=2m,uid="$(id -u)",gid="$(id -g)" tmpfs "$staging"
    unmount_small() { sudo umount "$staging"; }
fi
if drive stage 0.1.4 >"$work/drive-6.txt" 2>&1; then unmount_small; fail "a full disk staged a release"; fi
grep -qi 'space' "$work/drive-6.txt" || { cat "$work/drive-6.txt"; unmount_small; fail "not refused for want of space"; }
start "$work/run-6.log"
unmount_small
grep -q 'the version is healthy' "$work/run-6.log" || { cat "$work/run-6.log"; fail "the current version did not run"; }
expect_state "6. the current version runs beside a full disk" "current 0.1.1"

# 7. A launcher release (Linux: the launcher's folder exchanged).
if [[ "$system" == linux ]]; then
    build 0.1.5
    printf 'a launcher release' >>"$(programs_of 0.1.5)/neoteletext-launcher"
    release 0.1.5
    step "7. 0.1.5, a launcher release"
    drive stage 0.1.5 >/dev/null
    start "$work/run-7.log"
    expect_state "7. committed" "current 0.1.5"
    cmp -s "$launcher" "$(programs_of 0.1.5)/neoteletext-launcher" || fail "the launcher was not replaced"
    [[ -d "$updates/bin.old" ]] || fail "the old launcher was not kept"
    start "$work/run-7b.log"
    [[ ! -d "$updates/bin.old" ]] || fail "the old launcher was kept after the new one's healthy start"
    echo "ok: 7. the launcher replaced, the old one kept until the new one's healthy start"
fi

# 8. Removal.
step "8. removal, asking before the settings go"
mkdir -p "$settings" && touch "$settings/settings.txt"
printf 'y\ny\n' | env -i HOME="$home" PATH=/usr/bin:/bin "$launcher" --uninstall
[[ ! -e "$updates" ]] || fail "the installation is still there"
[[ ! -e "$settings" ]] || fail "the settings are still there"
if [[ "$system" == darwin ]]; then [[ ! -e "$bundle" ]] || fail "the bundle is still there"; fi
echo "ok: 8. removed"

step "All end-to-end checks passed ($system)"

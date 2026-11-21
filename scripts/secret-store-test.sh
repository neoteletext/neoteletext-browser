#!/usr/bin/env bash
# Runs the secret store's ignored tests on Linux against a real Secret
# Service without touching the desktop's: a private session bus
# (dbus-run-session) and a GNOME keyring daemon whose keyrings, runtime
# folder and control socket all live in a folder of the build's, unlocked
# with a throwaway password. Without its own runtime folder the daemon
# would use the desktop's control directory.
#   scripts/secret-store-test.sh
set -euo pipefail
cd "$(dirname "$0")/../native"
target="${CARGO_TARGET_DIR:-$PWD/target}"
keyrings="$target/secret-store-test"
rm -rf "$keyrings"
mkdir -p "$keyrings/runtime"
chmod 700 "$keyrings/runtime"
command -v dbus-run-session >/dev/null || { echo "needs dbus-run-session (the dbus-daemon package)" >&2; exit 1; }
command -v gnome-keyring-daemon >/dev/null || { echo "needs gnome-keyring-daemon (the gnome-keyring package)" >&2; exit 1; }
XDG_DATA_HOME="$keyrings" XDG_RUNTIME_DIR="$keyrings/runtime" dbus-run-session -- bash -c '
    printf throwaway | gnome-keyring-daemon --unlock --components=secrets --control-directory="$XDG_RUNTIME_DIR/keyring" >/dev/null
    cargo test --quiet -p neoteletext-secret-store -- --ignored
'

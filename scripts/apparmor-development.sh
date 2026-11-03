#!/usr/bin/env bash
# Development only: lets the browser's sandbox tests make the user
# namespaces they need on a Linux machine that restricts them, as Ubuntu
# from 24.04 does (kernel.apparmor_restrict_unprivileged_userns = 1;
# docs/sandboxing.md section 8.5).
#
#   scripts/apparmor-development.sh           install or refresh (sudo)
#   scripts/apparmor-development.sh --remove  take it away (sudo)
#
# It installs one AppArmor profile that allows a user namespace, and nothing
# else, to every program under this checkout's build folders and those of
# its worktrees: the profile a release carries (neoteletext.apparmor,
# decision 77) does the same for an installation's versions/* folders.
# Releases are untouched: nothing here is shipped.
set -euo pipefail
cd "$(dirname "$0")/.."

target=/etc/apparmor.d/neoteletext-development
restriction=/proc/sys/kernel/apparmor_restrict_unprivileged_userns

if [[ "${1:-}" == "--remove" ]]; then
    if [[ -f "$target" ]]; then
        sudo apparmor_parser -R "$target" || true
        sudo rm -f "$target"
    fi
    echo "removed $target"
    exit 0
fi

if [[ "$(cat "$restriction" 2>/dev/null || echo 0)" != "1" ]]; then
    echo "this machine does not restrict user namespaces ($restriction): no profile is needed"
    exit 0
fi

# AppArmor names a program by the path the kernel resolved.
root="$(pwd -P)"
profile="$(mktemp)"
trap 'rm -f "$profile"' EXIT
cat >"$profile" <<EOF
# NeoTeletext browser, development: programs built in $root, and in its
# worktrees, may make a user namespace for the sandbox's tests
# (scripts/apparmor-development.sh, docs/sandboxing.md section 8.5).
# It allows userns, and nothing else.
abi <abi/4.0>,
include <tunables/global>

profile neoteletext-development "$root{,/.claude/worktrees/*}/native/{target,fuzz/target}/**" flags=(unconfined) {
  userns,

  include if exists <local/neoteletext-development>
}
EOF
sudo apparmor_parser -Q "$profile"
sudo install -m 644 "$profile" "$target"
sudo apparmor_parser -r "$target"
echo "installed $target for $root"

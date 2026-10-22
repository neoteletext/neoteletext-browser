#!/usr/bin/env bash
# The project's one check script (requirement 14.1). Every step either
# passes or fails the run: nothing is skipped, a missing tool or an
# unreachable host is a failure.
#   scripts/check.sh commit   the pre-commit gate: formatting, lints and
#                             tests of only what the staged files touch,
#                             refused while those parts have changes that
#                             are not staged, so what is checked is what
#                             is committed
#   scripts/check.sh full     the pre-push gate: everything, on a clean
#                             tree at the commit being pushed, including
#                             release tests, licences and advisories,
#                             unused dependencies, fuzzing, and the Linux
#                             build and tests on $NEOTELETEXT_LINUX_HOST
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-}"
# The Linux host the browser is built and tested on: the fastest of the
# Linux test machines (20 cores, against oldvga's 4).
linux_host="${NEOTELETEXT_LINUX_HOST:-foundintelquiet4060}"
# Seconds each fuzz target runs in the full gate, after replaying every
# input that has ever found a fault (native/fuzz/regressions/).
fuzz_seconds="${NEOTELETEXT_FUZZ_SECONDS:-20}"

step() { printf '\n== %s\n' "$*"; }
fail() {
    printf '\nFAILED: %s\n' "$*" >&2
    exit 1
}

. scripts/tools.sh
out_of_date="$(tools_out_of_date)"
[[ -z "$out_of_date" ]] || fail "tools missing or at another version (run scripts/tools.sh):
$out_of_date"

# The crates that read input from outside their process, each with the
# same no-panic lint list at its root.
readonly NO_PANIC_ROOTS=(native/protocol/src/lib.rs native/decoder/src/lib.rs native/decoder/src/main.rs native/fetcher/src/main.rs
    native/core/src/lib.rs native/core/src/main.rs native/interface/src/lib.rs native/interface/src/main.rs native/web-opener/src/lib.rs
    native/keyboard/src/main.rs native/sound/src/lib.rs native/sound/src/main.rs native/confine/src/lib.rs)

check_no_panic_lists() {
    step "the no-panic lint list is the same in every crate that reads outside input"
    local first="" root list
    for root in "${NO_PANIC_ROOTS[@]}"; do
        [[ -f "$root" ]] || fail "$root is listed in NO_PANIC_ROOTS but does not exist"
        list="$(sed -n '/^#!\[deny(/,/^)\]/p' "$root")"
        [[ -n "$list" ]] || fail "$root has no #![deny(...)] list"
        if [[ -z "$first" ]]; then
            first="$list"
        elif [[ "$list" != "$first" ]]; then
            fail "$root's #![deny(...)] list differs from ${NO_PANIC_ROOTS[0]}'s"
        fi
    done
}

check_native() {
    # The fuzz crate has a lockfile of its own, which a dependency change
    # in the workspace can leave stale.
    step "native: lockfiles up to date"
    (cd native && cargo metadata --quiet --locked --format-version 1 >/dev/null)
    (cd native/fuzz && cargo metadata --quiet --locked --format-version 1 >/dev/null)
    step "native: formatting"
    (cd native && cargo fmt --check)
    check_no_panic_lists
    step "native: Clippy, warnings as errors"
    (cd native && cargo clippy --quiet --workspace --all-targets -- -D warnings)
    # Hyperlinks are a feature on by default; the browser must build without
    # them too (hyperlinks.md).
    step "native: Clippy with hyperlinks, typing and the phone keyboard compiled out, warnings as errors"
    (cd native && cargo clippy --quiet --workspace --all-targets --no-default-features -- -D warnings)
    # Each optional area alone, and typing without the phone keyboard
    # (interaction.md, requirement 18.10).
    local area
    for area in "links" "interaction" "links interaction"; do
        step "native: Clippy with only: $area"
        (cd native && cargo clippy --quiet --workspace --all-targets --no-default-features --features "$(feature_list $area)" -- -D warnings)
    done
    step "native: tests"
    (cd native && cargo nextest run --workspace --hide-progress-bar --status-level fail --final-status-level fail)
}

# The packages that have each optional area, as `package/area` features.
feature_list() {
    local area package list=""
    for area in "$@"; do
        for package in interface-core protocol neoteletext-decoder neoteletext-fetcher neoteletext neoteletext-interface; do
            list="${list:+$list,}$package/$area"
        done
    done
    printf '%s' "$list"
}

# Paths with changes that are not staged, or files git does not track,
# under any of the given paths.
unstaged_under() {
    git diff --name-only -- "$@"
    git ls-files --others --exclude-standard -- "$@"
}

commit_gate() {
    local staged
    staged="$(git diff --cached --name-only)"
    if [[ -z "$staged" ]]; then
        step "nothing staged"
        return
    fi

    local native=no path
    while IFS= read -r path; do
        case "$path" in
        native/* | scripts/* | .githooks/* | rust-toolchain.toml) native=yes ;;
        esac
    done <<<"$staged"

    # What is checked must be what is committed: no unstaged edit or
    # untracked file may sit in a part being checked.
    if [[ "$native" == yes ]]; then
        local loose
        loose="$(unstaged_under native scripts .githooks rust-toolchain.toml)"
        [[ -z "$loose" ]] || fail "changes that are not staged would make this check test something other than the commit; stage or stash them:
$loose"
        check_native
    fi
    return 0
}

check_linux() {
    step "Linux on $linux_host: native, Clippy and tests"
    ssh -o BatchMode=yes -o ConnectTimeout=10 "$linux_host" true || fail "$linux_host is unreachable; the full gate needs it"
    rsync -a --delete --exclude target --exclude tools --exclude .git \
        --exclude native/fuzz/corpus --exclude native/fuzz/artifacts ./ "$linux_host:neoteletext-browser-check/"
    # The engine is a git dependency on a repository on this machine; its
    # history goes to the host, and git there reads it in place of this
    # machine's path, for these commands only.
    local engine
    engine="$(sed -n 's/^neoteletext-[a-z]* = { git = "\(file:[^"]*\)".*/\1/p' native/Cargo.toml | sort -u)"
    [[ -n "$engine" && "$engine" != *$'\n'* ]] || fail "native/Cargo.toml must name one engine repository, found: ${engine:-none}"
    rsync -a --delete "${engine#file://}/.git/" "$linux_host:neoteletext-browser-check-engine.git/"
    # rustup installs the toolchain rust-toolchain.toml pins if the host
    # does not have it yet.
    # shellcheck disable=SC2029 # the engine is expanded here on purpose
    ssh "$linux_host" "set -e; . ~/.cargo/env; cd neoteletext-browser-check; rustup toolchain install
        export CARGO_NET_GIT_FETCH_WITH_CLI=true GIT_CONFIG_COUNT=1
        export GIT_CONFIG_KEY_0=url.file://\$HOME/neoteletext-browser-check-engine.git.insteadOf GIT_CONFIG_VALUE_0=$engine
        (cd native && cargo clippy --quiet --workspace --all-targets -- -D warnings && cargo test --quiet --workspace \\
            && cargo clippy --quiet --workspace --all-targets --no-default-features -- -D warnings)"
}

check_fuzz() {
    step "native fuzzing: Clippy, then $fuzz_seconds seconds a target after its regressions"
    (
        cd native/fuzz
        cargo "+$FUZZ_NIGHTLY" clippy --quiet --all-targets -- -D warnings
        local target log
        for target in $(cargo "+$FUZZ_NIGHTLY" fuzz list); do
            mkdir -p "corpus/$target" "regressions/$target"
            log="target/fuzz-$target.log"
            mkdir -p target
            cargo "+$FUZZ_NIGHTLY" fuzz run "$target" "corpus/$target" "regressions/$target" -- \
                -max_total_time="$fuzz_seconds" -timeout=10 -rss_limit_mb=2048 >"$log" 2>&1 ||
                { tail -40 "$log"; fail "fuzz target $target found a fault; its input is in native/fuzz/artifacts/$target/"; }
            printf '%s: %s\n' "$target" "$(grep '^Done' "$log")"
        done
    )
}

full_gate() {
    # The commit being pushed is the one checked: the pre-push hook passes
    # the pushed commits, and the tree must be clean at HEAD.
    local pushed
    for pushed in "$@"; do
        [[ "$pushed" == "$(git rev-parse HEAD)" ]] || fail "pushing $pushed, but HEAD is $(git rev-parse HEAD); check out what you push"
    done
    local loose
    loose="$(unstaged_under .; git diff --cached --name-only)"
    [[ -z "$loose" ]] || fail "the tree has changes that are not committed; the full gate checks the commit being pushed:
$loose"

    check_native
    step "native: documentation tests"
    (cd native && cargo test --quiet --workspace --doc)
    step "native: release tests"
    (cd native && cargo nextest run --workspace --release --hide-progress-bar --status-level fail --final-status-level fail)
    step "native: tests with hyperlinks compiled out"
    (cd native && cargo nextest run --workspace --no-default-features --hide-progress-bar --status-level fail --final-status-level fail)
    step "native: licences, advisories and sources"
    (cd native && cargo deny check)
    step "native: no unused dependencies"
    (cd native && cargo machete)
    check_fuzz

    check_linux

    step "the gate changed nothing that is committed"
    loose="$(git status --porcelain --untracked-files=no)"
    [[ -z "$loose" ]] || fail "running the gate changed committed files, so what passed is not what is committed:
$loose"
}

case "$mode" in
commit) commit_gate ;;
full)
    shift
    full_gate "$@"
    ;;
*) fail "usage: scripts/check.sh commit | full [pushed commit]..." ;;
esac

step "All checks passed ($mode)"

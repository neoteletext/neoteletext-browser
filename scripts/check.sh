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
#                             unused dependencies, fuzzing, the
#                             WebAssembly and frontend builds, and the
#                             Linux build and tests on $T42_LINUX_HOST
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-}"
# The Linux host the Wayland window host and the Linux sandbox are checked
# on: the fastest of the Linux test machines (20 cores, against oldvga's 4).
linux_host="${T42_LINUX_HOST:-foundintelquiet4060}"
# Seconds each fuzz target runs in the full gate, after replaying every
# input that has ever found a fault (native/fuzz/regressions/).
fuzz_seconds="${T42_FUZZ_SECONDS:-20}"

step() { printf '\n== %s\n' "$*"; }
fail() {
    printf '\nFAILED: %s\n' "$*" >&2
    exit 1
}

. scripts/tools.sh
out_of_date="$(tools_out_of_date)"
[[ -z "$out_of_date" ]] || fail "tools missing or at another version (run scripts/tools.sh):
$out_of_date"

# Crates that build only on Linux: they need the Wayland libraries.
readonly LINUX_ONLY=(prototypes/spikes/display-host)
# Every prototype crate: each folder of these with a Cargo.toml.
PROTOTYPE_CRATES=()
for crate in prototypes/src-tauri prototypes/standin prototypes/spikes/*/ prototypes/spikes/*/src-tauri; do
    [[ -f "${crate%/}/Cargo.toml" ]] && PROTOTYPE_CRATES+=("${crate%/}")
done
readonly PROTOTYPE_CRATES
# The crates that read input from outside their process, each with the
# same no-panic lint list at its root.
readonly NO_PANIC_ROOTS=(native/protocol/src/lib.rs native/decoder/src/lib.rs native/decoder/src/main.rs native/fetcher/src/main.rs
    native/core/src/lib.rs native/core/src/main.rs native/interface/src/lib.rs native/interface/src/main.rs)

is_linux_only() {
    local crate="$1" only
    for only in "${LINUX_ONLY[@]}"; do
        [[ "$crate" == "$only" ]] && return 0
    done
    return 1
}

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
    step "native: formatting"
    (cd native && cargo fmt --check)
    check_no_panic_lists
    step "native: Clippy, warnings as errors"
    (cd native && cargo clippy --quiet --workspace --all-targets -- -D warnings)
    step "native: tests"
    (cd native && cargo nextest run --workspace --hide-progress-bar --status-level fail --final-status-level fail)
}

check_prototype() {
    local crate="$1"
    if [[ "$(uname -s)" != Linux ]] && is_linux_only "$crate"; then
        step "prototype $crate: Linux only, checked on $linux_host in the full gate"
        return
    fi
    step "prototype $crate: Clippy and tests"
    cargo clippy --quiet --manifest-path "$crate/Cargo.toml" --workspace --all-targets -- -D warnings
    cargo test --quiet --manifest-path "$crate/Cargo.toml" --workspace
}

check_typescript() {
    step "prototypes: TypeScript lint and types"
    (cd prototypes && npx eslint . --max-warnings 0 && npx tsc -b)
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

    local native=no typescript=no everything=no path crate
    local prototypes=()
    while IFS= read -r path; do
        case "$path" in
        native/*) native=yes ;;
        scripts/* | .githooks/* | rust-toolchain.toml) everything=yes ;;
        prototypes/*)
            local in_crate=no
            for crate in "${PROTOTYPE_CRATES[@]}"; do
                if [[ "$path" == "$crate/"* ]]; then
                    in_crate=yes
                    [[ " ${prototypes[*]-} " == *" $crate "* ]] || prototypes+=("$crate")
                fi
            done
            # Whatever is not in a Rust crate belongs to the TypeScript build.
            [[ "$in_crate" == yes ]] || typescript=yes
            ;;
        esac
    done <<<"$staged"
    if [[ "$everything" == yes ]]; then
        native=yes
        typescript=yes
        prototypes=()
        prototypes=("${PROTOTYPE_CRATES[@]}")
    fi

    # What is checked must be what is committed: no unstaged edit or
    # untracked file may sit in a part being checked.
    local scope=()
    [[ "$everything" == yes ]] && scope=(.)
    [[ "$native" == yes ]] && scope+=(native)
    [[ "$typescript" == yes ]] && scope+=(prototypes)
    scope+=("${prototypes[@]+"${prototypes[@]}"}")
    if [[ ${#scope[@]} -gt 0 ]]; then
        local loose
        loose="$(unstaged_under "${scope[@]}")"
        [[ -z "$loose" ]] || fail "changes that are not staged would make this check test something other than the commit; stage or stash them:
$loose"
    fi

    [[ "$native" == yes ]] && check_native
    [[ "$typescript" == yes ]] && check_typescript
    for crate in "${prototypes[@]+"${prototypes[@]}"}"; do
        check_prototype "$crate"
    done
    return 0
}

check_linux() {
    step "Linux on $linux_host: native and the Linux-only prototypes, Clippy and tests"
    ssh -o BatchMode=yes -o ConnectTimeout=10 "$linux_host" true || fail "$linux_host is unreachable; the full gate needs it"
    rsync -a --delete --exclude target --exclude node_modules --exclude tools --exclude .git \
        --exclude native/fuzz/corpus --exclude native/fuzz/artifacts ./ "$linux_host:t42-browser-check/"
    local linux_only="${LINUX_ONLY[*]}"
    # shellcheck disable=SC2029 # the crate list is expanded here on purpose
    # rustup installs the toolchain rust-toolchain.toml pins if the host
    # does not have it yet.
    ssh "$linux_host" "set -e; . ~/.cargo/env; cd t42-browser-check; rustup toolchain install
        (cd native && cargo clippy --quiet --workspace --all-targets -- -D warnings && cargo test --quiet --workspace)
        for crate in $linux_only; do
            cargo clippy --quiet --manifest-path \$crate/Cargo.toml --workspace --all-targets -- -D warnings
            cargo test --quiet --manifest-path \$crate/Cargo.toml --workspace
        done"
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
    step "native: licences, advisories and sources"
    (cd native && cargo deny check)
    step "native: no unused dependencies"
    (cd native && cargo machete)
    check_fuzz

    check_typescript
    local crate
    for crate in "${PROTOTYPE_CRATES[@]}"; do
        check_prototype "$crate"
    done
    step "prototypes: egui-image Clippy for WebAssembly"
    cargo clippy --quiet --manifest-path prototypes/egui-image/Cargo.toml --target wasm32-unknown-unknown --all-targets -- -D warnings
    step "prototypes: WebAssembly and frontend builds"
    (cd prototypes && npm run --silent wasm && npm run --silent build)

    check_linux
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

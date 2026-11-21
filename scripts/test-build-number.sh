#!/usr/bin/env bash
# The tests of the development build's compile count and of
# `neoteletext --version` that have to run Cargo (REQ-7047, TODO-7049).
#
#   scripts/test-build-number.sh [COMMIT]
#
# They build the trusted core's crate in a git worktree of COMMIT (HEAD by
# default), so that the checkout's own count, native/build-number, is never
# read or written and no build waits on the checkout's build lock: a test
# under Cargo that ran Cargo would wait on the lock of the run it is part
# of. The worktree and its target folder are in BUILD_NUMBER_TEST_DIR
# (tmp/build-number-test by default, which git ignores); the target folder
# is kept between runs, so only the first run builds the dependencies. The
# worktree is removed when the script ends.
#
# The program is run only as `neoteletext --version`, with no display and
# no session bus, and with HOME and every XDG folder in the scratch folder,
# so that a program that does not take the flag (the code before REQ-7047)
# finds no display, no settings and no running browser to hand over to;
# whatever it starts is ended when RUN_SECONDS have passed.
#
# Each test prints ok, FAILED or ignored with its reason; the script fails
# when any test failed.
set -uo pipefail
cd "$(dirname "$0")/.."
# A hook run in a linked worktree inherits these, which would point every
# git command here at the checkout that started it.
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE

commit="${1:-HEAD}"
scratch="${BUILD_NUMBER_TEST_DIR:-$PWD/tmp/build-number-test}"
mkdir -p "$scratch"
scratch="$(cd "$scratch" && pwd)"
checkout="$scratch/checkout"
target="$scratch/target"
home="$scratch/home"
program="$target/debug/neoteletext"
count_file="$checkout/native/build-number"
# The longest a `neoteletext --version` may take before what it started is
# ended: the flag answers in milliseconds, and a broker started instead is
# not waited for.
readonly RUN_SECONDS=10
# The largest third number a version holds (`update::version::Version`),
# after which the count starts again at 1 (REQ-7047 item 3).
readonly LARGEST_COUNT=65535

if [[ -e "$checkout" ]]; then
    git worktree remove --force "$checkout" 2>/dev/null || rm -rf -- "$checkout"
    git worktree prune
fi
git worktree add --quiet --detach "$checkout" "$commit" || { echo "cannot make a worktree of $commit in $checkout" >&2; exit 1; }
cleanup() {
    git worktree remove --force "$checkout" 2>/dev/null
    git worktree prune
}
trap cleanup EXIT

# The workspace version's first two numbers, which a development version
# starts with (REQ-7047 item 1).
workspace_version="$(awk '/^\[workspace\.package\]/ { section = 1; next } /^\[/ { section = 0 } section && /^version/ { gsub(/"/, "", $3); print $3 }' "$checkout/native/Cargo.toml")"
major_minor="${workspace_version%.*}"

failed=0
passed=0
ignored=0
log="$scratch/log"
mkdir -p "$log"

# Builds the trusted core's program in the worktree; Cargo's output is in
# $log/build. Extra arguments are environment assignments.
build() {
    (cd "$checkout/native" && env "$@" CARGO_TARGET_DIR="$target" cargo build --locked -p neoteletext --bin neoteletext) >"$log/build" 2>&1
}

# Runs the program with the arguments given, apart from the owner's
# session: stdout in $log/stdout, stderr in $log/stderr, its status
# returned. Anything it started is ended after RUN_SECONDS.
run_apart() {
    rm -rf -- "$home"
    mkdir -p "$home/config" "$home/data" "$home/cache" "$home/state" "$home/runtime" "$home/tmp"
    chmod 700 "$home/runtime"
    local status=0
    env -i PATH="$PATH" HOME="$home" XDG_CONFIG_HOME="$home/config" XDG_DATA_HOME="$home/data" XDG_CACHE_HOME="$home/cache" \
        XDG_STATE_HOME="$home/state" XDG_RUNTIME_DIR="$home/runtime" TMPDIR="$home/tmp" \
        timeout --kill-after=2 "$RUN_SECONDS" "$@" >"$log/stdout" 2>"$log/stderr" </dev/null || status=$?
    # A confined process may have left timeout's process group.
    pkill -KILL -f "^$program" 2>/dev/null
    return "$status"
}

# The version `neoteletext --version` prints, or nothing.
printed_version() {
    run_apart "$program" --version || return 1
    local line
    line="$(cat "$log/stdout")"
    [[ "$line" =~ ^neoteletext\ ([0-9]+\.[0-9]+\.[0-9]+)$ ]] || return 1
    printf '%s\n' "${BASH_REMATCH[1]}"
}

# Runs one test: its name, then the function that checks it, which says
# why on failure.
run_test() {
    local name="$1" why
    shift
    if why="$("$@" 2>&1)"; then
        printf 'test %s ... ok\n' "$name"
        passed=$((passed + 1))
    else
        printf 'test %s ... FAILED\n%s\n' "$name" "$why" | sed '2,$s/^/    /'
        failed=$((failed + 1))
    fi
}

ignore_test() {
    printf 'test %s ... ignored, %s\n' "$1" "$2"
    ignored=$((ignored + 1))
}

# REQ-7047 verification (a) and (d)'s missing file: the first compile in a
# checkout is counted 1, each later one with no source change one higher,
# the version is the workspace's first two numbers and the count, and the
# count file holds the latest.
two_compiles_count_one_apart() {
    rm -f -- "$count_file"
    build || { cat "$log/build"; return 1; }
    local first second
    first="$(printed_version)" || { echo "--version did not print a version:"; cat "$log/stdout" "$log/stderr"; return 1; }
    [[ "$first" == "$major_minor.1" ]] || { echo "the first compile with no count file is $first, not $major_minor.1"; return 1; }
    [[ "$(cat "$count_file" 2>/dev/null)" == 1 ]] || { echo "the count file does not hold 1 after the first compile"; return 1; }
    build || { cat "$log/build"; return 1; }
    second="$(printed_version)" || { echo "--version did not print a version"; return 1; }
    [[ "$second" == "$major_minor.2" ]] || { echo "the second compile is $second, not $major_minor.2"; return 1; }
    [[ "$(cat "$count_file")" == 2 ]] || { echo "the count file holds $(cat "$count_file"), not 2"; return 1; }
}

# REQ-7047 verification (b): a compile changes no tracked file and leaves
# no file git lists, the count file included, which git ignores.
a_compile_changes_no_tracked_file() {
    printf '4\n' >"$count_file"
    build || { cat "$log/build"; return 1; }
    [[ "$(cat "$count_file")" == 5 ]] || { echo "the build did not count into native/build-number"; return 1; }
    git -C "$checkout" check-ignore --quiet native/build-number || { echo "git does not ignore native/build-number"; return 1; }
    local listed
    listed="$(git -C "$checkout" status --porcelain)"
    [[ -z "$listed" ]] || { echo "git status lists files after a build:"; echo "$listed"; return 1; }
}

# REQ-7047 verification (d): a count file that is not a count fails the
# build with a message naming the file and saying to delete it, and is
# left as it was.
refused() {
    local contents="$1"
    printf '%s' "$contents" >"$count_file"
    if build; then
        echo "a count file holding $(printf '%q' "$contents") was taken; the build passed"
        return 1
    fi
    grep -qF "$count_file is not a build count; delete it" "$log/build" || { echo "the build failed without naming the file:"; cat "$log/build"; return 1; }
    cmp -s "$count_file" <(printf '%s' "$contents") || { echo "the refused count file was changed"; return 1; }
}
letters_are_not_a_count() { refused $'twelve\n'; }
a_number_above_the_largest_is_not_a_count() { refused "$((LARGEST_COUNT + 1))"$'\n'; }
more_than_six_bytes_are_not_a_count() { refused $'000012\n'; }

# REQ-7047 verification (e): after the largest count the next compile is
# counted 1, and Cargo warns of it.
the_count_starts_again_after_the_largest() {
    printf '%s\n' "$LARGEST_COUNT" >"$count_file"
    build || { cat "$log/build"; return 1; }
    grep -qF "the build count passed $LARGEST_COUNT, the largest a version holds, and starts again at 1" "$log/build" || { echo "Cargo gave no warning:"; cat "$log/build"; return 1; }
    local version
    version="$(printed_version)" || { echo "--version did not print a version"; return 1; }
    [[ "$version" == "$major_minor.1" ]] || { echo "the compile after $LARGEST_COUNT is $version"; return 1; }
    [[ "$(cat "$count_file")" == 1 ]] || { echo "the count file holds $(cat "$count_file"), not 1"; return 1; }
}

# REQ-7047 verification (c), item 6: a release build is the version named,
# reads and writes no count, and a second one recompiles nothing.
a_release_build_is_the_named_version_and_counts_nothing() {
    printf '9\n' >"$count_file"
    build NEOTELETEXT_RELEASE_VERSION="$workspace_version" || { cat "$log/build"; return 1; }
    run_apart "$program" --version || { echo "--version failed:"; cat "$log/stdout" "$log/stderr"; return 1; }
    [[ "$(cat "$log/stdout")" == "neoteletext $workspace_version" ]] || { echo "a release of $workspace_version printed:"; cat "$log/stdout"; return 1; }
    [[ "$(cat "$count_file")" == 9 ]] || { echo "a release build changed the count file to $(cat "$count_file")"; return 1; }
    build NEOTELETEXT_RELEASE_VERSION="$workspace_version" || { cat "$log/build"; return 1; }
    if grep -E '^ +Compiling ' "$log/build"; then
        echo "a second release build recompiled the above"
        return 1
    fi
    [[ "$(cat "$count_file")" == 9 ]] || { echo "a second release build changed the count file"; return 1; }
}

# REQ-7047 verification (f) and item 4: `neoteletext --version` alone
# prints the program's name, its version and a line end on standard
# output and succeeds, with no display, starting no process or thread and
# connecting to nothing (a window needs a connection to the display).
the_version_flag_prints_the_version_and_starts_nothing() {
    printf '20\n' >"$count_file"
    build || { cat "$log/build"; return 1; }
    local trace="$log/trace"
    rm -f -- "$trace"
    run_apart strace -f -qq -o "$trace" -e trace=execve,clone,clone3,fork,vfork,connect "$program" --version || { echo "--version failed:"; cat "$log/stdout" "$log/stderr"; return 1; }
    cmp -s "$log/stdout" <(printf 'neoteletext %s.21\n' "$major_minor") || { echo "--version printed:"; od -c "$log/stdout" | head; return 1; }
    local calls
    calls="$(grep -vE '^[0-9]+ +execve\("'"$program"'"' "$trace" | grep -E 'execve|clone|fork|connect')"
    [[ -z "$calls" ]] || { echo "--version started or connected to something:"; echo "$calls"; return 1; }
}

run_test two_compiles_count_one_apart two_compiles_count_one_apart
run_test a_compile_changes_no_tracked_file a_compile_changes_no_tracked_file
run_test letters_are_not_a_count letters_are_not_a_count
run_test a_number_above_the_largest_is_not_a_count a_number_above_the_largest_is_not_a_count
run_test more_than_six_bytes_are_not_a_count more_than_six_bytes_are_not_a_count
run_test the_count_starts_again_after_the_largest the_count_starts_again_after_the_largest
run_test a_release_build_is_the_named_version_and_counts_nothing a_release_build_is_the_named_version_and_counts_nothing
run_test the_version_flag_prints_the_version_and_starts_nothing the_version_flag_prints_the_version_and_starts_nothing
# Running `--version extra` starts the browser, which no test may do; the
# flag is read inside main(), where no test can reach it.
ignore_test version_with_another_argument_is_not_the_flag "untested: main() reads the flag inline, so only running the browser could show it; needs the argument check moved into a function a unit test can call"

printf '\n%d passed, %d failed, %d ignored (commit %s)\n' "$passed" "$failed" "$ignored" "$(git -C "$checkout" rev-parse --short HEAD)"
[[ "$failed" -eq 0 ]]

#!/usr/bin/env bash
# The project's one check script (requirement 14.1). Every step either
# passes or fails the run: nothing is skipped, a missing tool or an
# unreachable host is a failure.
#   scripts/check.sh commit   the pre-commit gate: the code compiles clean
#                             (the text checks, formatting and Clippy),
#                             refused while the parts checked have changes
#                             that are not staged, so what is checked is
#                             what is committed
#   scripts/check.sh push     the pre-push gate of any branch but main, on
#                             a clean tree at the commit pushed: commit,
#                             plus Clippy in every feature shape and the
#                             tests
#   scripts/check.sh full     the pre-push gate of main, the long
#                             integration step: push, plus the engine
#                             check, release and feature-off tests,
#                             licences and advisories, unused dependencies
#                             and fuzzing, with the Linux build and tests
#                             on $NEOTELETEXT_LINUX_HOST alongside
# Each tier says how long its last pass took before it starts, and skips a
# lower tier that already passed on the same tree (DEVELOPMENT.md
# section 9).
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-}"
# The Linux host the browser is built and tested on: the fastest of the
# Linux test machines (20 cores, against oldvga's 4).
linux_host="${NEOTELETEXT_LINUX_HOST:-foundintelquiet4060}"
# Seconds each fuzz target runs in the full gate, after replaying every
# input that has ever found a fault (native/fuzz/regressions/).
fuzz_seconds="${NEOTELETEXT_FUZZ_SECONDS:-20}"

# Timing and stamps: each step prints its time as it ends; a pass records
# its steps' times, for the next run's warning, and the tree it passed on,
# so a higher tier does not run it again.
git_dir="$(git rev-parse --git-common-dir)"
step_name=""
step_start=$SECONDS
step_times=""
minutes() { printf '%dm%02ds' $(($1 / 60)) $(($1 % 60)); }
end_step() {
    [[ -n "$step_name" ]] || return 0
    local took=$((SECONDS - step_start))
    printf '   (%s)\n' "$(minutes "$took")"
    step_times+="$took	$step_name"$'\n'
    step_name=""
}
step() {
    end_step
    step_name="$*"
    step_start=$SECONDS
    printf '\n== %s\n' "$*"
}
announce() {
    local file="$git_dir/gate-times-$1"
    if [[ -f "$file" ]]; then
        printf '== the %s gate: its last pass took %s; its slowest steps:\n' "$1" "$(minutes "$(head -1 "$file")")"
        tail -n +2 "$file" | sort -rn | head -3 | while IFS=$'\t' read -r took name; do
            printf '   %s  %s\n' "$(minutes "$took")" "$name"
        done
    else
        printf '== the %s gate: no pass measured yet\n' "$1"
    fi
}
# What the gates read: a stamp is keyed on these paths alone, so a change
# elsewhere, such as a document, does not run a gate again.
readonly CHECKED_PATHS=(native scripts .githooks rust-toolchain.toml)
checked_tree() {
    git ls-tree "$1" -- "${CHECKED_PATHS[@]}" | git hash-object --stdin
}
passed() {
    local tree="$2"
    [[ -n "$tree" && -f "$git_dir/gate-passed-$1" && "$(cat "$git_dir/gate-passed-$1")" == "$tree" ]]
}
stamp() {
    [[ -z "$2" ]] || printf '%s\n' "$2" >"$git_dir/gate-passed-$1"
}
record_pass() {
    local tier="$1" tree="$2" took=$((SECONDS - gate_start))
    end_step
    printf '%s\n%s' "$took" "$step_times" >"$git_dir/gate-times-$tier"
    stamp "$tier" "$tree"
    printf '\nthe %s gate passed in %s\n' "$tier" "$(minutes "$took")"
}
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
readonly NO_PANIC_ROOTS=(native/protocol/src/lib.rs native/decoder/src/lib.rs native/decoder/src/main.rs native/fetcher/src/main.rs native/fetcher/src/lib.rs
    native/core/src/lib.rs native/core/src/main.rs native/interface/src/lib.rs native/interface/src/main.rs native/interface-core/src/lib.rs native/draw-vello/src/lib.rs native/draw-embedded/src/lib.rs
    native/web-opener/src/lib.rs native/dock-menu/src/lib.rs
    native/keyboard/src/main.rs native/sound/src/lib.rs native/sound/src/main.rs native/confine/src/lib.rs
    native/update/src/lib.rs native/icon/src/lib.rs native/launcher/src/lib.rs native/launcher/src/main.rs native/os-install/src/lib.rs native/installer/src/main.rs
    native/tuf/updater/src/lib.rs native/tuf/updater/src/main.rs native/tuf/release/src/main.rs native/tuf/release/src/bin/test_host.rs)

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

# Requirement 2.5: every workspace depends on the engine at one commit.
check_engine_pin() {
    step "the engine is pinned at one commit in every workspace"
    local revs
    revs="$(sed -n 's/^neoteletext-[a-z-]* = { git = "[^"]*", rev = "\([0-9a-f]*\)".*/\1/p' native/Cargo.toml native/tuf/Cargo.toml native/fuzz/Cargo.toml | sort -u)"
    [[ -n "$revs" && "$revs" != *$'\n'* ]] || fail "the workspaces must name one engine commit, found: ${revs:-none}"
}

# The programs a release script builds together, as it names them: the
# macOS and Linux scripts' shell lists and the Windows script's array.
release_programs() {
    case "$1" in
    *.ps1) sed -n 's/^\$programs = @(\(.*\))$/\1/p' "$1" | tr -d '",' ;;
    *build-macos.sh) sed -n 's/^programs=(\(.*\))$/\1/p' "$1" ;;
    *build-linux.sh) sed -n 's/^programs="\(.*\)"$/\1/p' "$1" ;;
    esac
}

# The features of the engine's audio crate in a build of `$@`'s packages
# with no tests, as Cargo resolves them: a development dependency's
# features (the decoder's tests write sounds with `encode`, which brings
# `ima`) stay out of such a build, as `-e normal,build` reproduces.
audio_features() {
    (cd native && cargo tree --quiet --locked -e normal,build --target all "$@" --prefix none -f '{p}|{f}') | sed -n 's/^neoteletext-audio [^|]*|//p' | sed 's/ (\*)$//' | sort -u
}

# Decision 121, the owner's instruction of 28 October 2026 and the engine's
# decision E.131: PCM16 and IMA ADPCM are compiled out of the desktop
# builds (Windows, macOS, Linux). Every release script's programs, built
# together as the script builds them, and the Windows installer, built on
# its own, have the engine's audio crate with neither codec.
check_desktop_codecs() {
    step "the shipped programs are built without PCM16 and IMA ADPCM"
    local script programs program features
    local -a arguments
    for script in scripts/release/build-macos.sh scripts/release/build-linux.sh scripts/release/build-windows.ps1; do
        programs="$(release_programs "$script")"
        [[ -n "$programs" ]] || fail "no list of programs found in $script"
        arguments=()
        for program in $programs; do arguments+=(-p "$program"); done
        features="$(audio_features "${arguments[@]}")"
        [[ -n "$features" ]] || fail "$script's programs do not build neoteletext-audio, so its codecs cannot be checked"
        if grep -Eq '(^|,)(ima|pcm16)(,|$)' <<<"$features"; then
            fail "$script's programs build neoteletext-audio with PCM16 or IMA ADPCM ($features); the desktop builds have neither"
        fi
    done
    features="$(audio_features -p neoteletext-installer)"
    if grep -Eq '(^|,)(ima|pcm16)(,|$)' <<<"$features"; then
        fail "the Windows installer builds neoteletext-audio with PCM16 or IMA ADPCM ($features)"
    fi
}

# Decision 123: a release is built only at the workspace's version, which
# the fetcher's user agent sends, so the version on the tab strip is the
# one sent. The macOS and Linux scripts are run with another version, and
# refuse it before they build anything; the Windows script, which runs
# only in the virtual machine, is read for its check.
check_release_versions() {
    step "the release scripts build only the workspace's version"
    local script output
    for script in scripts/release/build-macos.sh scripts/release/build-linux.sh; do
        if output="$(env -u NEOTELETEXT_RELEASE_IDENTITY bash "$script" 0.0.0-not-the-workspace /nonexistent 2>&1)"; then
            fail "$script built a version other than the workspace's"
        fi
        [[ "$output" == *"is not the workspace's"* ]] || fail "$script did not refuse a version other than the workspace's: $output"
    done
    grep -q 'if (\$Version -ne \$workspaceVersion)' scripts/release/build-windows.ps1 || fail "scripts/release/build-windows.ps1 does not refuse a version other than the workspace's"
}

# What the commit gate checks: the code compiles clean.
check_compiles() {
    check_engine_pin
    check_desktop_codecs
    check_release_versions
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
    # The updater's workspace (native/tuf: the updater and the release
    # tool, with `tough`), apart so its dependencies' features never reach
    # the browser's crates (installation-and-updates.md section 14).
    step "native/tuf: lockfiles up to date, formatting"
    (cd native/tuf && cargo metadata --quiet --locked --format-version 1 >/dev/null && cargo fmt --check)
    (cd native/tuf/fuzz && cargo metadata --quiet --locked --format-version 1 >/dev/null)
    step "native/tuf: Clippy, warnings as errors"
    (cd native/tuf && cargo clippy --quiet --workspace --all-targets -- -D warnings)
}

# What the push gate adds: every feature shape, and the tests.
check_shapes_and_tests() {
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
    # The phone keyboard alone (it brings typing with it): an unused test
    # import in that shape once slipped past the shapes above.
    step "native: Clippy with only: phone-keyboard"
    (cd native && cargo clippy --quiet -p interface-core -p protocol -p neoteletext -p neoteletext-interface -p neoteletext-keyboard --all-targets \
        --no-default-features --features "interface-core/phone-keyboard,protocol/phone-keyboard,neoteletext/phone-keyboard,neoteletext-interface/phone-keyboard,neoteletext-keyboard/phone-keyboard" -- -D warnings)
    step "native: tests"
    (cd native && cargo nextest run --workspace --hide-progress-bar --status-level fail --final-status-level fail)
    step "native/tuf: tests"
    (cd native/tuf && cargo nextest run --workspace --hide-progress-bar --status-level fail --final-status-level fail)
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
        native/* | scripts/* | .githooks/* | rust-toolchain.toml) native=yes ;; # CHECKED_PATHS
        esac
    done <<<"$staged"

    # What is checked must be what is committed: no unstaged edit or
    # untracked file may sit in a part being checked.
    if [[ "$native" == yes ]]; then
        local loose
        loose="$(unstaged_under "${CHECKED_PATHS[@]}")"
        [[ -z "$loose" ]] || fail "changes that are not staged would make this check test something other than the commit; stage or stash them:
$loose"
        tree="$(checked_tree "$(git write-tree)")"
        if passed commit "$tree"; then
            printf '\n== the commit gate already passed on this tree\n'
            tree=""
        else
            check_compiles
        fi
    fi
    return 0
}

# Run alongside the steps here; its output is shown when they are done.
check_linux() {
    step_name=""
    step "Linux on $linux_host: native, Clippy and tests"
    ssh -o BatchMode=yes -o ConnectTimeout=10 "$linux_host" true || fail "$linux_host is unreachable; the full gate needs it"
    # Files are compared by content and written with the host's own time:
    # with this machine's times kept, a file edited here before the host's
    # last build looks older than that build, and Cargo reuses it.
    rsync -a --no-times --checksum --delete --exclude target --exclude tools --exclude .git \
        --exclude native/fuzz/corpus --exclude native/fuzz/artifacts --exclude native/tuf/fuzz/corpus --exclude native/tuf/fuzz/artifacts \
        ./ "$linux_host:neoteletext-browser-check/"
    # The host fetches the engine from its published repository, as Cargo
    # does here. rustup installs the toolchain rust-toolchain.toml pins if
    # the host does not have it yet.
    ssh "$linux_host" "set -e; . ~/.cargo/env; cd neoteletext-browser-check; rustup toolchain install
        (cd native && cargo clippy --quiet --workspace --all-targets -- -D warnings && cargo test --quiet --workspace \\
            && cargo clippy --quiet --workspace --all-targets --no-default-features -- -D warnings)
        (cd native/tuf && cargo clippy --quiet --workspace --all-targets -- -D warnings && cargo test --quiet --workspace)"
    end_step
}

check_fuzz() {
    # The browser's fuzz targets, and the updater workspace's (The Update
    # Framework's metadata through `tough`, the update host's responses).
    local folder
    for folder in native/fuzz native/tuf/fuzz; do
        step "$folder: Clippy, then $fuzz_seconds seconds a target after its regressions"
        (
            cd "$folder"
            cargo "+$FUZZ_NIGHTLY" clippy --quiet --all-targets -- -D warnings
            local target log
            for target in $(cargo "+$FUZZ_NIGHTLY" fuzz list); do
                mkdir -p "corpus/$target" "regressions/$target"
                log="target/fuzz-$target.log"
                mkdir -p target
                cargo "+$FUZZ_NIGHTLY" fuzz run "$target" "corpus/$target" "regressions/$target" -- \
                    -max_total_time="$fuzz_seconds" -timeout=10 -rss_limit_mb=2048 >"$log" 2>&1 ||
                    { tail -40 "$log"; fail "fuzz target $target found a fault; its input is in $folder/artifacts/$target/"; }
                printf '%s: %s\n' "$target" "$(grep '^Done' "$log")"
            done
        )
    done
}

# The browser builds the engine with its lints capped, so the engine's own
# forbid(unsafe_code) is held only by the engine's gate (finding F-88):
# the source at the pinned commit is read here, and the word `unsafe` in
# any line's code, outside its string literals and its comment, fails the
# gate. A string is a word the code looks for, as the engine's own
# source checks look for "unsafe", not code that is unsafe.
check_engine_safe() {
    step "the engine at its pinned commit has no unsafe code"
    local engine revision found line code hits="" status=0
    engine="$(sed -n 's/^neoteletext-[a-z-]* = { git = "\([^"]*\)".*/\1/p' native/Cargo.toml | sort -u)"
    revision="$(sed -n 's/^neoteletext-[a-z-]* = { git = "[^"]*", rev = "\([0-9a-f]*\)".*/\1/p' native/Cargo.toml | sort -u)"
    [[ -n "$engine" && "$engine" != *$'\n'* ]] || fail "native/Cargo.toml must name one engine repository, found: ${engine:-none}"
    [[ -n "$revision" && "$revision" != *$'\n'* ]] || fail "native/Cargo.toml must name one engine commit, found: ${revision:-none}"
    # The source read is the commit as the repository Cargo builds from
    # publishes it, fetched into a copy the gate keeps between runs. A hook
    # runs with git's GIT_DIR set to this repository, absolute in a
    # worktree, which git would follow instead of the copy's.
    local copy="native/target/engine.git"
    engine_git() { env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE git --git-dir "$copy" "$@"; }
    [[ -d "$copy" ]] || env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE git init --quiet --bare "$copy"
    engine_git cat-file -e "$revision^{commit}" 2>/dev/null || engine_git fetch --quiet "$engine" "$revision" ||
        fail "the engine repository $engine has no commit $revision"
    found="$(engine_git grep --line-number -I --word-regexp -e unsafe "$revision" -- '*.rs')" || status=$?
    # 1 is no line found.
    ((status <= 1)) || fail "reading the engine at $revision failed"
    # git grep names each line as commit:path:line:text.
    while IFS= read -r line; do
        [[ -n "$line" ]] || continue
        code="${line#*:*:*:}"
        # A character literal of a quote first, so it cannot open a string.
        code="$(sed -E -e "s/'\\\\?\"'//g" -e 's/"([^"\\]|\\.)*"//g' <<<"$code")"
        code="${code%%//*}"
        if [[ "$code" =~ (^|[^A-Za-z0-9_])unsafe([^A-Za-z0-9_]|$) ]]; then
            hits="$hits${line#*:}"$'\n'
        fi
    done <<<"$found"
    [[ -z "$hits" ]] || fail "the engine at $revision has unsafe code:
$hits"
}

# A release script refuses a test identity unless told it is a test
# (finding F-24). publish.sh, given no destination, copies nothing
# anywhere; with the variable set and no --test it must refuse before it
# reads its arguments, so the arguments here name nothing.
check_release_identity() {
    step "release scripts: a test identity is refused without --test"
    local said
    if said="$(NEOTELETEXT_RELEASE_IDENTITY=/x bash scripts/release/publish.sh r root t s ts -- 2>&1 </dev/null)"; then
        fail "publish.sh ran with a test identity and no --test"
    fi
    [[ "$said" == *"names a test identity"* ]] || fail "publish.sh failed with a test identity, but not by refusing it:
$said"
}

# The commit pushed is the one checked: the pre-push hook passes the
# pushed commits, and the tree must be clean at HEAD.
clean_head() {
    local pushed
    for pushed in "$@"; do
        [[ "$pushed" == "$(git rev-parse HEAD)" ]] || fail "pushing $pushed, but HEAD is $(git rev-parse HEAD); check out what you push"
    done
    local loose
    loose="$(unstaged_under .; git diff --cached --name-only)"
    [[ -z "$loose" ]] || fail "the tree has changes that are not committed; the $mode gate checks the commit being pushed:
$loose"
    tree="$(checked_tree HEAD)"
    if passed "$mode" "$tree"; then
        printf '\n== the %s gate already passed on this tree\n' "$mode"
        exit 0
    fi
}

push_gate() {
    if passed commit "$tree"; then
        printf '\n== the commit gate passed on this tree: skipped\n'
    else
        check_compiles
    fi
    check_shapes_and_tests
}

full_gate() {
    local linux_log="$git_dir/gate-linux.log" linux_pid
    check_linux >"$linux_log" 2>&1 &
    linux_pid=$!
    printf '\n== the Linux steps on %s run alongside, logged in %s\n' "$linux_host" "$linux_log"

    if passed push "$tree"; then
        printf '\n== the push gate passed on this tree: skipped\n'
    else
        push_gate
    fi
    check_engine_safe
    step "native: documentation tests"
    (cd native && cargo test --quiet --workspace --doc)
    step "native: release tests"
    (cd native && cargo nextest run --workspace --release --hide-progress-bar --status-level fail --final-status-level fail)
    step "native: tests with hyperlinks compiled out"
    (cd native && cargo nextest run --workspace --no-default-features --hide-progress-bar --status-level fail --final-status-level fail)
    step "native: licences, advisories and sources"
    (cd native && cargo deny check)
    (cd native/tuf && cargo deny check)
    step "native: no unused dependencies"
    (cd native && cargo machete)
    (cd native/tuf && cargo machete)
    check_release_identity
    step "native/tuf: release tests"
    (cd native/tuf && cargo nextest run --workspace --release --hide-progress-bar --status-level fail --final-status-level fail)
    check_fuzz

    step "waiting for the Linux steps on $linux_host"
    if ! wait "$linux_pid"; then
        cat "$linux_log"
        fail "the Linux steps on $linux_host failed"
    fi
    cat "$linux_log"

    step "the gate changed nothing that is committed"
    local loose
    loose="$(git status --porcelain --untracked-files=no)"
    [[ -z "$loose" ]] || fail "running the gate changed committed files, so what passed is not what is committed:
$loose"
}

tree=""
[[ -n "$mode" ]] && announce "$mode"
gate_start=$SECONDS
case "$mode" in
commit)
    commit_gate
    [[ -z "$tree" ]] || record_pass commit "$tree"
    ;;
push)
    shift
    clean_head "$@"
    push_gate
    record_pass push "$tree"
    stamp commit "$tree"
    ;;
full)
    shift
    clean_head "$@"
    full_gate
    record_pass full "$tree"
    stamp push "$tree"
    stamp commit "$tree"
    ;;
*) fail "usage: scripts/check.sh commit | push | full [pushed commit]..." ;;
esac

step "All checks passed ($mode)"

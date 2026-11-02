# Copied by neoteletext-common gates/sync; change it there, not here.
# neoteletext-common gates at 26c4dd331b6e9903f04dc9b9cbca8e0fac31b1ab
# The gate machinery every neoteletext repository shares (gates.md). A
# repository's scripts/check.sh sets its own steps and sources its copy of
# this file, scripts/gate.sh, which gates/sync writes; the commit gate
# fails when the copy differs from this file at the commit it names.
#
# Before sourcing, check.sh defines:
#   CHECKED_PATHS   the paths the gates read, as git pathspecs; a stamp is
#                   keyed on these alone, so a change elsewhere runs no gate
#   linux_host      the Linux host of the full gate
#   commit_steps    the commit gate's steps: the code compiles clean
# and, where it has them:
#   push_steps      what a push of the work branch adds: shapes and tests
#   full_steps      what a push to main adds, on this machine
#   linux_steps     the full gate's steps on $linux_host, run alongside
# then calls gate_main "$@".

# The folder of neoteletext-common, whose gates/ this file is copied from.
common="${NEOTELETEXT_COMMON:-$HOME/devel/neoteletext-common}"
# The Linux host's copy has no git directory, and records nothing.
git_dir="$(git rev-parse --git-common-dir 2>/dev/null || true)"
# Git in neoteletext-common. A hook in a linked worktree inherits GIT_DIR
# and its neighbours, which would override -C and read the wrong repository.
common_git() { env -u GIT_DIR -u GIT_INDEX_FILE -u GIT_WORK_TREE git -C "$common" "$@"; }
# Names of the documents DEC-2022 deleted, as a tracked file might write them:
# bare, or with the dot escaped in a pattern. A longer name that ends in one
# of them is another document. This text cannot match itself, as a closing
# bracket stands where a name would meet its extension.
readonly DELETED_DOCUMENT_NAMES='(^|[^-A-Za-z0-9_])(INPUT|SERVER|CHANNEL)\\*\.md'

fail() {
    printf '\nFAILED: %s\n' "$*" >&2
    exit 1
}

# A step that cannot run fails the gate, naming what it lacks and how to get
# it (gates.md). A step never asks whether its tool is there and goes on
# without it: that gate would pass having checked nothing.
need_tool() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is not installed, so this gate cannot run; $2"
}
need_host() {
    ssh -o BatchMode=yes -o ConnectTimeout=10 "$linux_host" true || fail "$linux_host is unreachable; the full gate needs it"
}
# A tool on the Linux host. A command over ssh has neither folder on its path.
need_host_tool() {
    # shellcheck disable=SC2029 # the tool's name is expanded here on purpose
    ssh -o BatchMode=yes -o ConnectTimeout=10 "$linux_host" "PATH=\$HOME/.cargo/bin:\$HOME/.local/bin:\$PATH command -v $1 >/dev/null 2>&1" ||
        fail "$1 is not installed on $linux_host, so this gate cannot run; $2"
}

# Each step prints its time as it ends; a pass records its steps' times,
# for the next run's warning, and the tree it passed on, so a higher tier
# does not run it again.
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
checked_tree() {
    git ls-tree "$1" -- "${CHECKED_PATHS[@]}" | git hash-object --stdin
}
passed() {
    [[ -n "$2" && -f "$git_dir/gate-passed-$1" && "$(cat "$git_dir/gate-passed-$1")" == "$2" ]]
}
stamp() {
    printf '%s\n' "$2" >"$git_dir/gate-passed-$1"
}
record_pass() {
    local took=$((SECONDS - gate_start))
    end_step
    printf '%s\n%s' "$took" "$step_times" >"$git_dir/gate-times-$1"
    stamp "$1" "$tree"
    printf '\nthe %s gate passed in %s\n' "$1" "$(minutes "$took")"
}
defined() { declare -F "$1" >/dev/null; }

# Paths with changes that are not staged, or files git does not track,
# under any of the given paths.
unstaged_under() {
    git diff --name-only -- "$@"
    git ls-files --others --exclude-standard -- "$@"
}

# No tracked file names a deleted document (DEC-2022): the protocol's text
# is in rdm's items, and a reference to a file that is gone sends a reader
# nowhere. It reads the index, what is being committed, and every path, since
# a document is where such a mention lives and CHECKED_PATHS may not hold it.
# A repository that must keep a mention as history, such as a dated review,
# names that file alone in DELETED_DOCUMENT_HISTORY before sourcing this.
check_deleted_documents() {
    step "no tracked file names a deleted document (DEC-2022)"
    local found status=0 path
    local paths=(.)
    for path in "${DELETED_DOCUMENT_HISTORY[@]+"${DELETED_DOCUMENT_HISTORY[@]}"}"; do
        paths+=(":(exclude,literal)$path")
    done
    found="$(git grep --cached -nIE "$DELETED_DOCUMENT_NAMES" -- "${paths[@]}")" || status=$?
    [[ "$status" -le 1 ]] || fail "git grep failed looking for deleted documents"
    [[ -z "$found" ]] || fail "these name a document that is deleted; cite the item, DOC-n of neoteletext-common's PROTOCOL.html, that holds the rule:
$found"
}

# A check script that can report a step as skipped can pass without running
# it, which gates.md forbids. The word is written in two parts, so that this
# file does not name it.
readonly SKIP_REPORT='SKIP''PED'
check_no_skipping() {
    step "no step of this gate can be skipped (gates.md)"
    local found status=0
    found="$(git grep --cached -nF "$SKIP_REPORT" -- 'scripts/*.sh' ':(exclude)scripts/gate.sh')" || status=$?
    [[ "$status" -le 1 ]] || fail "git grep failed looking for a step that can be skipped"
    [[ -z "$found" ]] || fail "a step that cannot run must fail the gate, with need_tool or need_host, and not report itself skipped:
$found"
}

# The copy of this file and the hooks are neoteletext-common's at the
# commit the copy names on its second line, unedited. neoteletext-common
# itself runs the original.
check_gate_copy() {
    [[ ! -f gates/gate.sh ]] || return 0
    step "the gate machinery is neoteletext-common's, unedited"
    local revision file
    revision="$(sed -n '2s/^# neoteletext-common gates at \([0-9a-f]*\)$/\1/p' scripts/gate.sh)"
    [[ -n "$revision" ]] || fail "scripts/gate.sh names no neoteletext-common commit; run $common/gates/sync"
    [[ -d "$common/.git" ]] || fail "neoteletext-common is needed at $common (or NEOTELETEXT_COMMON) to check the gate's copy"
    tail -n +3 scripts/gate.sh | cmp -s - <(common_git show "$revision:gates/gate.sh") ||
        fail "scripts/gate.sh differs from neoteletext-common's gates/gate.sh at $revision; change it there and run gates/sync"
    for file in pre-commit pre-merge-commit pre-push; do
        cmp -s ".githooks/$file" <(common_git show "$revision:gates/$file") ||
            fail ".githooks/$file differs from neoteletext-common's gates/$file at $revision; run $common/gates/sync"
    done
}

commit_gate() {
    if [[ -z "$(git diff --cached --name-only)" ]]; then
        step "nothing staged"
        return 0
    fi
    check_deleted_documents
    if [[ -z "$(git diff --cached --name-only -- "${CHECKED_PATHS[@]}")" ]]; then
        step "nothing staged that the gates read"
        return 0
    fi
    # What is checked must be what is committed: no unstaged edit or
    # untracked file may sit in a part being checked.
    local loose
    loose="$(unstaged_under "${CHECKED_PATHS[@]}")"
    [[ -z "$loose" ]] || fail "changes that are not staged would make this check test something other than the commit; stage or stash them:
$loose"
    tree="$(checked_tree "$(git write-tree)")"
    if passed commit "$tree"; then
        printf '\n== the commit gate already passed on this tree\n'
        return 0
    fi
    check_gate_copy
    check_no_skipping
    commit_steps
    record_pass commit
}

# The commit pushed is the one checked: the pre-push hook passes the
# pushed commits, and the tree must be clean at HEAD.
clean_head() {
    local pushed loose
    for pushed in "$@"; do
        [[ "$pushed" == "$(git rev-parse HEAD)" ]] || fail "pushing $pushed, but HEAD is $(git rev-parse HEAD); check out what you push"
    done
    loose="$(unstaged_under .; git diff --cached --name-only)"
    [[ -z "$loose" ]] || fail "the tree has changes that are not committed; the $mode gate checks the commit being pushed:
$loose"
    check_deleted_documents
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
        check_gate_copy
        check_no_skipping
        commit_steps
    fi
    if defined push_steps; then push_steps; fi
}

full_gate() {
    local linux_log="$git_dir/gate-linux.log" linux_pid="" loose
    if defined linux_steps; then
        # step_name is cleared so that the background's first step does not
        # time the step running here when it started. Separate commands, as
        # bash ignores set -e inside all but the last of an && list.
        (
            step_name=""
            linux_steps
            end_step
        ) >"$linux_log" 2>&1 &
        linux_pid=$!
        printf '\n== the steps on %s run alongside, logged in %s\n' "$linux_host" "$linux_log"
    fi
    if passed push "$tree"; then
        printf '\n== the push gate passed on this tree: skipped\n'
    else
        push_gate
    fi
    if defined full_steps; then full_steps; fi
    if [[ -n "$linux_pid" ]]; then
        step "waiting for the steps on $linux_host"
        if ! wait "$linux_pid"; then
            cat "$linux_log"
            fail "the steps on $linux_host failed"
        fi
        cat "$linux_log"
    fi
    step "the gate changed nothing that is committed"
    loose="$(git status --porcelain --untracked-files=no)"
    [[ -z "$loose" ]] || fail "running the gate changed committed files, so what passed is not what is committed:
$loose"
}

gate_main() {
    mode="${1:-}"
    tree=""
    case "$mode" in
    commit | push | full) announce "$mode" ;;
    *) fail "usage: scripts/check.sh commit | push | full [pushed commit]..." ;;
    esac
    gate_start=$SECONDS
    shift
    case "$mode" in
    commit) commit_gate ;;
    push)
        clean_head "$@"
        push_gate
        record_pass push
        stamp commit "$tree"
        ;;
    full)
        clean_head "$@"
        full_gate
        record_pass full
        stamp push "$tree"
        stamp commit "$tree"
        ;;
    esac
    step "All checks passed ($mode)"
    step_name=""
}

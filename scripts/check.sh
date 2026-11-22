#!/usr/bin/env bash
# The project's one check script (requirement 14.1): its own steps, on the
# gate machinery every neoteletext repository shares (scripts/gate.sh,
# copied from neoteletext-common; DEVELOPMENT.md section 9 there). Every step either
# passes or fails the run: nothing is skipped, a missing tool or an
# unreachable host is a failure.
#   scripts/check.sh commit   the code compiles clean (the text checks,
#                             formatting and Clippy), on what is staged
#   scripts/check.sh push     Clippy in every feature shape and for the
#                             microcontrollers, the dependency trees, the
#                             advisories and the tests: a push of
#                             livedevelopment
#   scripts/check.sh full     the engine's unsafe-code check and its own
#                             full gate, release and feature-off tests,
#                             licences, unused dependencies and fuzzing,
#                             with the Linux build and tests on
#                             $NEOTELETEXT_LINUX_HOST alongside: a push to
#                             main
# Windows is checked by hand in the virtual machine (DEVELOPMENT.md
# section 12.1, DEC-2358), not by a stage of this script.
set -euo pipefail
cd "$(dirname "$0")/.."

# The Linux host the browser is built and tested on: the fastest of the
# Linux test machines (20 cores, against oldvga's 4).
linux_host="${NEOTELETEXT_LINUX_HOST:-foundintelquiet4060}"
# Seconds each fuzz target runs in the full gate, after replaying every
# input that has ever found a fault (native/fuzz/regressions/).
fuzz_seconds="${NEOTELETEXT_FUZZ_SECONDS:-20}"

# What the gates read.
readonly CHECKED_PATHS=(native scripts .githooks rust-toolchain.toml)

. scripts/gate.sh

. scripts/tools.sh
out_of_date="$(tools_out_of_date)"
[[ -z "$out_of_date" ]] || fail "tools missing or at another version (run scripts/tools.sh):
$out_of_date"

# The crates that read input from outside their process, each with the
# same no-panic lint list at its root.
readonly NO_PANIC_ROOTS=(native/protocol/src/lib.rs native/decoder/src/lib.rs native/decoder/src/main.rs native/fetcher/src/main.rs native/fetcher/src/lib.rs
    native/core/src/lib.rs native/core/src/main.rs native/interface/src/lib.rs native/interface/src/main.rs native/interface-core/src/lib.rs native/draw-raster/src/lib.rs native/draw-embedded/src/lib.rs
    native/web-opener/src/lib.rs native/dock-menu/src/lib.rs native/dbus/src/lib.rs native/secret-store/src/lib.rs native/dialog/src/lib.rs
    native/keyboard/src/main.rs native/sound/src/lib.rs native/sound/src/main.rs native/confine/src/lib.rs native/script/src/lib.rs
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

# Requirement 2.5 (REQ-666), DEVELOPMENT.md section 11 and appendix B
# browser item 4: every workspace depends on the engine by one commit of
# one repository, through every neoteletext-* dependency, never through a
# copy, a working tree or a patch. Each manifest line that names a source
# is read: a git source must be the engine's, in the one form below; a
# path must stay inside this repository; an engine crate without either
# inherits the workspace's line. A form this does not read fails, so that
# a dependency cannot pass by being written differently. The lockfiles are
# read too, since they are what Cargo builds: every git package in them is
# the same commit of the same repository.
check_engine_pin() {
    step "every renderer dependency names its repository at one commit (requirement 2.5)"
    local manifest line target sources="" unread="" outside="" patched="" locked
    # The engine's crates, the ReGIS renderer's (DEC-6335) and the NAPLPS renderer's (DEC-6407).
    local -r engine_line='^(neoteletext-[a-z0-9-]+|regis|regis-fonts|naplps|naplps-fonts) = \{ git = "([^"]+)", rev = "([0-9a-f]{40})"(, [^{}]*)? \}$'
    local root
    root="$(pwd -P)"
    while IFS= read -r manifest; do
        patched+="$(grep -nE '^\[(patch|replace)' "$manifest" | sed "s|^|$manifest:|" || true)"
        while IFS= read -r line; do
            if [[ "$line" =~ $engine_line ]]; then
                sources+="${BASH_REMATCH[2]} ${BASH_REMATCH[3]}"$'\n'
            elif [[ "$line" == *'git = '* || "$line" =~ ^\[.*dependencies\. ]]; then
                unread+="$manifest: $line"$'\n'
            elif [[ "$line" =~ path\ =\ \"([^\"]+)\" ]]; then
                target="$(realpath -m "$(dirname "$manifest")/${BASH_REMATCH[1]}")"
                [[ "$target" == "$root"/* ]] || outside+="$manifest: $line"$'\n'
            elif [[ "$line" =~ ^neoteletext-[a-z0-9-]+\ = && "$line" != *'workspace = true'* ]]; then
                unread+="$manifest: $line"$'\n'
            fi
        done < <(grep -E '^(neoteletext-[a-z0-9-]+ *=|[a-z0-9_-]+ *= *\{.*(git|path) = |\[.*dependencies\.)' "$manifest")
    done < <(git ls-files -- 'native/Cargo.toml' 'native/**/Cargo.toml')
    [[ -z "$patched" ]] || fail "a manifest patches or replaces a dependency, which requirement 2.5 forbids:
$patched"
    [[ -z "$unread" ]] || fail "these name a git source, or an engine crate, in a form this check does not read; write each as neoteletext-<crate> = { git = \"...\", rev = \"<40 hexadecimal digits>\", ... } or with workspace = true:
$unread"
    [[ -z "$outside" ]] || fail "these name a path outside this repository, such as a working tree of the engine:
$outside"
    sources="$(sort -u <<<"$sources" | sed '/^$/d')"
    # One commit of each renderer repository: the engine's, and the ReGIS
    # renderer's where a workspace uses it.
    local repositories allowed="" source
    repositories="$(sed 's/ .*//' <<<"$sources")"
    [[ -n "$sources" && "$(sort -u <<<"$repositories")" == "$repositories" ]] || fail "the workspaces must name each renderer repository at one commit, found: ${sources:-none}"
    while IFS= read -r source; do
        [[ "${source% *}" == */neoteletext-renderer-teletext || "${source% *}" == */neoteletext-renderer-regis || "${source% *}" == */neoteletext-renderer-naplps ]] || fail "the workspaces name a git source that is not the engine, the ReGIS renderer or the NAPLPS renderer: $source"
        allowed+="git+${source% *}?rev=${source#* }#${source#* }"$'\n'
    done <<<"$sources"
    [[ "$repositories" == *neoteletext-renderer-teletext* ]] || fail "the workspaces name no engine repository"
    while IFS= read -r locked; do
        grep -qxF "$locked" <<<"$allowed" || fail "a lockfile builds a git package from $locked, not a renderer at its pinned commit"
    done < <(git ls-files -- 'native/Cargo.lock' 'native/**/Cargo.lock' | xargs sed -n 's/^source = "\(git+[^"]*\)"$/\1/p' | sort -u)
}

# Requirement 3.2 (REQ-668, DEC-2355): the trusted core (the package
# neoteletext, which is also the broker's program) links no engine crate
# but these, each for the bounded use DEC-2355 names, and no web engine,
# script engine or interpreter. An engine crate added to its tree fails
# here until that decision names it.
# neoteletext-rules comes with neoteletext-input, whose RULE_EVENT, PAGE_STATE
# and RULES_FAULT name the rule table's targets, values and limits
# (page-rules.md RULE-46 to RULE-48), and with the protocol crate, which
# takes its limits by name. The trusted core calls neither its decoder nor
# its evaluator (RULE-58).
readonly CORE_ENGINE_CRATES="neoteletext-audio neoteletext-core neoteletext-formats neoteletext-input neoteletext-opus neoteletext-rules neoteletext-web"
# Requirement 1.3 (REQ-661): no crate in any program's tree executes or
# interprets code from outside. Each is a crate a program could run a
# script, a WebAssembly module or a web page with. One is allowed in one
# place (the owner's DEC-7246 and DEC-7256; the security review's finding
# S-11, TODO-7277 item 3): wasmi, in the tree of the script library and
# the programs built on it alone, named in SCRIPT_PACKAGES. Every other
# package of the workspace, the trusted core, the fetcher, the decoder,
# the interface and the broker among them, is held to having none.
readonly INTERPRETER_CRATES='boa_engine|deno_core|dyon|gluon|hlua|js-sandbox|ketos|koto|mlua|mun_runtime|pyo3|quickjs|quickjs-rs|rhai|rlua|rquickjs|rune|rustpython-vm|servo|starlark|steel-core|tauri|v8|wasm3|wasmer|wasmi|wasmtime|webkit2gtk|webview2-com|wry'
# The packages that may hold the script engine's interpreter, and the one
# interpreter they may hold.
readonly SCRIPT_PACKAGES=(neoteletext-script)
readonly SCRIPT_INTERPRETER='wasmi'
# Requirement 2.2 (REQ-663): no engine crate the browser uses brings a
# user interface or graphics toolkit; those are the interface's alone.
readonly TOOLKIT_CRATES='cairo-rs|cocoa|druid|eframe|egui|embedded-graphics|femtovg|fltk|fontique|gdk|gdk4|glib|glow|glutin|gtk|gtk4|iced|minifb|objc2-app-kit|pango|parley|sdl2|slint|smithay-client-toolkit|softbuffer|tao|tiny-skia|vello|vello_cpu|vello_hybrid|wayland-client|wgpu|winit|x11rb|xilem'

# The packages in the normal and build dependencies of `$@` (cargo tree's
# package options), on every target, run in native/: one a line, as the
# package's name, then its source in brackets where it has one.
tree_packages() {
    (cd native && cargo tree --quiet --locked -e normal,build --target all --prefix none -f '{p}' "$@") | sed 's/ (\*)$//' | sort -u
}
# Their names alone.
tree_names() { tree_packages "$@" | sed 's/ .*//' | sort -u; }
# The engine's crates among them: those from a git source, which
# check_engine_pin holds to the engine's one commit.
engine_names() { tree_packages "$@" | sed -n 's/^\([^ ]*\) v[^ ]* (\(https\|file\):.*/\1/p' | sort -u; }

check_dependency_trees() {
    step "the trusted core's engine crates, and no interpreter or toolkit where none may be (requirements 1.3, 2.2 and 3.2)"
    local found crate
    found="$(engine_names -p neoteletext | tr '\n' ' ' | sed 's/ $//')"
    [[ "$found" == "$CORE_ENGINE_CRATES" ]] || fail "the trusted core links the engine crates \"$found\", not the \"$CORE_ENGINE_CRATES\" DEC-2355 permits"
    local -a outside=(--workspace)
    for crate in "${SCRIPT_PACKAGES[@]}"; do outside+=(--exclude "$crate"); done
    found="$( (tree_names "${outside[@]}"; cd native/tuf && cargo tree --quiet --locked --workspace -e normal,build --target all --prefix none -f '{p}' | sed 's/ .*//') | sort -u | grep -xE "$INTERPRETER_CRATES" || true)"
    [[ -z "$found" ]] || fail "a program's tree holds a script engine, a web engine or an interpreter: $found"
    # The trusted core by name as well, so that it is held even if it were
    # ever listed among the script packages by mistake.
    found="$(tree_names -p neoteletext | grep -xE "$INTERPRETER_CRATES" || true)"
    [[ -z "$found" ]] || fail "the trusted core's tree holds a script engine, a web engine or an interpreter: $found"
    for crate in "${SCRIPT_PACKAGES[@]}"; do
        found="$(tree_names -p "$crate" | grep -xE "$INTERPRETER_CRATES" | tr '\n' ' ' | sed 's/ $//' || true)"
        [[ "$found" == "$SCRIPT_INTERPRETER" ]] || fail "the script package $crate holds the interpreters \"$found\", not exactly \"$SCRIPT_INTERPRETER\""
    done
    for crate in $(engine_names --workspace); do
        found="$(tree_names -p "$crate" | grep -xE "$TOOLKIT_CRATES" || true)"
        [[ -z "$found" ]] || fail "the engine crate $crate brings a user interface or graphics toolkit: $found"
    done
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

# TODO-1280: source code does not name another browser as the source of
# a rule or a number (the owner's baseline rule: no outside reference
# project is named in code). A rule the browser follows from another
# browser is stated in its design document, which may name that browser,
# and the comment cites the section. Allowed are only the lines below,
# each as git grep prints it (path:text), where the browser is the
# subject: a bookmarks file as that browser writes it, which the
# bookmarks import reads.
readonly BROWSER_NAMES='chrome|chromium|firefox'
readonly BROWSER_NAMES_ALLOWED=(
    "native/decoder/src/netscape/tests.rs:/// As Chrome writes it, with Firefox's comments and descriptions."
    'native/decoder/src/netscape/tests.rs:const CHROME: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>'
    'native/decoder/src/netscape/tests.rs:    let found = read(CHROME.as_bytes()).unwrap();'
    'native/decoder/tests/confined_bookmarks.rs:/// As Firefox writes it.'
    'native/decoder/tests/confined_bookmarks.rs:const FIREFOX: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>'
    'native/decoder/tests/confined_bookmarks.rs:    write_to_decoder(&mut input, &ToDecoder::Data(FIREFOX.as_bytes().to_vec())).unwrap();'
)
check_browser_names() {
    step "no source line names Chrome, Chromium or Firefox outside the allow list (TODO-1280)"
    local found status=0 line allowed unexpected=""
    found="$(git grep --cached -IiwE "$BROWSER_NAMES" -- native)" || status=$?
    [[ "$status" -le 1 ]] || fail "git grep failed looking for other browsers' names"
    while IFS= read -r line; do
        [[ -n "$line" ]] || continue
        for allowed in "${BROWSER_NAMES_ALLOWED[@]}"; do
            [[ "$line" == "$allowed" ]] && continue 2
        done
        unexpected+="$line"$'\n'
    done <<<"$found"
    [[ -z "$unexpected" ]] || fail "these lines name another browser; state the rule in its design document and cite the section, or, where the browser is the subject, add the line to BROWSER_NAMES_ALLOWED in scripts/check.sh:
$unexpected"
}

# The engine's REQ-329 (TODO-2422): the standard's page ranges, 100 to
# 8FF and the keyable 100 to 899, are named once, in the engine's
# `neoteletext_core::limits`; no range here ends in them as a literal.
readonly PAGE_RANGE_LITERAL='\.\.=? *0x(8ff|899)\b'
check_page_range_literals() {
    step "no page range is written with the literals 0x8FF or 0x899 (the engine's REQ-329)"
    local found status=0
    found="$(git grep --cached -n -IiE "$PAGE_RANGE_LITERAL" -- 'native/*.rs')" || status=$?
    [[ "$status" -le 1 ]] || fail "git grep failed looking for page range literals"
    [[ -z "$found" ]] || fail "these lines write a page range's end as a literal; use neoteletext_core::limits's FIRST_PAGE_ADDRESS, LAST_PAGE_ADDRESS, FIRST_KEYABLE_PAGE or LAST_KEYABLE_PAGE:
$found"
}

# The security review's finding S-11 (TODO-7277): the script engine's
# module, native/script/qjs/nt-quickjs.wasm, is a committed binary that
# every script runs in, built by native/script/qjs/build.sh from pinned
# sources with a pinned compiler. The commit gate holds the committed file
# to the SHA-256 recorded beside it, and the fuzz crate's list of targets
# to those the script library owes (script-api.md section 11.4). The
# full gate builds the module again from its sources and requires the
# same SHA-256 of the build and of the committed file; the build script
# itself refuses a module that holds a bulk-memory instruction, and fails,
# never skips, when a tool it needs is missing.
readonly SCRIPT_FUZZ_TARGETS=(script_host_ops script_teletext_ops script_imports script_js)
check_script_engine_committed() {
    step "the script engine's committed module has its recorded SHA-256, and its fuzz targets exist (TODO-7277)"
    native/script/qjs/build.sh --committed
    local target
    for target in "${SCRIPT_FUZZ_TARGETS[@]}"; do
        [[ -f "native/fuzz/fuzz_targets/$target.rs" ]] && grep -q "^name = \"$target\"$" native/fuzz/Cargo.toml ||
            fail "the fuzz target $target, which script-api.md section 11.4 gives the script library, is missing from native/fuzz"
    done
}
# The compiler the module is built with is wasi-sdk's for x86_64 Linux, so
# the rebuild runs on Linux: here when this machine is Linux, and always
# on the Linux host with its other steps (linux_steps).
check_script_engine_rebuilds() {
    [[ "$(uname -s)" == Linux ]] || return 0
    step "the script engine's module rebuilds from its pinned sources to the committed bytes (TODO-7277)"
    need_tool cmake "the wasi-libc build's generator; the system's package cmake"
    need_tool ninja "the wasi-libc build; the system's package ninja-build"
    native/script/qjs/build.sh --check
}

# What the commit gate checks: the code compiles clean.
check_compiles() {
    check_browser_names
    check_page_range_literals
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
    check_script_engine_committed
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

# Cargo's line for the one package a development build compiles on every
# run, the trusted core's (native/core, the package `neoteletext`): its
# name alone before the version, so that no `neoteletext-...` package
# matches, and its folder after it.
readonly COUNTED_PACKAGE_COMPILE='^ +Compiling neoteletext v[^ ]+ \(.*/native/core\)$'

# Of Cargo's output on standard input, the lines that say a package was
# compiled, other than the trusted core's.
needless_compiles() {
    grep -E '^ +Compiling ' | grep -vE "$COUNTED_PACKAGE_COMPILE" || true
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
    check_dependency_trees
    check_microcontroller_builds
    # Requirement 14.3 (REQ-783, DEC-2354): the advisory check runs at every
    # push, not only on the way to main, so a crate yanked or found unsafe
    # stops the work while it is fresh.
    step "native, native/tuf: advisories"
    (cd native && cargo deny check advisories)
    (cd native/tuf && cargo deny check advisories)
    step "native: tests"
    (cd native && cargo nextest run --workspace --hide-progress-bar --status-level fail --final-status-level fail)
    step "native/tuf: tests"
    (cd native/tuf && cargo nextest run --workspace --hide-progress-bar --status-level fail --final-status-level fail)
    # REQ-7047's tests that run Cargo: in a worktree of their own, so that
    # neither this checkout's count nor its build lock is touched (TODO-7049).
    step "native: the development build's count and --version (REQ-7047)"
    scripts/test-build-number.sh HEAD
    # DEVELOPMENT.md section 9.2: a pass says nothing of a test that is
    # ignored, so the gate says
    # which ones, each with what would make it run.
    step "the tests that have not run: each ignored test and its reason (DEVELOPMENT.md section 9.2)"
    git grep -n -A1 -F '#[ignore' -- 'native/*.rs'
    # A build with nothing changed must build nothing it need not. A build
    # script that watched files that do not exist once rebuilt the crates
    # under it on every Cargo run, which cost each step of this gate
    # minutes. One package is built again by design: the trusted core's,
    # whose build script runs on every development compile to count it
    # (REQ-7047 item 1), and Cargo compiles again the package whose script
    # ran. That one line is expected; any other fails the step.
    step "native: a build with nothing changed builds nothing but the trusted core's package (REQ-7047)"
    local built others
    built="$(cd native && cargo nextest run --workspace --no-run 2>&1)" || fail "the build failed:
$built"
    others="$(needless_compiles <<<"$built")"
    [[ -z "$others" ]] || fail "a build with nothing changed compiled again:
$others"
}

# Requirement 15.3 (REQ-789, DEC-2357): the interface core and its
# microcontroller back end build for microcontrollers with no standard
# library: Arm Cortex-M0 (no atomic compare-and-swap, no floating point)
# and RISC-V RV32IMC, the smallest of the engine's targets (its
# REQ-GEN-015), with every optional area and with none.
readonly MICROCONTROLLER_TARGETS=(thumbv6m-none-eabi riscv32imc-unknown-none-elf)
check_microcontroller_builds() {
    local target
    for target in "${MICROCONTROLLER_TARGETS[@]}"; do
        # rust-toolchain.toml installs each target's standard library.
        grep -qE "^targets = \[.*\"$target\"" rust-toolchain.toml || fail "$target is not among rust-toolchain.toml's targets"
        step "native: Clippy for $target, interface-core and draw-embedded, with every optional area and with none, warnings as errors"
        (cd native && cargo clippy --quiet --locked -p interface-core -p draw-embedded --target "$target" -- -D warnings &&
            cargo clippy --quiet --locked -p interface-core -p draw-embedded --target "$target" --no-default-features -- -D warnings)
    done
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

linux_steps() {
    step "Linux on $linux_host: native, Clippy and tests, and the script engine's module rebuilt"
    need_host
    need_host_tool cmake "the script engine's wasi-libc build needs it; the system's package cmake"
    need_host_tool ninja "the script engine's wasi-libc build needs it; the system's package ninja-build"
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
        native/script/qjs/build.sh --check
        (cd native && cargo clippy --quiet --workspace --all-targets -- -D warnings && cargo test --quiet --workspace \\
            && cargo clippy --quiet --workspace --all-targets --no-default-features -- -D warnings)
        (cd native/tuf && cargo clippy --quiet --workspace --all-targets -- -D warnings && cargo test --quiet --workspace)"
}

# libFuzzer's options for a target whose inputs must reach past its
# default length limit of 4,096 bytes: dbus_message reads messages up to
# neoteletext-dbus's MAX_MESSAGE_BYTES, 128 KiB (131,072 bytes), where the
# security review of 22 November 2026 found its fault (TODO-7173 A1). The
# script link's and the web client's messages are read up to a whole
# frame, the protocol crate's MAX_PAYLOAD (262,144 bytes) after its five
# header bytes: a batch's longest lists lie past 4,096 (TODO-7288).
fuzz_options() {
    case "$1" in
        dbus_message) echo "-max_len=131072" ;;
        # A whole script, and a memory image of a page.
        script_js | script_imports) echo "-max_len=65536" ;;
        script_link_to_host | script_link_to_decoder | web_to_client | web_from_client) echo "-max_len=262149" ;;
    esac
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
            local target log seeds
            for target in $(cargo "+$FUZZ_NIGHTLY" fuzz list); do
                mkdir -p "corpus/$target" "regressions/$target"
                log="target/fuzz-$target.log"
                mkdir -p target
                # A target's committed seeds, where it has any (script_js
                # starts from the security review's hostile scripts).
                seeds=""
                [[ ! -d "seeds/$target" ]] || seeds="seeds/$target"
                cargo "+$FUZZ_NIGHTLY" fuzz run "$target" "corpus/$target" "regressions/$target" $seeds -- \
                    -max_total_time="$fuzz_seconds" -timeout=10 -rss_limit_mb=2048 $(fuzz_options "$target") >"$log" 2>&1 ||
                    { tail -40 "$log"; fail "fuzz target $target found a fault; its input is in $folder/artifacts/$target/"; }
                printf '%s: %s\n' "$target" "$(grep '^Done' "$log")"
            done
        )
    done
}

# The engine's repository and pinned commit, as native/Cargo.toml names
# them (check_engine_pin holds every workspace to the same), fetched into
# a bare copy the gate keeps between runs, native/target/engine.git. A
# hook runs with git's GIT_DIR set to this repository, absolute in a
# worktree, which git would follow instead of the copy's.
engine_copy="native/target/engine.git"
engine_git() { env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE git --git-dir "$engine_copy" "$@"; }
engine_fetch() {
    engine="$(sed -n 's/^neoteletext-[a-z-]* = { git = "\([^"]*\)".*/\1/p' native/Cargo.toml | sort -u)"
    revision="$(sed -n 's/^neoteletext-[a-z-]* = { git = "[^"]*", rev = "\([0-9a-f]*\)".*/\1/p' native/Cargo.toml | sort -u)"
    [[ -n "$engine" && "$engine" != *$'\n'* ]] || fail "native/Cargo.toml must name one engine repository, found: ${engine:-none}"
    [[ -n "$revision" && "$revision" != *$'\n'* ]] || fail "native/Cargo.toml must name one engine commit, found: ${revision:-none}"
    [[ -d "$engine_copy" ]] || env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE git init --quiet --bare "$engine_copy"
    engine_git cat-file -e "$revision^{commit}" 2>/dev/null || engine_git fetch --quiet "$engine" "$revision" ||
        fail "the engine repository $engine has no commit $revision"
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
    engine_fetch
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

# Requirement 2.3 (REQ-664, DEC-2360): the engine's crates are built with
# their own checks, fuzzing and feature rules, so the engine's own full
# gate runs at the commit the browser builds against; its feature rule
# for a browser build, decision E.131, is check_desktop_codecs. It runs
# in a clone of the bare
# copy, native/target/engine-check, never in a working tree of the
# engine, and the clone keeps the engine gate's record of what passed, so
# the step takes seconds until the pin moves. The engine's pinned tools
# are linked from its checkout ($NEOTELETEXT_ENGINE, as worktrees of the
# engine link them); at another version, the engine's gate says so and
# fails.
engine_checkout="${NEOTELETEXT_ENGINE:-$HOME/devel/neoteletext-renderer-teletext}"
engine_check="native/target/engine-check"
check_engine_gate() {
    step "the engine's own full gate at its pinned commit (requirement 2.3)"
    local engine revision
    engine_fetch
    check_git() { env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE git -C "$engine_check" "$@"; }
    [[ -d "$engine_check/.git" ]] || env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE git init --quiet "$engine_check"
    check_git cat-file -e "$revision^{commit}" 2>/dev/null || check_git fetch --quiet "$(pwd -P)/$engine_copy" "$revision"
    check_git checkout --quiet --detach "$revision"
    [[ -z "$(check_git status --porcelain)" ]] || fail "$engine_check has changes; the engine's gate would check something other than $revision"
    [[ -d "$engine_checkout/tools" ]] || fail "the engine's pinned tools are needed at $engine_checkout/tools (or NEOTELETEXT_ENGINE); run scripts/tools.sh there"
    [[ -e "$engine_check/tools" ]] || ln -s "$engine_checkout/tools" "$engine_check/tools"
    (cd "$engine_check" && env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE scripts/check.sh full) ||
        fail "the engine's full gate failed at $revision"
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

commit_steps() { check_compiles; }
push_steps() { check_shapes_and_tests; }
full_steps() {
    check_engine_safe
    check_engine_gate
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
    check_script_engine_rebuilds
    step "native/tuf: release tests"
    (cd native/tuf && cargo nextest run --workspace --release --hide-progress-bar --status-level fail --final-status-level fail)
    check_fuzz
}

gate_main "$@"

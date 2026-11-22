#!/usr/bin/env bash
# Builds the script engine's module, nt-quickjs.wasm (DEC-7246, DEC-7256;
# script-api.md section 3.2 rule 5; the security review's findings S-04,
# S-11 and S-18 item 1: TODO-7270, TODO-7277, TODO-7285): quickjs-ng at an
# exact release, the host-boundary shim (shim.c) and the module's own
# memory functions (mem.c), compiled by wasi-sdk at an exact release
# against wasi-libc rebuilt here from its source at the commit that SDK
# was made with.
#
#   native/script/qjs/build.sh           builds nt-quickjs.wasm beside this
#                                        file and checks it against
#                                        nt-quickjs.wasm.sha256
#   native/script/qjs/build.sh --record  builds it and records its SHA-256
#   native/script/qjs/build.sh --check   builds it in the work folder
#                                        alone and fails unless both the
#                                        build and the committed module
#                                        have the recorded SHA-256 (the
#                                        full gate)
#   native/script/qjs/build.sh --committed  no build: fails unless the
#                                        committed module has the recorded
#                                        SHA-256 (the commit gate)
#
# Every input is downloaded into the work folder (the repository's ignored
# tools/script-engine, or $NEOTELETEXT_SCRIPT_ENGINE_WORK) and refused
# unless its SHA-256 is the one recorded here. Downloads use Python's
# standard library, never curl: nothing here links libcurl (DEC-7253).
#
# What the flags below hold, each a property the security of the container
# rests on (TODO-7277 item 2):
#   * no bulk memory: -mno-bulk-memory and -mno-bulk-memory-opt for every
#     object, the C library included, so that no single instruction copies
#     or fills an unbounded span (the prebuilt C library of the SDK is
#     compiled with bulk memory, which is why it is rebuilt), and the
#     module's code is then read and refused if one is found;
#   * no threads: the single-threaded C library and target, so no shared
#     memory and no Atomics;
#   * no quickjs-libc: only QuickJS's four engine sources are compiled, so
#     there is no `os` or `std` module, no file and no process access;
#   * a reactor with no main: the shim's exports are the only entry points;
#   * a 1 MiB stack and at most 64 MiB of memory.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
work="${NEOTELETEXT_SCRIPT_ENGINE_WORK:-$root/tools/script-engine}"
mode="${1:-build}"

readonly QUICKJS_VERSION="0.18.0"
readonly QUICKJS_URL="https://github.com/quickjs-ng/quickjs/archive/refs/tags/v${QUICKJS_VERSION}.tar.gz"
readonly QUICKJS_SHA256="28bcd89aa3683fd2a946537e5ce3f11cc46b5cbf3383c7561aaef3df5ff9f1dd"
readonly WASI_SDK_VERSION="34.0"
readonly WASI_SDK_URL="https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-${WASI_SDK_VERSION}-x86_64-linux.tar.gz"
readonly WASI_SDK_SHA256="b761e3a0721dbae9c09a0059e5fdb2bf917d1b4a8a7b430fb3b5aafb0984b2c4"
# wasi-libc at the commit wasi-sdk 34.0 was made with (the SDK's VERSION
# file: 2e6fb9d8ee0c), which its repository tags wasi-sdk-34.
readonly WASI_LIBC_TAG="wasi-sdk-34"
readonly WASI_LIBC_URL="https://github.com/WebAssembly/wasi-libc/archive/refs/tags/${WASI_LIBC_TAG}.tar.gz"
readonly WASI_LIBC_SHA256="21bc47f96cf0eb535e532697dce4cdfdbc91f59223bdc00d8495d191888c5fe8"

# The WebAssembly stack QuickJS runs on, and the most linear memory the
# module may declare; the host's own limits (native/script/src/limits.rs:
# QUICKJS_STACK_BYTES below the first, WASM_MEMORY_BYTES equal to the
# second) are the ones enforced, and the crate's tests compare them with
# the module.
readonly STACK_BYTES=$((1024 * 1024))
readonly MAX_MEMORY_BYTES=$((64 * 1024 * 1024))
# No instruction that copies or fills a span of memory or a table in one
# step. `-mno-bulk-memory-opt` is the part of the feature LLVM 20 and later
# turn on by default for memory.copy and memory.fill alone.
readonly NO_BULK_MEMORY="-mno-bulk-memory -mno-bulk-memory-opt"
# The instructions the finished module may not hold (the WebAssembly bulk
# memory operations proposal), as llvm-objdump names them.
readonly BULK_INSTRUCTIONS='memory\.copy|memory\.fill|memory\.init|data\.drop|table\.copy|table\.init|table\.fill|elem\.drop'

module="$here/nt-quickjs.wasm"
recorded="$here/nt-quickjs.wasm.sha256"

fail() {
    echo "build.sh: $*" >&2
    exit 1
}
sha256_of() { sha256sum "$1" | cut -d' ' -f1; }
recorded_sha256() {
    [[ -f "$recorded" ]] || fail "$recorded is missing; build.sh --record writes it"
    cut -d' ' -f1 "$recorded"
}
check_committed() {
    local got want
    want="$(recorded_sha256)"
    [[ -f "$module" ]] || fail "$module is missing"
    got="$(sha256_of "$module")"
    [[ "$got" == "$want" ]] || fail "$module has SHA-256 $got, not the recorded $want: it was changed without being rebuilt and recorded"
}

if [[ "$mode" == "--committed" ]]; then
    check_committed
    exit 0
fi
[[ "$mode" == "build" || "$mode" == "--record" || "$mode" == "--check" ]] || fail "unknown argument $mode"

for tool in python3 cmake ninja tar sha256sum; do
    command -v "$tool" >/dev/null || fail "$tool is needed and was not found"
done

# Downloads `url` to `file` unless it is there, and refuses it unless its
# SHA-256 is `sha256`.
fetch() {
    local url="$1" file="$2" sha256="$3" got
    if [[ ! -f "$file" ]]; then
        python3 -I -c 'import sys, urllib.request; urllib.request.urlretrieve(sys.argv[1], sys.argv[2])' "$url" "$file.part"
        mv "$file.part" "$file"
    fi
    got="$(sha256_of "$file")"
    [[ "$got" == "$sha256" ]] || fail "$file has SHA-256 $got, not $sha256; refused"
}

mkdir -p "$work"
fetch "$QUICKJS_URL" "$work/quickjs-ng-v$QUICKJS_VERSION.tar.gz" "$QUICKJS_SHA256"
fetch "$WASI_SDK_URL" "$work/wasi-sdk-$WASI_SDK_VERSION.tar.gz" "$WASI_SDK_SHA256"
fetch "$WASI_LIBC_URL" "$work/wasi-libc-$WASI_LIBC_TAG.tar.gz" "$WASI_LIBC_SHA256"

# Each archive is unpacked afresh into a folder named for its checksum, so
# that a source tree changed after it was unpacked is never built from.
unpack() {
    local archive="$1" sha256="$2" target="$work/src-$2"
    if [[ ! -f "$target/.unpacked" ]]; then
        rm -rf "$target"
        mkdir -p "$target"
        tar -xzf "$archive" -C "$target" --strip-components=1
        touch "$target/.unpacked"
    fi
    echo "$target"
}
quickjs="$(unpack "$work/quickjs-ng-v$QUICKJS_VERSION.tar.gz" "$QUICKJS_SHA256")"
sdk="$(unpack "$work/wasi-sdk-$WASI_SDK_VERSION.tar.gz" "$WASI_SDK_SHA256")"
libc_source="$(unpack "$work/wasi-libc-$WASI_LIBC_TAG.tar.gz" "$WASI_LIBC_SHA256")"

# The C library, rebuilt without bulk memory by its own build, with the
# SDK's compiler and the SDK's prebuilt compiler-rt (so that its build
# downloads nothing). Built afresh each time: it takes seconds.
libc_build="$work/libc-build"
rm -rf "$libc_build"
builtins="$("$sdk/bin/clang" --target=wasm32-wasip1 --print-libgcc-file-name)"
[[ -f "$builtins" ]] || fail "the SDK has no compiler-rt at $builtins"
cmake -G Ninja -S "$libc_source" -B "$libc_build" \
    -DCMAKE_C_COMPILER="$sdk/bin/clang" -DCMAKE_AR="$sdk/bin/llvm-ar" -DCMAKE_NM="$sdk/bin/llvm-nm" -DCMAKE_RANLIB="$sdk/bin/llvm-ranlib" \
    -DTARGET_TRIPLE=wasm32-wasip1 -DBUILD_SHARED=OFF -DBUILD_TESTS=OFF -DCMAKE_BUILD_TYPE=Release -DBUILTINS_LIB="$builtins" \
    "-DCMAKE_C_FLAGS=$NO_BULK_MEMORY -ffile-prefix-map=$work=." >"$work/libc-configure.log" 2>&1 || { tail -30 "$work/libc-configure.log" >&2; fail "configuring wasi-libc failed"; }
ninja -C "$libc_build" >"$work/libc-build.log" 2>&1 || { tail -30 "$work/libc-build.log" >&2; fail "building wasi-libc failed"; }
sysroot="$libc_build/sysroot"
[[ -f "$sysroot/lib/wasm32-wasip1/libc.a" ]] || fail "the wasi-libc build made no libc.a"

built="$work/nt-quickjs.wasm"
objects="$work/objects"
rm -rf "$objects" "$built"
mkdir -p "$objects"
# The source paths are mapped away so the module does not carry this
# machine's folders.
common=(--target=wasm32-wasip1 --sysroot="$sysroot" -O2 -std=gnu11 $NO_BULK_MEMORY
    -D_GNU_SOURCE -D_WASI_EMULATED_SIGNAL -D_WASI_EMULATED_PROCESS_CLOCKS
    "-ffile-prefix-map=$work=." "-ffile-prefix-map=$here=." "-I$quickjs")
for source in "$here/shim.c" "$quickjs/quickjs.c" "$quickjs/libregexp.c" "$quickjs/libunicode.c" "$quickjs/dtoa.c"; do
    "$sdk/bin/clang" "${common[@]}" -c "$source" -o "$objects/$(basename "$source" .c).o"
done
# The memory functions: no builtins, so their loops stay loops.
"$sdk/bin/clang" "${common[@]}" -fno-builtin -c "$here/mem.c" -o "$objects/mem.o"
# The module's own objects come first, so that its memcpy, memmove,
# memset, sbrk, gettimeofday and localtime_r are the ones linked and the C
# library's are never pulled in.
"$sdk/bin/clang" --target=wasm32-wasip1 --sysroot="$sysroot" -mexec-model=reactor $NO_BULK_MEMORY \
    "$objects/mem.o" "$objects/shim.o" "$objects/quickjs.o" "$objects/libregexp.o" "$objects/libunicode.o" "$objects/dtoa.o" \
    -lwasi-emulated-signal -lwasi-emulated-process-clocks \
    -Wl,-z,stack-size=$STACK_BYTES -Wl,--max-memory=$MAX_MEMORY_BYTES -Wl,--strip-all \
    -o "$built"

# The finished module's code, read whole: no bulk-memory instruction
# anywhere in it, whichever object it came from.
disassembly="$work/nt-quickjs.dis"
"$sdk/bin/llvm-objdump" -d "$built" >"$disassembly"
[[ "$(wc -l <"$disassembly")" -gt 100000 ]] || fail "the module's disassembly is too short to be its code; the bulk-memory check read nothing"
if grep -qE "$BULK_INSTRUCTIONS" "$disassembly"; then
    grep -oE "$BULK_INSTRUCTIONS" "$disassembly" | sort | uniq -c >&2
    fail "the module holds bulk-memory instructions; refused"
fi

got="$(sha256_of "$built")"
echo "build.sh: $built, $(stat -c %s "$built") bytes, SHA-256 $got"
case "$mode" in
--record)
    cp "$built" "$module"
    echo "$got  nt-quickjs.wasm" >"$recorded"
    ;;
--check)
    [[ "$got" == "$(recorded_sha256)" ]] || fail "the rebuilt module has SHA-256 $got, not the recorded $(recorded_sha256)"
    check_committed
    ;;
build)
    [[ "$got" == "$(recorded_sha256)" ]] || fail "the rebuilt module has SHA-256 $got, not the recorded $(recorded_sha256); build.sh --record records a deliberate change"
    cp "$built" "$module"
    ;;
esac

#!/usr/bin/env python3
"""Fetches the Public Suffix List and, with --refresh, replaces the copy
kept beside this script, or with --check compares it.

Python's standard library alone (no libcurl in any form: DEC-7253). The
list is taken from its publisher's own address, as the file itself asks,
and kept unchanged (NOTICE.txt; DEC-7340).
"""

import hashlib
import pathlib
import sys
import urllib.request

LIST_URL = "https://publicsuffix.org/list/public_suffix_list.dat"
LIST_NAME = "public_suffix_list.dat"
# The largest list taken: the fetcher's own bound on the file it reads
# (MAX_SUFFIX_LIST_BYTES in native/fetcher/src/web/limits.rs).
MAX_LIST_BYTES = 1 << 20
# The longest a download may take, in seconds.
TIMEOUT_SECONDS = 60


def fail(message):
    print(f"fetch.py: {message}", file=sys.stderr)
    sys.exit(1)


def header(content, name):
    """The value of the list's own `// NAME: value` line."""
    for line in content.decode("utf-8").splitlines():
        if line.startswith(f"// {name}: "):
            return line.split(": ", 1)[1]
    fail(f"the list has no {name} line")


def main():
    if sys.argv[1:] not in (["--check"], ["--refresh"]):
        fail("usage: fetch.py --check | --refresh")
    refresh = sys.argv[1] == "--refresh"
    folder = pathlib.Path(__file__).resolve().parent
    request = urllib.request.Request(LIST_URL, headers={"User-Agent": "python-urllib"})
    with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as answer:
        content = answer.read(MAX_LIST_BYTES + 1)
    if len(content) > MAX_LIST_BYTES:
        fail(f"the list is longer than {MAX_LIST_BYTES} bytes")
    digest = hashlib.sha256(content).hexdigest()
    kept = folder / LIST_NAME
    if refresh:
        kept.write_bytes(content)
        licence = hashlib.sha256((folder / "LICENSE").read_bytes()).hexdigest()
        (folder / "SHA256SUMS").write_text(f"{digest}  {LIST_NAME}\n{licence}  LICENSE\n")
        print(f"written: version {header(content, 'VERSION')}, commit {header(content, 'COMMIT')}, {len(content)} bytes, SHA-256 {digest}")
        print("now rewrite NOTICE.txt's version, commit, date, size and SHA-256")
    elif kept.read_bytes() == content:
        print(f"checked: the kept list is the publisher's, version {header(content, 'VERSION')}")
    else:
        fail(f"the publisher's list is now version {header(content, 'VERSION')}; the kept one is {header(kept.read_bytes(), 'VERSION')}")


if __name__ == "__main__":
    main()

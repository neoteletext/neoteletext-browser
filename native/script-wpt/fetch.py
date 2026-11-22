#!/usr/bin/env python3
"""Fetches the web-platform-tests at the pinned commit, and keeps the
folders the harness of this crate runs.

The tests are third-party files (3-Clause BSD, the project's LICENSE.md)
and are never copied into this repository: they are written under the
folder given, by default ~/devel/teletext-reference/inbound/, and the
harness reads them from the path NEOTELETEXT_WPT names. Python's standard
library alone (no libcurl in any form: DEC-7253).

The project's archive of one commit is some hundreds of megabytes; it is
read as a stream and only the folders named below are written. What is
written is held to MANIFEST_SHA256, the SHA-256 of the sorted lines
"<sha256 of the file>  <its path>" of every file kept, so that the tests
run are the tests that were run when the expectations were written.
"""

import hashlib
import pathlib
import sys
import tarfile
import urllib.request

# The commit of https://github.com/web-platform-tests/wpt the expectations
# were written against: the head of its master branch on 22 November 2026.
COMMIT = "42e27408034f57afe4c726476d1a0bd96b873129"
ARCHIVE_URL = f"https://codeload.github.com/web-platform-tests/wpt/tar.gz/{COMMIT}"
# The folders kept, and the licence. `resources` holds testharness.js;
# `common` what many tests load beside it.
KEPT = ("LICENSE.md", "resources/", "common/", "url/", "encoding/", "streams/", "fetch/", "cookies/", "websockets/", "eventsource/", "FileAPI/", "xhr/formdata/")
# The digest of what is kept (see the module's text); empty until first taken.
MANIFEST_SHA256 = "4d3c73bb828c6930f87efd6a4180edf8eb2bbd598ab8171a469220786e66e146"
# The longest the whole download may take, in seconds, and the largest
# file kept: the suite's largest in these folders is a few megabytes.
TIMEOUT_SECONDS = 120
MAX_FILE_BYTES = 64 * 1024 * 1024


def fail(message):
    print(f"fetch.py: {message}", file=sys.stderr)
    sys.exit(1)


def main():
    arguments = sys.argv[1:]
    if len(arguments) > 1:
        fail("usage: fetch.py [FOLDER]")
    parent = pathlib.Path(arguments[0] if arguments else "~/devel/teletext-reference/inbound").expanduser()
    target = parent / f"wpt-{COMMIT[:12]}"
    if target.exists():
        fail(f"{target} exists already; remove it to fetch again")
    lines = []
    request = urllib.request.Request(ARCHIVE_URL, headers={"User-Agent": "python-urllib"})
    with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as answer:
        with tarfile.open(fileobj=answer, mode="r|gz") as archive:
            for member in archive:
                # Every path begins with the archive's own folder.
                path = member.name.split("/", 1)[1] if "/" in member.name else ""
                if not member.isfile() or not path.startswith(KEPT) or ".." in path.split("/"):
                    continue
                if member.size > MAX_FILE_BYTES:
                    fail(f"{path} is {member.size} bytes")
                content = archive.extractfile(member).read()
                kept = target / path
                kept.parent.mkdir(parents=True, exist_ok=True)
                kept.write_bytes(content)
                lines.append(f"{hashlib.sha256(content).hexdigest()}  {path}\n")
    digest = hashlib.sha256("".join(sorted(lines)).encode()).hexdigest()
    print(f"{len(lines)} files kept in {target}")
    print(f"manifest SHA-256 {digest}")
    if MANIFEST_SHA256 and digest != MANIFEST_SHA256:
        fail(f"what was kept is not what is pinned ({MANIFEST_SHA256})")


if __name__ == "__main__":
    main()

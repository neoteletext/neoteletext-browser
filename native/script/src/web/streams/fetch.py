#!/usr/bin/env python3
"""Fetches the pinned web-streams-polyfill package and writes, or with
--check compares, the two files of it kept beside this script.

Python's standard library alone (no libcurl in any form: DEC-7253). The
package is refused unless its SHA-256 is the one recorded here, and each
file unless its SHA-256 is the one SHA256SUMS records, so that the
bytes compiled into the script library are the bytes that were reviewed
when the pin was made (NOTICE.txt; neoteletext-common's DEC-7261).
"""

import hashlib
import io
import pathlib
import sys
import tarfile
import urllib.request

# The exact version, the registry's address for it and the SHA-256 of
# the package file, taken on 22 November 2026. The registry's own
# sha512 integrity value for this file is
# pFmV/G8K9uKW3I3xx3vqpKYLBer7K1wZej5uU66zhtjzG1bfE6e+OHNxhmGb9UsXe+geX4/lndxJDgs7wbYO9g==
VERSION = "4.5.1"
PACKAGE_URL = f"https://registry.npmjs.org/web-streams-polyfill/-/web-streams-polyfill-{VERSION}.tgz"
PACKAGE_SHA256 = "5d7862804fd5a3e018a84b78631159368ecdd683c71725a02758cfed3f827a03"
# The largest package taken: ten times this version's 114,284 bytes.
MAX_PACKAGE_BYTES = 1_200_000
# Each file kept, by its path in the package.
FILES = {"ponyfill.js": "package/dist/ponyfill.js", "LICENSE": "package/LICENSE"}
# The longest a download may take, in seconds.
TIMEOUT_SECONDS = 60


def fail(message):
    print(f"fetch.py: {message}", file=sys.stderr)
    sys.exit(1)


def pinned_sums(folder):
    sums = {}
    for line in (folder / "SHA256SUMS").read_text().splitlines():
        digest, name = line.split()
        sums[name] = digest
    return sums


def main():
    check = sys.argv[1:] == ["--check"]
    if sys.argv[1:] and not check:
        fail("usage: fetch.py [--check]")
    folder = pathlib.Path(__file__).resolve().parent
    sums = pinned_sums(folder)
    request = urllib.request.Request(PACKAGE_URL, headers={"User-Agent": "python-urllib"})
    with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as answer:
        package = answer.read(MAX_PACKAGE_BYTES + 1)
    if len(package) > MAX_PACKAGE_BYTES:
        fail(f"the package is longer than {MAX_PACKAGE_BYTES} bytes")
    digest = hashlib.sha256(package).hexdigest()
    if digest != PACKAGE_SHA256:
        fail(f"the package's SHA-256 is {digest}, not the pinned {PACKAGE_SHA256}")
    with tarfile.open(fileobj=io.BytesIO(package)) as archive:
        for name, member in FILES.items():
            extracted = archive.extractfile(member)
            if extracted is None:
                fail(f"the package has no file {member}")
            content = extracted.read()
            digest = hashlib.sha256(content).hexdigest()
            if digest != sums.get(name):
                fail(f"{member}'s SHA-256 is {digest}, not the {sums.get(name)} SHA256SUMS records")
            kept = folder / name
            if check:
                if kept.read_bytes() != content:
                    fail(f"{kept} is not the package's {member}")
            else:
                kept.write_bytes(content)
    print(f"web-streams-polyfill {VERSION}: {'checked' if check else 'written'}")


if __name__ == "__main__":
    main()

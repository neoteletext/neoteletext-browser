"""A channel's program for the tests of the shared connection (REQ-6989):
every page a connection selects is answered with one sub-page whose row 1
names the page, so each tab can be seen to have been sent its own
selection. It speaks the program protocol's JSON lines on standard input
and output (DOC-1964), and appends what it hears to the file named as its
argument."""
import base64
import json
import sys

log = open(sys.argv[1], "a") if len(sys.argv) > 1 else None
COLUMNS = 40
ROWS = 25


def row(text):
    return base64.b64encode(text.encode("ascii").ljust(COLUMNS)[:COLUMNS]).decode()


def page(number):
    rows = [None] * ROWS
    rows[1] = row("PAGE " + number)
    return {
        "page": number,
        "sub": "0000",
        "rows": rows,
        "links": [None] * 6,
        "display_row_24": False,
        "link_extension": {"keys": [], "numbers": [], "texts": []},
        "control": {
            "erase": False,
            "newsflash": False,
            "subtitle": False,
            "suppress_header": False,
            "update": False,
            "interrupted_sequence": False,
            "inhibit_display": False,
            "magazine_serial": False,
        },
        "character_set": {"region": None, "national_option": 0},
        "enhancements": [],
        "packets": [],
        "cursor": None,
    }


MAGAZINES = [
    {"magazine": 1, "name": "One", "pages": [{"page": "100", "title": "First", "section": None, "subpages": 1}]},
    {"magazine": 2, "name": "Two", "pages": [{"page": "200", "title": "Second", "section": None, "subpages": 1}]},
]

for line in sys.stdin:
    if log:
        log.write(line)
        log.flush()
    message = json.loads(line)
    # A connection's first selection comes in "connected"; each later one
    # is a "selected" of its own.
    number = (message.get("selection") or {}).get("page") or message.get("page")
    if message.get("type") == "connected":
        answer = {"type": "reply", "connection": message["connection"], "request": message["request"], "magazines": MAGAZINES}
        answer["selected"] = page(number) if number else None
    elif message.get("type") == "selected" and number:
        answer = {"type": "page", "connections": [message["connection"]], "request": message["request"], "page": page(number)}
    else:
        continue
    print(json.dumps(answer), flush=True)

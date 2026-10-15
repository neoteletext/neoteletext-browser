"""Reads the accessibility bus as a screen reader does (AT-SPI, through the
Atspi library Orca uses): lists every application, prints each one's tree,
and, given --focus NAME, asks the node with that name for the keyboard.
With --listen SECONDS it instead prints the focus events applications send
for that long, as a screen reader receives them."""
import sys

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

MAX_DEPTH = 6


def describe(node):
    parts = [node.get_role_name(), repr(node.get_name())]
    if node.get_text_iface() is not None:
        parts.append("text=" + repr(Atspi.Text.get_text(node, 0, Atspi.Text.get_character_count(node))))
    if node.get_state_set().contains(Atspi.StateType.FOCUSED):
        parts.append("FOCUSED")
    return " ".join(parts)


def walk(node, depth, found):
    print("  " + "  " * depth + describe(node))
    found.append(node)
    if depth < MAX_DEPTH:
        for index in range(node.get_child_count()):
            walk(node.get_child_at_index(index), depth + 1, found)


def on_event(event):
    if event.detail1:
        print(f"  focus event: {event.source.get_role_name()} {event.source.get_name()!r}", flush=True)


if len(sys.argv) == 3 and sys.argv[1] == "--listen":
    listener = Atspi.EventListener.new(on_event)
    listener.register("object:state-changed:focused")
    GLib = gi.repository.GLib
    loop = GLib.MainLoop()
    GLib.timeout_add_seconds(int(sys.argv[2]), loop.quit)
    loop.run()
    sys.exit(0)

desktop = Atspi.get_desktop(0)
nodes = []
print(f"applications on the accessibility bus: {desktop.get_child_count()}")
for index in range(desktop.get_child_count()):
    application = desktop.get_child_at_index(index)
    try:
        process = application.get_process_id()
    except gi.repository.GLib.GError:
        # A screen reader that has just been stopped can linger on the bus.
        print(f"application {application.get_name()!r}, process gone")
        continue
    print(f"application {application.get_name()!r}, process {process}")
    try:
        walk(application, 0, nodes)
    except gi.repository.GLib.GError as error:
        print(f"  (gone while being read: {error.message})")

if len(sys.argv) == 3 and sys.argv[1] == "--focus":
    target = next((node for node in nodes if node.get_name() == sys.argv[2]), None)
    granted = target is not None and target.get_component_iface() is not None and Atspi.Component.grab_focus(target)
    print(f"asking {sys.argv[2]!r} for the keyboard: {bool(granted)}")

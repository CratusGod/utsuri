#!/usr/bin/env python3
"""A test window that starts real Wayland file drags (text/uri-list), like a file manager.

The files offered come from a control file (one absolute path per line), read when each
drag begins, so a test run can drag a different file every case without restarting it.
Usage: drag_source.py <control-file> [title]
"""
import sys
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, Gio, GObject, Gtk  # noqa: E402

CONTROL = Path(sys.argv[1])
TITLE = sys.argv[2] if len(sys.argv) > 2 else "utsuri-e2e-source"


def files():
    lines = CONTROL.read_text().splitlines() if CONTROL.exists() else []
    return [Gio.File.new_for_path(line) for line in lines if line.strip()]


def on_prepare(_src, _x, _y):
    fl = Gdk.FileList.new_from_array(files())
    return Gdk.ContentProvider.new_for_value(GObject.Value(Gdk.FileList, fl))


def on_activate(app):
    win = Gtk.ApplicationWindow(application=app, title=TITLE)
    win.set_default_size(300, 300)
    label = Gtk.Label(label="drag me")
    label.set_hexpand(True)
    label.set_vexpand(True)
    src = Gtk.DragSource(actions=Gdk.DragAction.COPY)
    src.connect("prepare", on_prepare)
    label.add_controller(src)
    win.set_child(label)
    win.present()


app = Gtk.Application(application_id="dev.cratusgod.utsurie2esource", flags=Gio.ApplicationFlags.NON_UNIQUE)
app.connect("activate", on_activate)
app.run([])

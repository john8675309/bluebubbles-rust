#!/usr/bin/env python3
"""An isolated notification service; run inside dbus-run-session."""
import os
from pathlib import Path
import subprocess
import threading
import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

DBusGMainLoop(set_as_default=True)
loop = GLib.MainLoop()
received = []

class Notifications(dbus.service.Object):
    @dbus.service.method("org.freedesktop.Notifications", in_signature="susssasa{sv}i", out_signature="u")
    def Notify(self, app, replaces, icon, summary, body, actions, hints, timeout):
        received.append((str(app), str(summary), str(body)))
        return 1

    @dbus.service.method("org.freedesktop.Notifications", out_signature="ssss")
    def GetServerInformation(self):
        return "BlueBubbles mock", "Tests", "1", "1.2"

    @dbus.service.method("org.freedesktop.Notifications", out_signature="as")
    def GetCapabilities(self):
        return ["body", "body-markup"]

bus = dbus.SessionBus()
name = dbus.service.BusName("org.freedesktop.Notifications", bus=bus)
service = Notifications(name, "/org/freedesktop/Notifications")
outcome = []

def run():
    result = subprocess.run([os.environ.get("CARGO_BIN", "cargo"), "test", "--locked", "--test", "desktop_notifications", "--", "--ignored"], cwd=Path(__file__).resolve().parents[1], timeout=60)
    outcome.append(result.returncode)
    GLib.idle_add(loop.quit)

threading.Thread(target=run, daemon=True).start()
loop.run()
assert outcome == [0], outcome
assert received == [("BlueBubbles", "BlueBubbles", "Desktop notifications are working.")], received
print("Desktop notification reached the isolated D-Bus service")

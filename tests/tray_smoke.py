#!/usr/bin/env python3
"""Run with dbus-run-session + xvfb-run; all messages and desktop services are fake."""
import ctypes
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
import traceback
import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib
import ui_smoke as ui

DBusGMainLoop(set_as_default=True)
loop = GLib.MainLoop()
bus = dbus.SessionBus()
registered = []
notifications = []
outcome = []

class Watcher(dbus.service.Object):
    @dbus.service.method('org.kde.StatusNotifierWatcher', in_signature='s')
    def RegisterStatusNotifierItem(self, service):
        registered.append(str(service))

    @dbus.service.method('org.freedesktop.DBus.Properties', in_signature='ss', out_signature='v')
    def Get(self, interface, name):
        return dbus.Boolean(True)

    @dbus.service.method('org.freedesktop.DBus.Properties', in_signature='s', out_signature='a{sv}')
    def GetAll(self, interface):
        return {'IsStatusNotifierHostRegistered': True, 'RegisteredStatusNotifierItems': dbus.Array(registered, signature='s'), 'ProtocolVersion': 0}

class Notifications(dbus.service.Object):
    @dbus.service.method('org.freedesktop.Notifications', in_signature='susssasa{sv}i', out_signature='u')
    def Notify(self, app, replaces, icon, summary, body, actions, hints, timeout):
        notifications.append((str(app), str(summary), str(body)))
        return len(notifications)

    @dbus.service.method('org.freedesktop.Notifications', out_signature='ssss')
    def GetServerInformation(self):
        return 'BlueBubbles mock', 'Tests', '1', '1.2'

    @dbus.service.method('org.freedesktop.Notifications', out_signature='as')
    def GetCapabilities(self):
        return ['body', 'body-markup']

watcher_name = dbus.service.BusName('org.kde.StatusNotifierWatcher', bus=bus)
watcher = Watcher(watcher_name, '/StatusNotifierWatcher')
notification_name = dbus.service.BusName('org.freedesktop.Notifications', bus=bus)
notification_service = Notifications(notification_name, '/org/freedesktop/Notifications')

def close_window(window):
    x11 = ctypes.CDLL('libX11.so.6')
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    x11.XInternAtom.restype = ctypes.c_ulong
    class ClientMessage(ctypes.Structure):
        _fields_ = [('type', ctypes.c_int), ('serial', ctypes.c_ulong), ('send_event', ctypes.c_int),
                    ('display', ctypes.c_void_p), ('window', ctypes.c_ulong), ('message_type', ctypes.c_ulong),
                    ('format', ctypes.c_int), ('data', ctypes.c_long * 5)]
    display = x11.XOpenDisplay(None)
    event = ClientMessage(33, 0, 1, display, int(window), x11.XInternAtom(display, b'WM_PROTOCOLS', 0), 32)
    event.data[0] = x11.XInternAtom(display, b'WM_DELETE_WINDOW', 0)
    x11.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
    x11.XSendEvent(display, int(window), 0, 0, ctypes.byref(event))
    x11.XFlush.argtypes = [ctypes.c_void_p]
    x11.XFlush(display)
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    x11.XCloseDisplay(display)

def visible(window):
    return 'IsViewable' in subprocess.check_output(['xwininfo', '-id', window], text=True)

def main():
    process = None
    server = ui.ThreadingHTTPServer(('127.0.0.1', 0), ui.Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='bb-tray-test-') as temporary:
            env = dict(os.environ, XDG_CONFIG_HOME=temporary, XDG_DATA_HOME=temporary,
                       XDG_STATE_HOME=temporary, XDG_CACHE_HOME=temporary,
                       WINIT_UNIX_BACKEND='x11', LIBGL_ALWAYS_SOFTWARE='1')
            env.pop('WAYLAND_DISPLAY', None)
            with open(ui.ARTIFACTS / 'tray-launch.log', 'w') as log:
                process = subprocess.Popen([str(ui.ROOT / 'target/release/bluebubbles-linux')], env=env, stdout=log, stderr=log)
                ui.wait_for(lambda: bool(registered))
                time.sleep(2)
                window = ui.xdo('search', '--name', 'BlueBubbles').splitlines()[-1]
                ui.xdo('windowmove', window, 0, 0)
                ui.xdo('windowfocus', '--sync', window)
                ui.click_type(490, 390, f'http://127.0.0.1:{server.server_port}')
                ui.click_type(490, 460, 'smoke-test-password')
                ui.xdo('mousemove', 530, 518, 'click', 1)
                ui.wait_for(lambda: any(path.endswith('/message/query') for path, _ in ui.requests))
                time.sleep(1)
                assert notifications == [], 'Initial history generated an alert'
                ui.click_type(580, 625, 'Draft survives hiding')
                close_window(window)
                ui.wait_for(lambda: not visible(window))
                assert process.poll() is None, 'X quit instead of hiding'
                # A new incoming message while hidden must be polled and notified.
                ui.messages.append({'guid': 'tray-incoming', 'text': 'Private incoming text', 'isFromMe': False,
                                    'dateCreated': int(time.time() * 1000)})
                ui.wait_for(lambda: len(notifications) == 1, timeout=20)
                assert notifications == [('BlueBubbles', 'BlueBubbles', 'New message')], notifications
                time.sleep(4)
                assert len(notifications) == 1, 'Polling repeated the alert'
                ui.messages[-1]['dateRead'] = int(time.time() * 1000)
                time.sleep(4)
                assert len(notifications) == 1, 'Receipt generated an alert'
                remote = dbus.SessionBus(private=True)
                item = remote.get_object(registered[-1], '/StatusNotifierItem')
                properties = dbus.Interface(item, 'org.freedesktop.DBus.Properties')
                icon_path = Path(str(properties.Get('org.kde.StatusNotifierItem', 'IconName')))
                assert icon_path.is_absolute() and icon_path.is_file(), icon_path
                assert icon_path.read_bytes() == (ui.ROOT / 'assets/icon.png').read_bytes()
                assert str(properties.Get('org.kde.StatusNotifierItem', 'Status')) == 'Active'
                dbus.Interface(item, 'org.kde.StatusNotifierItem').Activate(0, 0)
                ui.wait_for(lambda: visible(window))
                ui.xdo('windowfocus', '--sync', window)
                time.sleep(0.4)
                ui.screenshot('tray-restored.png')
                # Focus remains on the draft after reopening.
                ui.xdo('mousemove', 580, 625, 'click', 1)
                ui.xdo('key', 'Return')
                ui.wait_for(lambda: any(body.get('message') == 'Draft survives hiding' for _, body in ui.requests))
                time.sleep(4)
                assert len(notifications) == 1, 'Outgoing message generated an alert'
                # Losing the panel must restore a hidden window; a returning panel
                # must register the icon again.
                close_window(window)
                ui.wait_for(lambda: not visible(window))
                bus.release_name('org.kde.StatusNotifierWatcher')
                ui.wait_for(lambda: visible(window))
                previous_registrations = len(registered)
                bus.request_name('org.kde.StatusNotifierWatcher')
                ui.wait_for(lambda: len(registered) > previous_registrations)
                time.sleep(0.3)
                # The menu must provide a real exit, even after hiding again.
                properties = dbus.Interface(item, 'org.freedesktop.DBus.Properties')
                menu_path = str(properties.Get('org.kde.StatusNotifierItem', 'Menu'))
                menu = dbus.Interface(remote.get_object(registered[-1], menu_path), 'com.canonical.dbusmenu')
                _, layout = menu.GetLayout(0, -1, [])
                quit_id = next(int(child[0]) for child in layout[2] if str(child[1].get('label')) == 'Quit BlueBubbles')
                close_window(window)
                ui.wait_for(lambda: not visible(window))
                menu.Event(quit_id, 'clicked', dbus.Int32(0, variant_level=1), dbus.UInt32(0))
                process.wait(timeout=10)
                assert process.returncode == 0
                for path in Path(temporary).rglob('*'):
                    if path.is_file():
                        content = path.read_bytes()
                        assert b'Private incoming text' not in content, path
                        assert b'smoke-test-password' not in content, path
                outcome.append(True)
                print('Tray smoke passed: X hides; hidden polling alerts once; icon restores; draft survives; outgoing/receipts stay quiet; panel recovery works; Quit exits')
    except Exception:
        traceback.print_exc()
        outcome.append(False)
    finally:
        if process and process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
        server.shutdown()
        GLib.idle_add(loop.quit)

threading.Thread(target=main, daemon=True).start()
loop.run()
assert outcome == [True], outcome

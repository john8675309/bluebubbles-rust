#!/usr/bin/env python3
"""Run under dbus-run-session + xvfb-run; verifies real XFCE icon painting/clicks."""
import os
from pathlib import Path
import subprocess
import tempfile
import time
from PIL import Image
import dbus

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / 'dist/screenshots/xfce-tray'
ARTIFACTS.mkdir(parents=True, exist_ok=True)

def run(*args):
    return subprocess.check_output(args, text=True).strip()

def wait_for(predicate, timeout=15):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            if predicate(): return
        except (subprocess.CalledProcessError, dbus.DBusException):
            pass
        time.sleep(.2)
    raise AssertionError('Timed out waiting for XFCE/app state')

with tempfile.TemporaryDirectory(prefix='bb-xfce-tray-') as temporary:
    env = dict(os.environ, XDG_CONFIG_HOME=temporary, XDG_CACHE_HOME=temporary,
               XDG_DATA_HOME=temporary, XDG_STATE_HOME=temporary,
               WINIT_UNIX_BACKEND='x11', LIBGL_ALWAYS_SOFTWARE='1')
    env.pop('WAYLAND_DISPLAY', None)
    directory = Path(temporary) / 'xfce4/xfconf/xfce-perchannel-xml'
    directory.mkdir(parents=True)
    (directory / 'xfce4-panel.xml').write_text('''<?xml version="1.0" encoding="UTF-8"?>
<channel name="xfce4-panel" version="1.0">
  <property name="configver" type="int" value="2"/>
  <property name="panels" type="array"><value type="int" value="1"/>
    <property name="panel-1" type="empty">
      <property name="position" type="string" value="p=10;x=550;y=810"/>
      <property name="position-locked" type="bool" value="true"/>
      <property name="size" type="uint" value="48"/>
      <property name="length" type="double" value="20"/>
      <property name="plugin-ids" type="array"><value type="int" value="1"/></property>
    </property>
  </property>
  <property name="plugins" type="empty"><property name="plugin-1" type="string" value="systray"/></property>
</channel>''')
    # D-Bus activated xfconfd must see the isolated config too.
    subprocess.run(['dbus-update-activation-environment', f'XDG_CONFIG_HOME={temporary}'], check=True)
    processes = []
    with open(ARTIFACTS / 'launch.log', 'w') as log:
        try:
            for command in [['xfwm4', '--compositor=off'], ['xfce4-panel', '--disable-wm-check'],
                            [os.environ.get('BB_TRAY_BINARY', str(ROOT / 'target/release/bluebubbles-linux'))]]:
                processes.append(subprocess.Popen(command, env=env, stdout=log, stderr=log))
                time.sleep(1)
            bus = dbus.SessionBus()
            watcher = dbus.Interface(bus.get_object('org.kde.StatusNotifierWatcher', '/StatusNotifierWatcher'), 'org.freedesktop.DBus.Properties')
            wait_for(lambda: len(watcher.Get('org.kde.StatusNotifierWatcher', 'RegisteredStatusNotifierItems')) == 1)
            window = run('xdotool', 'search', '--name', 'BlueBubbles').splitlines()[-1]
            run('xdotool', 'windowactivate', '--sync', window)
            run('xdotool', 'key', 'alt+F4')
            wait_for(lambda: 'IsUnMapped' in run('xwininfo', '-id', window))
            assert processes[-1].poll() is None
            time.sleep(1)
            panel = 'root'
            screenshot = ARTIFACTS / 'icon.png'
            subprocess.run(['import', '-window', panel, str(screenshot)], check=True)
            im = Image.open(screenshot).convert('RGB')
            blue = [(x,y) for y in range(im.height) for x in range(im.width)
                    if (lambda r,g,b: b > 110 and b > r * 1.25 and g > r * 1.1)(*im.getpixel((x,y))) ]
            assert len(blue) > 50, f'No visible BlueBubbles icon: {len(blue)} blue pixels'
            geometry = {'X': 0, 'Y': 0}
            x = int(geometry['X']) + sum(x for x,y in blue)//len(blue)
            y = int(geometry['Y']) + sum(y for x,y in blue)//len(blue)
            run('xdotool', 'mousemove', str(x), str(y), 'click', '1')
            wait_for(lambda: 'IsViewable' in run('xwininfo', '-id', window))
            print(f'XFCE tray passed: visible BlueBubbles icon ({len(blue)} blue pixels), actual icon click restores the window')
        finally:
            for process in reversed(processes):
                if process.poll() is None: process.terminate()
            for process in processes:
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired: process.kill()

#!/usr/bin/env python3
"""Exercise server contact-name editing against a local fake Mac server."""
import os
os.environ['BB_SMOKE_PRIVATE']='1'
os.environ.setdefault('BB_SMOKE_VARIANT','contact-editor')
import ui_smoke as ui
import json
import ctypes
import time
import threading
import tempfile
import subprocess
from urllib.parse import urlsplit

ui.messages[:]=[{'guid':'contact-message','text':'Edit a contact name','dateCreated':int(time.time()*1000)}]
contact={"id":7,"sourceType":"db","displayName":"Alex Morgan","firstName":"Alex","lastName":"Morgan","emails":[{"address":"alex@example.com"}],"phoneNumbers":[]}
class Handler(ui.Handler):
    def do_PUT(self):self.respond()
    def respond(self):
        path=urlsplit(self.path).path
        if path.endswith(('/contact','/contact/capabilities','/contact/7')):
            body=json.loads(self.rfile.read(int(self.headers.get('Content-Length','0'))) or b'{}')
            ui.requests.append((path,body))
            if path.endswith('/capabilities'):data={'localContactNames':True}
            elif self.command=='PUT':
                assert path.endswith('/contact/7')
                assert set(body)=={'displayName'}
                contact['displayName']=body['displayName'];data=contact
            else:data=[contact]
            payload=json.dumps({'status':200,'data':data}).encode()
            self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
        else:super().respond()
def click(x,y):
    ui.xdo('mousemove',x,y,'click',1);time.sleep(.3)
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


server=ui.ThreadingHTTPServer(('127.0.0.1',0),Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
with tempfile.TemporaryDirectory(prefix='bb-private-ui-') as temporary:
    env=dict(os.environ,XDG_CONFIG_HOME=temporary,XDG_DATA_HOME=temporary,XDG_STATE_HOME=temporary,XDG_CACHE_HOME=temporary,WINIT_UNIX_BACKEND='x11',LIBGL_ALWAYS_SOFTWARE='1')
    env.pop('WAYLAND_DISPLAY',None)
    with open(ui.ARTIFACTS/'launch.log','w') as log:
        app=subprocess.Popen([str(ui.ROOT/'target/release/bluebubbles-linux')],env=env,stdout=log,stderr=log)
        try:
            time.sleep(2)
            window=ui.xdo('search','--name','BlueBubbles').splitlines()[-1]
            ui.xdo('windowmove',window,0,0);ui.xdo('windowfocus','--sync',window)
            ui.click_type(490,390,f'http://127.0.0.1:{server.server_port}')
            ui.click_type(490,460,'smoke-test-password');click(530,518)
            ui.wait_for(lambda:any(path.endswith('/message/query') for path,_ in ui.requests))
            time.sleep(.5);ui.screenshot('ready.png')
            click(520,92)
            ui.screenshot('contact-editor.png')
            ui.wait_for(lambda:any(path.endswith('/contact/capabilities') for path,_ in ui.requests))
            time.sleep(.3)
            ui.click_type(180,165,'Friendly Name');ui.xdo('key','Return')
            ui.wait_for(lambda:any(path.endswith('/contact/7') for path,_ in ui.requests))
            time.sleep(.5);ui.screenshot('saved-name.png')
            assert contact['displayName']=='Friendly Name'
            assert contact['emails']==[{'address':'alex@example.com'}]
            close_window(window);app.wait(timeout=10)
            files=list(__import__('pathlib').Path(temporary).rglob('*.ron'))
            assert all('Friendly Name' not in path.read_text() for path in files),'Name was stored as a local override'
            print('Click-to-edit saves to the server contact ID, preserves addresses, and stores no local override')
        finally:
            app.terminate();app.wait(timeout=10);server.shutdown()

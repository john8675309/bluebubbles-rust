#!/usr/bin/env python3
"""Exercise actual Private API GUI actions against a local fake Mac server."""
import os
os.environ['BB_SMOKE_PRIVATE']='1'
os.environ.setdefault('BB_SMOKE_VARIANT','private-actions')
import ui_smoke as ui
import json
import time
import threading
import tempfile
import subprocess
from urllib.parse import urlsplit

ui.chat['guid']='iMessage;+;private-group'
ui.chat['participants']=[{'address':'alex@example.com'},{'address':'jamie@example.com'}]
ui.messages[:]=[{'guid':'private-original','text':'Original text','isFromMe':True,'dateCreated':int(time.time()*1000),'dateDelivered':int(time.time()*1000)}]
ui.chat['lastMessage']=ui.messages[0]

class Handler(ui.Handler):
    def do_PUT(self): self.respond()
    def do_DELETE(self): self.respond()
    def respond(self):
        path=urlsplit(self.path).path
        if path.endswith(('/edit','/unsend','/read','/unread','/react','/typing','/participant/add','/participant/remove','/leave')) or self.command=='PUT':
            body=json.loads(self.rfile.read(int(self.headers.get('Content-Length','0'))) or b'{}')
            ui.requests.append((path,body))
            if path.endswith('/edit'):
                ui.messages[0].update(text=body['editedMessage'],dateEdited=int(time.time()*1000))
            elif path.endswith('/unsend'):
                ui.messages[0].update(text=None,dateEdited=int(time.time()*1000),dateRetracted=int(time.time()*1000))
            payload=json.dumps({'status':200,'data':ui.messages[0] if path.endswith(('/edit','/unsend')) else {}}).encode()
            self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(payload))); self.end_headers(); self.wfile.write(payload)
        else: super().respond()

def click(x,y):
    ui.xdo('mousemove',x,y,'click',1);time.sleep(.3)

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
            if not os.environ.get('BB_PRIVATE_CAPTURE'):
                click(393,398)
                ui.screenshot('message-menu.png')
                click(390,561)
                ui.screenshot('edit-dialog.png')
                ui.click_type(160,148,'Edited through the GUI')
                click(80,238)
                ui.wait_for(lambda:any(path.endswith('/edit') for path,_ in ui.requests))
                assert ui.messages[0]['text']=='Edited through the GUI'
                time.sleep(.5)
                ui.screenshot('edited.png')
                click(393,398);click(390,481)
                ui.click_type(580,625,'Reply through the GUI');ui.xdo('key','Return')
                ui.wait_for(lambda:any(path.endswith('/message/text') for path,_ in ui.requests))
                sent=next(body for path,body in ui.requests if path.endswith('/message/text'))
                assert sent['method']=='private-api' and sent['selectedMessageGuid']=='private-original',sent
            print('Private API GUI edit and reply passed')
        finally:
            app.terminate();app.wait(timeout=10);server.shutdown()

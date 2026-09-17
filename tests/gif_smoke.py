#!/usr/bin/env python3
"""Verify automatic GIF animation against a local mock server. Requires Pillow."""
import os
os.environ['BB_SMOKE_PRIVATE']='1'
os.environ.setdefault('BB_SMOKE_VARIANT','gif-autoplay')
import ui_smoke as ui
import json
import time
import threading
import tempfile
import subprocess
from urllib.parse import urlsplit

from pathlib import Path
from PIL import Image
fixtures=tempfile.TemporaryDirectory(prefix="bb-media-fixtures-")
image_path=Path(fixtures.name)/"animation.gif"
video_path=Path(fixtures.name)/"clip.mp4"
frame_count=302 if os.environ.get('BB_SMOKE_LARGE_GIF') else 2
frames=[Image.new('RGB',(240,100),(255,0,0) if i%2==0 else (0,0,255)) for i in range(frame_count)]
frames[0].save(image_path,save_all=True,append_images=frames[1:],duration=[200,300]*(frame_count//2),loop=0)
from urllib.parse import parse_qs
ui.messages[:]=[{'guid':'gif-autoplay','text':'Inline media test','dateCreated':int(time.time()*1000),'attachments':[
    {'guid':'test-image','mimeType':'image/gif','transferName':'animation.gif'}]}]
class Handler(ui.Handler):
    def respond(self):
        target=urlsplit(self.path)
        if '/attachment/' in target.path:
            assert parse_qs(target.query)['guid']==['smoke-test-password']
            ui.requests.append((target.path,{}))
            is_image='test-image' in target.path
            if is_image: assert parse_qs(target.query)['original']==['true']
            payload=(image_path if is_image else video_path).read_bytes()
            self.send_response(200);self.send_header('Content-Type','image/gif' if is_image else 'video/mp4');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
        else:super().respond()
def click(x,y):
    ui.xdo('mousemove',x,y,'click',1);time.sleep(.3)
server=ui.ThreadingHTTPServer(('127.0.0.1',0),Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
with tempfile.TemporaryDirectory(prefix='bb-private-ui-') as temporary:
    env=dict(os.environ,XDG_CONFIG_HOME=temporary,XDG_DATA_HOME=temporary,XDG_STATE_HOME=temporary,XDG_CACHE_HOME=temporary,WINIT_UNIX_BACKEND='x11',LIBGL_ALWAYS_SOFTWARE='1')
    env.pop('WAYLAND_DISPLAY',None)
    with open(ui.ARTIFACTS/'launch.log','w') as log:
        app=subprocess.Popen([os.environ.get('BB_SMOKE_BINARY',str(ui.ROOT/'target/release/bluebubbles-linux'))],env=env,stdout=log,stderr=log)
        try:
            time.sleep(2)
            window=ui.xdo('search','--name','BlueBubbles').splitlines()[-1]
            ui.xdo('windowmove',window,0,0);ui.xdo('windowfocus','--sync',window)
            ui.click_type(490,390,f'http://127.0.0.1:{server.server_port}')
            ui.click_type(490,460,'smoke-test-password');click(530,518)
            ui.wait_for(lambda:any(path.endswith('/message/query') for path,_ in ui.requests))
            time.sleep(.5);ui.screenshot('ready.png')
            ui.wait_for(lambda:any('test-image/download' in path for path,_ in ui.requests))
            observed=set()
            for index in range(10):
                ui.screenshot(f'gif-{index}.png')
                colors=Image.open(ui.ARTIFACTS/f'gif-{index}.png').convert('RGB').crop((355,250,850,530)).getdata()
                red=sum(r>240 and g<10 and b<10 for r,g,b in colors)
                blue=sum(b>240 and r<10 and g<10 for r,g,b in colors)
                if red>1000:observed.add('red')
                if blue>1000:observed.add('blue')
                time.sleep(.09)
            assert observed=={'red','blue'},f'GIF did not autoplay: {observed}'
            ui.click_type(580,625,'GIF playback keeps messaging responsive');ui.xdo('key','Return')
            ui.wait_for(lambda:any(path.endswith('/message/text') for path,_ in ui.requests))
            assert sum('test-image/download' in path for path,_ in ui.requests)==1
            print('GIF autoplays, loops, downloads once, and allows messaging')
        finally:
            app.terminate();app.wait(timeout=10);server.shutdown()

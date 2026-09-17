#!/usr/bin/env python3
"""Verify inline image/video rendering against a local mock server. Requires ffmpeg/Pillow."""
import os
os.environ['BB_SMOKE_PRIVATE']='1'
os.environ.setdefault('BB_SMOKE_VARIANT','inline-media')
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
image_path=Path(fixtures.name)/"photo.png"
video_path=Path(fixtures.name)/"clip.mp4"
Image.new("RGB",(240,100),(255,0,0)).save(image_path)
subprocess.run(["ffmpeg","-hide_banner","-loglevel","error","-y","-f","lavfi","-i","testsrc2=size=320x180:rate=24","-t","8","-c:v","libx264","-pix_fmt","yuv420p",str(video_path)],check=True)
from urllib.parse import parse_qs
ui.messages[:]=[{'guid':'inline-media','text':'Inline media test','dateCreated':int(time.time()*1000),'attachments':[
    {'guid':'test-image','mimeType':'image/png','transferName':'photo.png'},
    {'guid':'test-video','mimeType':'video/mp4','transferName':'clip.mp4'}]}]
class Handler(ui.Handler):
    def respond(self):
        target=urlsplit(self.path)
        if '/attachment/' in target.path:
            assert parse_qs(target.query)['guid']==['smoke-test-password']
            ui.requests.append((target.path,{}))
            is_image='test-image' in target.path
            if is_image: assert parse_qs(target.query)['width']==['1200']
            payload=(image_path if is_image else video_path).read_bytes()
            self.send_response(200);self.send_header('Content-Type','image/png' if is_image else 'video/mp4');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
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
            time.sleep(1);ui.screenshot('image-loaded.png')
            pixels=Image.open(ui.ARTIFACTS/'image-loaded.png').convert('RGB')
            assert pixels.getpixel((500,270))==(255,0,0), 'Inline image not rendered'
            click(580,379)
            ui.wait_for(lambda:any('test-video/download' in path for path,_ in ui.requests))
            time.sleep(2);ui.screenshot('video-playing.png')
            pixels=Image.open(ui.ARTIFACTS/'video-playing.png').convert('RGB')
            assert sum(g>180 and r<100 for r,g,b in pixels.crop((370,250,850,310)).getdata())>100, 'Inline video frame not rendered'
            click(402,340)
            ui.screenshot('paused.png');time.sleep(.6);ui.screenshot('still-paused.png')
            def frame(name):return Image.open(ui.ARTIFACTS/name).crop((370,250,850,310)).tobytes()
            assert frame('paused.png')==frame('still-paused.png'), 'Pause did not stop frames'
            ui.xdo('mousemove',405,378,'mousedown',1);time.sleep(.2);ui.xdo('mousemove',457,378);time.sleep(.3);ui.xdo('mouseup',1);time.sleep(.4);ui.screenshot('seek.png')
            assert frame('seek.png')!=frame('paused.png'), 'Seek did not change the frame'
            click(538,340)
            ui.click_type(580,625,'Messaging remains responsive');ui.xdo('key','Return')
            ui.wait_for(lambda:any(path.endswith('/message/text') for path,_ in ui.requests))
            assert sum('test-image/download' in path for path,_ in ui.requests)==1, 'Image was downloaded repeatedly'
            print('Inline image, video, pause, seek, stop, and responsive send passed')
        finally:
            app.terminate();app.wait(timeout=10);server.shutdown()

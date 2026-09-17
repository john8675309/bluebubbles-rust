#!/usr/bin/env python3
"""Generate rotated clips and verify libmpv output pixels and aspect ratios."""
import os
from pathlib import Path
import subprocess
import tempfile

with tempfile.TemporaryDirectory(prefix='bb-video-orientation-') as directory:
    root=Path(directory)
    def ffmpeg(*args):
        subprocess.run(['ffmpeg','-hide_banner','-loglevel','error','-y',*map(str,args)],check=True)
    ffmpeg('-f','lavfi','-i','color=c=red:size=320x180:rate=10,drawbox=x=0:y=90:w=320:h=90:color=blue:t=fill','-t','2','-c:v','libx264',root/'0.mp4')
    for rotation in [90,180,270]:
        ffmpeg('-display_rotation',-rotation,'-i',root/'0.mp4','-c','copy',root/f'{rotation}.mp4')
    subprocess.run([os.environ.get('CARGO_BIN','cargo'),'test','--locked','--bin','bluebubbles-linux','video_rotation_metadata','--','--ignored'],env=dict(os.environ,BB_ROTATION_DIR=directory),check=True,cwd=Path(__file__).resolve().parents[1])

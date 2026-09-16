#!/usr/bin/env python3
"""Run under xvfb-run. Uses a local fake server; never sends real messages."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / "dist" / "screenshots"
SCALE = float(os.environ.get("BB_SMOKE_SCALE", "1"))
if variant := os.environ.get("BB_SMOKE_VARIANT"):
    ARTIFACTS = ARTIFACTS / variant
ARTIFACTS.mkdir(parents=True, exist_ok=True)
requests = []
messages = [
    {"guid": "one", "text": "Welcome to BlueBubbles on Linux.", "isFromMe": False,
     "dateCreated": 1789486800000, "handle": {"address": "alex@example.com"}},
    {"guid": "two", "text": "A native Rust desktop client, connected to your Mac.", "isFromMe": True,
     "dateCreated": 1789486860000, "dateRead": 1789486870000},
    {"guid": "three", "text": "Conversation history, text messages, and file sharing are ready.",
     "isFromMe": False, "dateCreated": 1789486920000, "handle": {"address": "alex@example.com"},
     "associatedMessageType": "love", "associatedMessageGuid": "p:0/two"},
]
chat = {"guid": "iMessage;-;alex@example.com", "displayName": "Alex Morgan",
        "participants": [{"address": "alex@example.com"}], "lastMessage": messages[-1]}
lock = threading.Lock()
SLOW_REFRESH = bool(os.environ.get("BB_SMOKE_SLOW_REFRESH"))
PERSIST_HISTORY = bool(os.environ.get("BB_SMOKE_HISTORY"))
poll_started = threading.Event()
release_poll = threading.Event()
other_messages = [{"guid": "other", "text": "Another conversation", "dateCreated": 1789486000000}]
other_chat = {"guid": "iMessage;-;jamie@example.com", "displayName": "Jamie Lee",
              "participants": [{"address": "jamie@example.com"}], "lastMessage": other_messages[-1]}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        self.respond()

    def do_POST(self):
        self.respond()

    def respond(self):
        target = urlsplit(self.path)
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))) or b"{}")
        with lock:
            requests.append((target.path, body))
        status = 200
        if parse_qs(target.query).get("guid") != ["smoke-test-password"]:
            status, data = 401, None
        elif target.path.endswith("/server/info"):
            data = {"serverVersion": "1.9.9", "osVersion": "14.0"}
        elif target.path.endswith("/contact"):
            data = []
        elif target.path.endswith("/fcm/client"):
            data = {"project_info": {"project_id": "bb-smoke-test"},
                    "client": [{"client_info": {"mobilesdk_app_id": "1:123:android:test"},
                                "api_key": [{"current_key": "smoke-firebase-api-key"}]}]}
        elif target.path.endswith("/chat/query"):
            data = [dict(chat, lastMessage=messages[-1])]
            if SLOW_REFRESH:
                data.append(dict(other_chat, lastMessage=other_messages[-1]))
                if body.get("offset", 0) == 0 and sum(path.endswith("/chat/query") and request.get("offset", 0) == 0 for path, request in requests) == 2:
                    poll_started.set()
                    assert release_poll.wait(25), "Test did not release the paused refresh"
            data = data[body.get("offset", 0):body.get("offset", 0) + body.get("limit", 100)]
        elif target.path.endswith("/message/query"):
            history = other_messages if body.get("chatGuid") == other_chat["guid"] else messages
            data = list(reversed(history))
        elif target.path.endswith("/message/text"):
            if body["message"] == "SIMULATED FAILURE":
                status, data = 500, None
            else:
                message = {"guid": body["tempGuid"], "text": body["message"], "isFromMe": True,
                           "dateCreated": 1789487000000 + len(messages), "dateDelivered": 1789487001000}
                history = other_messages if body.get("chatGuid") == other_chat["guid"] else messages
                history.append(message)
                data = message
        else:
            status, data = 404, None
        payload = json.dumps({"status": status, "data": data}).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def xdo(*args):
    if args[0] == "mousemove":
        args = (args[0], round(float(args[1]) * SCALE), round(float(args[2]) * SCALE), *args[3:])
    return subprocess.check_output(["xdotool", *map(str, args)], text=True).strip()


def wait_for(predicate, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise AssertionError("Timed out waiting for expected UI/server action")


def screenshot(name):
    subprocess.run(["import", "-window", "root", str(ARTIFACTS / name)], check=True)


def click_type(x, y, text):
    xdo("mousemove", x, y, "click", 1)
    time.sleep(0.2)
    xdo("key", "ctrl+a")
    time.sleep(0.2)
    xdo("type", "--clearmodifiers", "--delay", 10, text)
    time.sleep(0.3)


def main():
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="bluebubbles-ui-") as temporary:
        env = dict(os.environ, XDG_CONFIG_HOME=temporary, XDG_DATA_HOME=temporary,
                   XDG_STATE_HOME=temporary, XDG_CACHE_HOME=temporary,
                   WINIT_UNIX_BACKEND="x11", LIBGL_ALWAYS_SOFTWARE="1")
        env.pop("WAYLAND_DISPLAY", None)
        headless_env = dict(env)
        headless_env.pop("DISPLAY", None)
        subprocess.run([str(ROOT / "target/release/bluebubbles-linux"), "--push-worker", str(Path(temporary) / "disabled.json")], env=headless_env, timeout=10, check=True)
        with open(ARTIFACTS / "launch.log", "w") as log:
            process = subprocess.Popen([str(ROOT / "target/release/bluebubbles-linux")], env=env, stdout=log, stderr=log)
            try:
                time.sleep(2)
                assert process.poll() is None, "Application exited at startup"
                window = xdo("search", "--name", "BlueBubbles").splitlines()[-1]
                xdo("windowmove", window, 0, 0)
                xdo("windowfocus", "--sync", window)
                time.sleep(0.5)
                if os.environ.get("BB_SMOKE_LIGHT"):
                    xdo("mousemove", 1035, 28, "click", 1)
                    time.sleep(0.3)
                geometry = dict(line.split("=", 1) for line in xdo("getwindowgeometry", "--shell", window).splitlines())
                assert int(geometry["WIDTH"]) == round(1100 * SCALE), geometry
                assert int(geometry["HEIGHT"]) == round(760 * SCALE), geometry
                screenshot("connection.png")
                if os.environ.get("BB_SMOKE_CAPTURE_ONLY"):
                    print("Captured connection screen")
                    return
                click_type(490, 390, f"http://127.0.0.1:{server.server_port}")
                click_type(490, 460, "smoke-test-password")
                if PERSIST_HISTORY:
                    xdo("mousemove", 356, 574, "click", 1)
                xdo("mousemove", 530, 518, "click", 1)
                wait_for(lambda: any(path.endswith("/message/query") for path, _ in requests))
                time.sleep(0.5)
                screenshot("conversation.png")
                if SLOW_REFRESH:
                    wait_for(poll_started.is_set)
                    # The server will not complete the poll until navigation and
                    # sending have succeeded: this reproduces the old disabled UI.
                    xdo("mousemove", 120, 330, "click", 1)
                    wait_for(lambda: any(path.endswith("/message/query") and body.get("chatGuid") == other_chat["guid"] for path, body in requests))
                    time.sleep(0.5)
                click_type(580, 625, "Hello from the Rust Linux client!")
                xdo("key", "shift+Return")
                time.sleep(0.3)
                assert not any(path.endswith("/message/text") for path, _ in requests), "Shift+Enter sent a message"
                xdo("type", "--clearmodifiers", "Second line")
                xdo("key", "Return")
                sent_history = other_messages if SLOW_REFRESH else messages
                wait_for(lambda: any(m.get("text") == "Hello from the Rust Linux client!\nSecond line" for m in sent_history))
                if SLOW_REFRESH:
                    assert not release_poll.is_set()
                    screenshot("interactive-during-refresh.png")
                    release_poll.set()
                time.sleep(0.5)
                xdo("mousemove", 470, 680, "click", 1)
                time.sleep(0.5)
                screenshot("emoji-picker.png")
                xdo("mousemove", 452, 490, "click", 1)
                time.sleep(0.3)
                xdo("key", "Return")
                wait_for(lambda: any(m.get("text") == "😀" for m in sent_history))
                time.sleep(0.3)
                screenshot("emoji-message.png")
                click_type(580, 625, "SIMULATED FAILURE")
                xdo("key", "ctrl+Return")
                wait_for(lambda: any(body.get("message") == "SIMULATED FAILURE" for _, body in requests))
                time.sleep(0.5)
                screenshot("send-failure.png")
                # The failed draft remains: a second explicit click sends the same text.
                xdo("key", "ctrl+Return")
                wait_for(lambda: sum(body.get("message") == "SIMULATED FAILURE" for _, body in requests) == 2)
                wait_for(lambda: any(path.endswith("/fcm/client") for path, _ in requests))
                if os.environ.get("BB_SMOKE_FIREBASE"):
                    xdo("mousemove", 735, 28, "click", 1)
                    time.sleep(0.5)
                    screenshot("firebase.png")
                if os.environ.get("BB_SMOKE_MINIMUM"):
                    xdo("windowsize", window, round(760 * SCALE), round(520 * SCALE))
                    time.sleep(0.5)
                    screenshot("minimum-window.png")
                # Close the app normally and verify that persisted settings contain no secrets/content.
                # Send WM_DELETE_WINDOW even when Xvfb has no window manager.
                import ctypes
                x11 = ctypes.CDLL("libX11.so.6")
                x11.XOpenDisplay.restype = ctypes.c_void_p
                x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
                x11.XInternAtom.restype = ctypes.c_ulong
                class ClientMessage(ctypes.Structure):
                    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong),
                                ("send_event", ctypes.c_int), ("display", ctypes.c_void_p),
                                ("window", ctypes.c_ulong), ("message_type", ctypes.c_ulong),
                                ("format", ctypes.c_int), ("data", ctypes.c_long * 5)]
                display = x11.XOpenDisplay(None)
                event = ClientMessage(33, 0, 1, display, int(window),
                                      x11.XInternAtom(display, b"WM_PROTOCOLS", 0), 32)
                event.data[0] = x11.XInternAtom(display, b"WM_DELETE_WINDOW", 0)
                x11.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int,
                                          ctypes.c_long, ctypes.c_void_p]
                x11.XSendEvent(display, int(window), 0, 0, ctypes.byref(event))
                x11.XFlush.argtypes = [ctypes.c_void_p]
                x11.XFlush(display)
                x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
                x11.XCloseDisplay(display)
                process.wait(timeout=10)
                for path in Path(temporary).rglob("*"):
                    if path.is_file():
                        content = path.read_bytes()
                        assert b"smoke-test-password" not in content, path
                        assert b"smoke-firebase-api-key" not in content, path
                        if not PERSIST_HISTORY:
                            assert b"SIMULATED FAILURE" not in content, path
                if PERSIST_HISTORY:
                    import sqlite3
                    databases = list(Path(temporary).rglob("history.sqlite3"))
                    assert len(databases) == 1, databases
                    with sqlite3.connect(databases[0]) as db:
                        assert db.execute("SELECT text FROM drafts").fetchall() == [("SIMULATED FAILURE",)]
                        rows = db.execute("SELECT payload FROM messages").fetchall()
                        assert any(json.loads(row[0]).get("text") == "Hello from the Rust Linux client!\nSecond line" for row in rows)
                    assert databases[0].stat().st_mode & 0o777 == 0o600
                    print("Optional history passed: saved messages/draft, private file permissions, no password")
                print("UI smoke passed: connect, load history, send, failed draft retry, no persisted secrets")
            finally:
                release_poll.set()
                if process.poll() is None:
                    screenshot("last-frame.png")
                    process.terminate()
                    process.wait(timeout=5)
                server.shutdown()


if __name__ == "__main__":
    main()

# BlueBubbles Rust

A lightweight Rust client for BlueBubbles.

A native Linux desktop client using egui/eframe, the BlueBubbles REST API, and
Socket.IO events. This independent community client requires a BlueBubbles Mac
server. Full feature parity is still in progress; see [PARITY.md](PARITY.md).

Based on the [BlueBubbles project](https://github.com/BlueBubblesApp/bluebubbles-app).
This is not an official BlueBubbles release.

## Downloads

Get the Debian package or portable Linux archive from [GitHub Releases](https://github.com/john8675309/bluebubbles-rust/releases).
The first release provides x86_64/amd64 builds. Checksums are included as `SHA256SUMS`.

## Run

Launch `bluebubbles-linux` from the extracted release archive, or from this checkout:

```sh
./target/release/bluebubbles-linux
```

Enter your BlueBubbles server URL and its password. You need a configured
[BlueBubbles server on a Mac](https://bluebubbles.app/install/)
signed in to Messages. This client does not replace the Mac server.
Direct HTTP(S) URLs, reverse-proxy path prefixes, and URLs ending in `/api/v1` are
accepted. HTTPS certificates are verified. Redirects are rejected; enter the final
server URL. For remote connections, use HTTPS.

## Included

- Server connection with password/key authentication.
- Complete conversation list sorted newest first, with participants and current
  latest-message previews. Older message history loads in pages.
- Message history with incoming/outgoing bubbles, timestamps, and delivery/read status.
- Text sending: Enter sends; Shift+Enter adds a line break (Ctrl+Enter also sends).
- Built-in emoji picker inserts at the cursor in existing and new conversations.
  Unicode emoji can also be pasted; bundled fonts render the picker in monochrome.
- New iMessage conversations with one or more recipients.
- Attachment uploads and downloads through native file dialogs.
- Live message/receipt events and incoming typing indicators through Socket.IO.
- Background catch-up every minute while live, or every three seconds while reconnecting.
  Polling keeps chat navigation and sending interactive; overlapping refreshes
  and duplicate loads of the same conversation are coalesced.
- Search within loaded conversations and messages; load older pages to search more.
- Server contact names in the conversation list and header.
- Optional SQLite history and draft persistence, isolated by server URL.
- Per-conversation drafts, preserved on failed sends; outgoing messages are never automatically retried.
- Refined light/dark themes, avatar-based conversation list, day separators,
  rounded message bubbles and composer; X11 and Wayland support.
- Automatic monitor scaling: respects desktop HiDPI settings and uses 200% on
  4K monitors reported at 100%, including portrait displays. Adjusts when moving
  between monitors; ordinary and ultrawide 1080p screens keep their normal scale.

The server URL, theme, and local-history preference are saved. Passwords stay in
memory. History and drafts stay in memory unless you enable “Keep message history
and drafts on this computer” before connecting. Enabled history is stored in
`$XDG_DATA_HOME/rust-linux/history.sqlite3` (normally `~/.local/share/rust-linux/`)
with private file permissions. Disabling history stops future reads/writes; it
does not erase previously saved history. Cached content is restored after login. A timed-out send may
still have reached the server: check the conversation before retrying.
Downloads require a new filename and never overwrite existing files.

## System tray and new-message alerts

Click the window's **X** to hide BlueBubbles in the system tray. The connection,
message sync, and unsent drafts stay active. Click the tray icon to reopen the
window, or use **Open BlueBubbles** in its menu. **Quit BlueBubbles** exits the GUI;
an explicitly enabled Firebase receiver can still deliver background notifications.

New incoming messages show a desktop notification, including while the window is
hidden. Initial history, outgoing messages, reactions, read receipts, and repeated
polls do not generate alerts. Previews are off by default; the Firebase panel's
preview option also controls connected-session alerts. The app does not steal
focus when a message arrives. Your desktop's Do Not Disturb settings still apply.

The tray uses [StatusNotifierItem through ksni](https://github.com/iovxw/ksni).
The embedded BlueBubbles icon is cached as a PNG for panel compatibility,
including portable builds. XFCE needs its Status Notifier/AppIndicator panel plugin; GNOME needs an
AppIndicator extension. If no tray host is available, X exits normally instead
of leaving an inaccessible hidden window. If the tray disappears while hidden,
the window is restored. Wayland compositor focus policies can limit activation.

GUI and Firebase alerts share a private, seven-day message-ID ledger under
`~/.local/share/rust-linux/notifications/`. It stores no message bodies or passwords.
Encrypted Firebase alerts have no message ID; a short-lived healthy-session marker
lets the GUI handle those alerts while connected, with Firebase taking over when
the marker expires. The notification service must be running in your desktop session.

## Firebase and background notifications

Open **Firebase** in the toolbar after connecting. The client loads your Mac's
`google-services.json` configuration automatically. You can also import that
**client configuration** file from the Firebase panel. Do not import a service-account key.

- **URL recovery:** reads `server/config.serverUrl` from Firestore or
  `config/serverUrl` from Realtime Database. Recovers HTTPS tunnel URLs after
  connection failures, with at most one lookup per minute. Existing session
  credentials verify the discovered server before it replaces the old URL.
  You can also use **Recover URL now**. No Mac password is sent to Firebase.
- **Closed-window notifications:** choose **Enable / update notifications**.
  This registers an FCM receiver with Google and your Mac, then starts a separate
  Rust process. It continues after the GUI exits. **Start the notification
  receiver at login** optionally creates a desktop autostart entry.
- **Privacy:** notifications say “New message” by default. Enable previews if
  desired. The receiver stores its FCM registration/decryption keys and recent
  delivery IDs with private file permissions, without storing your Mac password
  or message bodies. Encrypted BlueBubbles payloads use a generic notification.
- **Controls:** use **Test desktop notification**, **Disable notifications**, or
  **Reset registration** in the Firebase panel. Disable stops the receiver and
  removes its autostart entry. Reset also removes the local registration so the
  next enable obtains a new token. Disconnecting the messaging session does not
  disable notifications; use the explicit notification control.

Configuration lives in the app settings; notification state lives under
`~/.local/share/rust-linux/push/` (or your XDG data directory). Keep the executable
at its current location if you enable startup at login; re-enable notifications
after moving an extracted portable build. A desktop notification service and an
active user session are required. Closing the GUI works; powering off the computer
or logging out stops delivery until the receiver starts again.

Firebase database reads use the same unauthenticated database-rule access as the
original desktop client. Projects requiring Google/Firebase sign-in are not yet
supported. FCM reception uses the community
[fcm-push-listener](https://github.com/RandomEngy/fcm-push-listener) implementation;
Google does not supply this Rust desktop receiver. Local payload, persistence,
HTTP, and desktop-bus tests pass; Google registration and delivery from a real Mac
still require live verification. Notification click-to-open/reply is unfinished.

Protocol references: [Firestore REST](https://firebase.google.com/docs/firestore/use-rest-api)
and [Realtime Database REST](https://firebase.google.com/docs/database/rest/retrieve-data).

## Current limits

### Cloudflare connection errors

HTTP 530 with Cloudflare error 1033 means the tunnel is unavailable. Check that
BlueBubbles Server and its tunnel are running on the Mac, then use the current
server URL displayed there. A saved temporary tunnel URL may no longer be valid.
On the same network, the server's local address bypasses Cloudflare. The client
identifies this error for both API requests and attachment downloads without
displaying raw error pages or credentials.

### Feature coverage

Full parity is the target; see [PARITY.md](PARITY.md) for remaining work. The app must remain running (visible or in the tray) for chat synchronization. The optional
FCM receiver runs independently for notifications when the window is closed. Offline login/search, cache management,
keyring login, inline media playback,
tapback/thread presentation, private-API editing/unsending,
scheduled sending, and theme/skin parity with Flutter remain unfinished. Attachments can be saved
and opened in another application. Participant addresses use server contact names where available. The latest message page is refreshed for receipts;
older cached messages are not continuously re-fetched for edits/deletions.

## Build

Use Rust **1.89 or newer** and a Linux C toolchain (`build-essential` and
`pkg-config`, `cmake`, `protobuf-compiler` (protoc), and `libssl-dev` on Debian/Ubuntu). Runtime support requires Mesa/OpenGL, X11 or
Wayland, OpenSSL 3, libxkbcommon, and a desktop portal backend for file dialogs. GTK
development headers are not required.

```sh
git clone https://github.com/john8675309/bluebubbles-rust.git
cd bluebubbles-rust
cargo build --release --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

On machines where `/usr/bin/cargo` is older than the rustup toolchain, use
`~/.cargo/bin/cargo` for these commands. `Cargo.lock` pins the dependency set.
The supplied build was compiled with Rust 1.98 on x86_64 Linux.

## Packaging

```sh
bash packaging/package.sh
```

Creates a `.tar.gz` with the executable, icon, desktop entry, and documentation.
When `dpkg-deb` is available, also creates a Debian package. Install that with
`sudo apt install ./dist/bluebubbles-linux_0.1.0_amd64.deb` on compatible Debian/Ubuntu
systems, or run the archive's executable directly. Installation is optional.
Packages require the build host's glibc version or newer as recorded in the
Debian metadata; these are not universal static binaries.

## Validation

`tests/api_contract.rs` uses local mock HTTP servers to check authentication,
request formats, pagination, Unicode sending, error handling, response parsing,
multipart uploads, download permissions, and duplicate/receipt merging.
`tests/realtime_contract.rs` checks the authenticated Socket.IO polling handshake
and live delivery. Cache tests check restored drafts/history and server isolation.
No test sends a real message or needs Mac credentials.

The GUI smoke test needs Python 3, Xvfb, xdotool, and ImageMagick:

```sh
xvfb-run -a -s '-screen 0 1100x760x24' python3 tests/ui_smoke.py
xvfb-run -a -s '-screen 0 1100x760x24' env BB_SMOKE_SLOW_REFRESH=1 BB_SMOKE_VARIANT=slow-refresh python3 tests/ui_smoke.py
xvfb-run -a -s '-screen 0 3840x2160x24' env WINIT_X11_SCALE_FACTOR=1 BB_SMOKE_SCALE=2 BB_SMOKE_HISTORY=1 BB_SMOKE_VARIANT=4k-history python3 tests/ui_smoke.py
```

It connects to a local mock, loads a conversation, sends text, retries a failed
draft, and checks that settings contain no password or draft content.
The slow-refresh variant holds a poll open until switching to another chat and
sending a message succeed, to catch accidental disabling of the conversation list.
The 4K-history variant checks scaling and opt-in SQLite persistence without storing
the password.
`dbus-run-session -- python3 tests/notification_smoke.py` verifies delivery to an
isolated desktop notification service (requires Python dbus and GLib bindings).
`dbus-run-session -- xvfb-run -a -s '-screen 0 1100x760x24' python3 tests/tray_smoke.py`
checks close-to-tray, icon activation, hidden polling, notification deduplication,
draft preservation, and explicit Quit with isolated mock desktop services.
Screenshots are written to `dist/screenshots/`. A real Mac server and Wayland desktop still
need end-to-end validation.

## Source layout

- `src/api.rs`: HTTP transport and server API operations.
- `src/realtime.rs`: Socket.IO connection and event decoding.
- `src/firebase.rs`: Firebase config import and HTTPS URL discovery.
- `src/push.rs`: FCM registration, background receiver, and desktop notifications.
- `src/cache.rs`: optional SQLite history and drafts on a worker thread.
- `src/api_actions.rs`: additional API operations and contact mapping.
- `src/model.rs`: server data models and history merging.
- `src/app.rs`: session state, background work, polling, and drafts.
- `src/views.rs`, `src/theme.rs`, `src/widgets.rs`: desktop interface and shared visuals.
- `src/composer.rs`: keyboard sending, multiline editing, and emoji picker.
- `src/tray.rs`: system tray, window restoration, and explicit quit.
- `src/notifications.rs`: incoming-message alerts and shared Firebase deduplication.
- `src/display.rs`: automatic monitor scaling and window sizing.
- `packaging/`: launcher and local distribution script.

The API implementation was checked against the upstream client and server
sources. Licensed under Apache-2.0; see [LICENSE](LICENSE) and [NOTICE](NOTICE)
for upstream attribution, including the bundled BlueBubbles icon.

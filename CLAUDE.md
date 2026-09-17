# Rust Linux Client

This standalone Cargo package implements a native Linux messaging client.
It uses the BlueBubbles Mac server API and is independent of the upstream Flutter
build. Source assets must stay inside this repository. Preserve upstream
attribution in NOTICE and the Apache-2.0 LICENSE.

`src/api.rs` implements the existing BlueBubbles REST API; `src/model.rs` contains
typed payloads. `src/app.rs` owns session state and receives background results
tagged by session generation to reject responses after disconnect. Views stay in
`src/views.rs`; the light/dark palette and spacing are in `src/theme.rs`.
`src/composer.rs` handles Enter-to-send, Shift+Enter line breaks, IME-safe
shortcuts, and Unicode emoji insertion at the selection.
`src/widgets.rs` draws reusable avatars, accessible conversation rows, the brand
mark, and date separators. Keep chat row IDs stable and preserve keyboard focus. `src/display.rs` adapts
scaling to the current monitor, with a 2x fallback for unscaled 4K displays.

`src/tray.rs` owns the StatusNotifierItem service on a background thread. Cancel
window close only with a registered tray host; explicit Quit must bypass that guard.
Serve the embedded icon as a cached PNG via IconName as well as ARGB pixmaps;
portable builds must not depend on an installed icon theme.
Restore the window when the host disappears. `src/notifications.rs` filters new
messages and serializes GUI/FCM notification delivery using a private ID-only ledger.
Never notify on initial history, old pages, receipts, or outgoing messages. Keep
notification D-Bus and storage operations off the UI thread.

All network and attachment work runs off the UI thread.
Load all conversation pages before sorting globally by latest message activity;
the server sorts only within each page. Keep read receipts from promoting chats,
and retain newer previews when stale snapshots or older history pages arrive.
Background refresh and per-chat history loads have independent in-flight state;
only foreground operations use `busy`. Keep conversation navigation enabled,
deduplicate per-chat loads, and route late responses to their original chat.
Never let background completion clear a pending foreground send's busy state.
Never log authenticated URLs or persist passwords. Store messages/drafts only when
the user enables local history. Keep each server URL isolated in the SQLite cache.
`src/realtime.rs` receives Socket.IO events; `src/cache.rs` owns background SQLite
work. `src/api_actions.rs` contains additional REST operations and contact mapping.
`src/firebase.rs` reads Firebase URL configuration. `src/push.rs` owns opt-in FCM
registration and the separate `--push-worker` process. Keep Google requests free
of Mac passwords. Persist receiver keys only in private files, never raw payloads.
URL recovery must not interrupt sends or apply results to a different session.
Do not automatically retry message sends.
Keep API behavior aligned with the upstream BlueBubbles client and server contracts.
Changes to API/state behavior should be covered by meaningful tests.

Build/test/package commands and current feature limitations are in README.md.

`src/private_api.rs` and `src/private_views.rs` gate Private API actions against
server/helper/macOS capabilities and edit/unsend time limits. Keep per-chat composer
options on failed sends; never automatically retry or downgrade private actions.
`src/media.rs` owns bounded image/GIF decoding and UI textures. GIFs request original
bytes and animate only in visible views. Decode GIFs on demand on a worker with
one prefetched frame; never cache the complete decoded animation. Account for
full-size compositing buffers in cache memory estimates. `src/video.rs` keeps
libmpv and decoding on one worker, passing only frames/state to egui. Video buffers
must remain anonymous temporary files; stop playback on chat change, hide, and
disconnect. Never pass authenticated server URLs to media players.

`src/contact_editor.rs` edits server-managed display names using the companion
patch in `server-patches/`. Gate writes on its capability endpoint; never silently
save app-local names or simulate renaming with the stock name-matched create API.
Keep exact server contact IDs, recipient addresses, avatars, and unrelated names
unchanged. Native macOS Contacts writes remain unsupported.

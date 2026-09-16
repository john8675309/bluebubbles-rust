# Linux feature parity

Target: the original BlueBubbles **Linux desktop** client, including features
enabled by the connected Mac's capabilities. Android-only integrations are not
Linux requirements. This is an implementation and verification checklist, not a
claim that parity has been reached.

Baseline reviewed: upstream client `e2eaced6e`, especially
`lib/services/network/api/`, `lib/app/layouts/`, desktop settings, and socket events.

| Area | Current state | Remaining work / acceptance |
| --- | --- | --- |
| Firebase | Firestore/RTDB URL recovery and config import implemented; local tests | Google sign-in; live project verification |
| FCM notifications | Separate Rust receiver, registration, deduplication, optional login startup, desktop notification controls | Live Google/Mac delivery verification; notification actions and click-to-open; richer encrypted previews |
| Direct server login | Implemented, mock tested | Secure remembered login; setup/import; custom headers; connection rediscovery |
| Monitor scaling | Implemented, virtual-display tested | Physical mixed-DPI/Wayland checks |
| Chat list and history | Implemented, mock/GUI tested | Persistent cache; unread/pinned/archive/mute/custom groups; contact names |
| Text and new chats | Implemented, mock/GUI tested | SMS selection; pending outbox; richer composer |
| Attachments | Upload/download implemented, mock tested | Inline image/video/audio, galleries, previews, drag/drop, audio recording, stickers |
| Background interaction | Fixed, stalled-poll GUI tested | Preserve responsiveness across all new operations |
| Real-time events | Socket.IO messages/receipts/typing implemented; local handshake tested | Live Mac reconnect verification; full group/read event handling; encrypted event support |
| Local database | Opt-in SQLite history/drafts and URL isolation implemented, unit tested | Offline login, local search, cache management, future migrations |
| Contacts | Server contact names implemented | Avatars, contact editing/import, regional phone matching |
| Message actions | Pending | Copy/reply/reactions/edit/unsend/delete/forward/share/bookmark/reminders |
| Threading and reactions | Pending | Parse/present thread parts, stickers, tapbacks and custom emoji |
| Private API | Pending | Capability/version gates; subjects/effects; typing/read state; group administration |
| Group details | Pending | Rename, participants, leave/delete, group icons, media/links, local preferences |
| Scheduling | Pending | Create/edit/delete schedules; scheduled messages list; reminders |
| Search | Loaded-content search implemented | Database/server search, filters, jump to message, media/contact search |
| Desktop integration | Launcher/packages, close-to-tray, icon restore, explicit quit, connected-session desktop alerts | Notification actions/sound, main-app startup, protocol links, shortcuts; physical desktop/Wayland checks |
| Appearance | Light/dark implemented | Font/scale controls, themes/skins, accent/bubble/avatar/background customization, import/export |
| Server management | Pending | Status/capabilities, logs/stats, restart/update controls, troubleshooting |
| Backup/storage | Pending | Export/import, local/server backups, storage analysis/cleanup |
| FaceTime and Find My | Pending | Feature availability and Linux workflows from upstream |
| Accessibility/localization | Pending | Keyboard/focus navigation, screen reader, fonts/emoji, localization |

## Verification

Unit/API and virtual-display tests use mock data and must never send messages to
a real account. Mac-dependent features require explicit live verification before
being marked tested against a real server. Preserve the working Linux executable
by completing builds/tests at each development checkpoint.

# BlueBubbles Server contact-name editing

This optional source patch is required to save contact-name edits from the Rust
client. It is **not installed on your Mac by building the Linux client**.

Targets upstream BlueBubbles Server 1.9.9, commit
`f2e2286241a7c3b6617a82b37d4afaab4df3a6b9`. It adds authenticated endpoints under
the existing protected contact route group:

- `GET /api/v1/contact/capabilities`: advertises `localContactNames: true`.
- `PUT /api/v1/contact/:id`: updates an exact server contact's display name.
- `POST /api/v1/contact/local`: creates a server contact for an unknown address.

These operate on BlueBubbles Server's database, as its own contact editor does.
They do not modify the macOS Contacts/iCloud address book. The Rust client refuses
native macOS Contacts edits and ambiguous matches. Updating a name preserves all
addresses, first/last names, and the avatar. Saving never falls back to local storage.

## Apply on the Mac

Use a matching BlueBubbles Server source checkout. Copy this directory to the Mac,
then from the server checkout:

```sh
git apply --check /path/to/contact-name-editing.patch
git apply /path/to/contact-name-editing.patch
```

Build and run that checkout using the upstream server's macOS build instructions.
Its package scripts use Node 20 and include native macOS modules. This patch cannot
be added to the installed server merely by replacing the Rust Linux client. Keep
a backup of your server data and the original installed server when testing a
custom build. Updates to the upstream server may require reapplying the patch.

The Rust app checks capabilities before enabling Save. Test a disposable server
contact first. Stock servers continue to show contact names but cannot accept
remote renames. No contacts on your Mac have been changed by the local tests.

## Validation

Client transport/UI tests use mock servers. `tests/server_contact_patch.cjs`
transpiles and exercises the patched controller with a mocked contact repository.
Run it with a TypeScript installation and the patched source checkout:

```sh
NODE_PATH=/path/to/node_modules node tests/server_contact_patch.cjs /path/to/bluebubbles-server
```

A complete Electron/macOS build and live Mac validation are still required.

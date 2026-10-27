# Live sync

A device keeps one WebSocket to its mailbox. The socket is on the same Durable Object that already stores the sealed library. When a write is accepted, that object tells every connected device the new revision number. The message is `{ "rev": N }`. It does not include the sealed library.

The device then pulls that library and merges it. The merge is the same one a poll already used. Page text, the sidebar, Recently deleted, handles, folder locks, and the shared settings are inside the library, so the server still sees only ciphertext.

A desktop edit waits about half a second after the last change, then one write carries it. The other open app hears the revision in that write and pulls. With the socket down, a visible app asks every couple of seconds, and again when it comes to the front or back online. The wait grows if the socket keeps failing, up to half a minute.

The status says Live while the socket is up, Syncing while a save is going out, and Offline when this device is not connected. It does not say Synced.

## Later: more than one person in a page

Not in this version. The wire unit is already an encrypted change to the library document. A later version can send that sealed change on the same socket, instead of only the revision number. Each device merges the change into its own copy. The server still cannot read it. Presence, cursors, and more than one person typing in a page are a separate step on top of that merge. They are not part of this connection.

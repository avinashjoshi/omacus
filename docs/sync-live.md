# Live sync

A device keeps one WebSocket to its mailbox. The socket is on the same Durable Object that already stores the sealed library. When a write is accepted, that object tells every connected device the new revision number. The message is `{ "rev": N }`. It does not include the sealed library.

The device then pulls that library and merges it. The desktop window sleeps between frames, so the notice wakes it. Without that, an idle window only looked again on its fallback poll. The merge is the same one a poll already used. Page text, the sidebar, Recently deleted, handles, folder locks, and the shared settings are inside the library, so the server still sees only ciphertext.

A desktop edit waits about half a second after the last change, then one write carries it. The other open app hears the revision in that write and pulls. With the socket down, a visible app asks every couple of seconds, and again when it comes to the front or back online. The wait grows if the socket keeps failing, up to half a minute.

A page can be typed in while the pull is under way. The pull reads the page, merges, and installs the result, and a key can land between any two of those. The text typed since the read is folded into the merged page as a third copy: the text that was read is the start, the typed page is one side, the merged page is the other. The editor is then changed only where the page differs, so the cursor and undo stay. The desktop does the same when its window holds an edit that sync has not seen.

A device that joins has its own sidebar, handles, and settings, with revision numbers that were only ever counted on that device. On the first merge those numbers are set aside, so the copy already syncing wins where both have the same item, and anything only the new device has is kept. After that the two share one document and a later change wins as usual. The background pull does not run on the web while that first copy is being installed.

The status says Live while the socket is up, Syncing while a save is going out, and Offline when this device is not connected. It does not say Synced.

## Later: more than one person in a page

Not in this version. The wire unit is already an encrypted change to the library document. A later version can send that sealed change on the same socket, instead of only the revision number. Each device merges the change into its own copy. The server still cannot read it. Presence, cursors, and more than one person typing in a page are a separate step on top of that merge. They are not part of this connection.

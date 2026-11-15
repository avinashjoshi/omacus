# Reset sync on every device

Pages on a phone or computer stay. Start fresh forgets sync on that one device. Do it on each device that has joined the set.

## On this phone or computer

1. Open Sync.
2. Choose Start fresh.
3. Read the confirmation. It says this device leaves the set, the key on this device is deleted, and this device forgets how far it had synced. The pages stay.
4. Choose Start fresh again.

That removes this device from the list, deletes the device key, and clears the set id, the revision, and the cursors stored here. On a computer the key is the keychain item, the Secret Service item, or the passphrase file. On a phone it is the device file in this browser. An older app shell in the service worker cache is dropped. The current shell stays, so the app still opens offline. The pages, folders, and Recently deleted stay.

If sync cannot be reached, Try again. Start fresh here does the same forget on this device without waiting for the server. This device can still be on the list. Remove it from a device that is still linked, or run Start fresh again when this device is online.

Turn sync off does the same leave and the same local forget. Start fresh also drops every other set stored on this device, and the local backup taken before a sync merge. Turn sync off drops only the set that is open.

## On every device

1. Repeat the steps above on each phone and computer that has joined.
2. A device you skip keeps its key and stays on the list until you start fresh there, or until a device that is still linked removes it.
3. After the last device has started fresh, no device in the set still has a key. The sealed pages can still be opened with the recovery words. Do not enter those words if you want the set to stay closed.
4. To delete the sealed copy on the server, an admin removes it from the admin page after typing the account id again. That step is separate from Start fresh.

The log on the Sync tab is only counts. It does not include page names, page text, keys, recovery words, invite codes, or tokens.

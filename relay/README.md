# Private sync

This is invite-only, labeled 1.0.0. This repository does not deploy it. Hosting is in `docs/sync-ops.md`.

The server keeps sealed copies of pages. It cannot open them. The words stay on the device.

## Small test page

From the repository:

```
./relay/run
```

Then open <http://127.0.0.1:8787>. The first screen can point at another server, and `?server=` does the same.

To try it, open the address in two windows. In one, choose Start fresh, type an invite code, and write down the 12 words. Then choose Add a phone or computer. In the other, choose Join my existing pages and type that code. Joining does not ask for an invite code. Approve only when the six numbers match. Use my recovery key instead pastes the 12 words, or scans the paper, then asks for three taps. The web app calls those words recovery words. Typing them is enough, and they still open the latest pages after a phone is removed. The local server reads `ACCESS_SECRET` from the environment and does not print it.

## Invite codes

Start fresh asks for an invite code. Join and the recovery key do not. The admin page at `/admin`, and `admin.mjs`, mint a code. The server stores a hash, how many uses are left, a label, and the times. It does not store the code. The code is printed once. The secret stays out of the repository.

```
read -s ADMIN_KEY && export ADMIN_KEY
node relay/worker/admin.mjs mint --uses 3 --label "Friends"
node relay/worker/admin.mjs list
```

The command talks to `https://sync.omacus.com` and prints that it is aimed at the production sync host. Set `SYNC_URL` only to point it somewhere else. A code works once, unless `--uses` sets a higher number from 1 to 20. Existing pages keep syncing without a new code. The admin page can also pause new signups, change the account, storage, device, and rate limits, copy the sealed accounts for the night, restore one account or every account from a saved night, and remove a sealed copy. It cannot open a page. A phone merges a restored copy with the pages it already has. A longer device list that already extends the restored one stays. Every admin request needs the Access sign-in. A missing audience tag or team host refuses it. `ALLOW_NO_ACCESS` is for tests and is never set on a deployed worker. `node relay/worker/admin.mjs limits` prints the current numbers. Add a flag such as `--max-accounts` to change one. `node relay/worker/admin.mjs backups` lists the nights.

With the server already running, `node relay/check.mjs` walks that path in a browser.

## Web app bundle

`./relay/worker/build.sh` builds the web app into `relay/worker/public`. The deployed worker serves the admin at `/admin` and the sealed mailbox at `/v1`. It does not serve the app at `/`.

```
./relay/worker/build.sh && cd relay/worker && npx wrangler deploy
```

A bare `npx wrangler deploy` publishes `omacus-sync` at `sync.omacus.com`. The config has no other environment.

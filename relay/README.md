# Private sync test

This is a private test, labeled 1.0.0-beta.14. This repository does not deploy it. Hosting for the test and for production is in `docs/sync-ops.md`.

The server keeps sealed copies of pages. It cannot open them. The words stay on the device.

## Small test page

From the repository:

```
./relay/run
```

Then open <http://127.0.0.1:8787>. The first screen can point at another test server, and `?server=` does the same.

To try it, open the address in two windows. In one, choose Start fresh, type an invite code, and write down the 12 words. Then choose Add a phone or computer. In the other, choose Join my existing pages and type that code. Joining does not ask for an invite code. Approve only when the six numbers match. Use my recovery key instead pastes the 12 words, or scans the paper, then asks for three taps. The web app calls those words recovery words. Typing them is enough, and they still open the latest pages after a phone is removed. The local server reads `ACCESS_SECRET` from the environment and does not print it.

## Invite codes

Start fresh asks for an invite code. Join and the recovery key do not. The admin page at `/admin`, and `admin.mjs`, mint a code. The server stores a hash, how many uses are left, a label, and the times. It does not store the code. The code is printed once. The secret stays out of the repository.

```
read -s ADMIN_KEY && export ADMIN_KEY
node relay/worker/admin.mjs mint --uses 3 --label "Friends"
node relay/worker/admin.mjs list
```

The command talks to the test host. A code works once, unless `--uses` sets a higher number from 1 to 20. Existing pages keep syncing without a new code. The admin page can also pause new signups and remove a sealed copy. It cannot open a page.

With the server already running, `node relay/check.mjs` walks that path in a browser.

## Web app bundle

`./relay/worker/build.sh` builds the web app into `relay/worker/public`. The worker serves that app at `/`, the admin at `/admin`, and the sealed mailbox at `/v1`. Sync is in Settings, and that copy keeps its own pages.

```
./relay/worker/build.sh && cd relay/worker && npx wrangler deploy
```

Remove with `npx wrangler delete --name omacus-sync-test`, which also removes the sync-test custom domain and its stored test data.

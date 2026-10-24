# Private sync test

This is a private test, labeled 1.0.0-beta.1. This repository does not deploy it.

The server keeps sealed copies of pages. It cannot open them. The words stay on the device.

## Small test page

From the repository:

```
./relay/run
```

Then open <http://127.0.0.1:8787>. The first screen can point at another test server, and `?server=` does the same.

To try approval, open the address in two windows. Turn sync on in one. In the other, choose Add this device. Approve only when the six-digit code matches. Use your recovery key pastes the words from the sheet, or scans it, then asks for three taps.

With the server already running, `node relay/check.mjs` walks that path in a browser.

## Web app bundle

`./relay/worker/build.sh` builds the web app into `relay/worker/public`. The worker serves that app at `/` and the sealed mailbox at `/v1`. Sync is in Settings, and that copy keeps its own pages.

```
./relay/worker/build.sh && cd relay/worker && npx wrangler deploy
```

Remove with `npx wrangler delete --name omacus-sync-test`, which also removes the sync-test custom domain and its stored test data.

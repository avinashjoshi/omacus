# Private sync test

This is a private test, labeled 1.0.0-beta.1. It is not the app, and it is not deployed.

From the repository:

```
./relay/run
```

Then open <http://127.0.0.1:8787>. The first screen can point at another test server, and `?server=` does the same.

The server keeps sealed copies of pages. It cannot open them. The words stay on the device.

To try approval, open the address in two windows. Turn sync on in one. In the other, choose Add this device. Approve only when the six-digit code matches. Use your recovery key pastes the words from the sheet, or scans it, then asks for three taps.

With the server already running, `node relay/check.mjs` walks that path in a browser.

# Production

This is the short list for the day sync goes live. The longer hosting notes are in `docs/sync-ops.md`. This repository does not deploy the worker.

Production is the worker `omacus-sync` at `https://sync.omacus.com`. The notepad at `https://app.omacus.com` needs no sign-up. Sync is invite-only. The worker's health check answers `GET /v1/health` with label `1.0.0` and `"push": true`.

## Before the first production deploy

Follow "Before the first production deploy" in `docs/sync-ops.md`. In short:

- Set `ACCESS_SECRET`, `ADMIN_KEY`, and `ACCESS_AUD` on `omacus-sync`. Do not set `ALLOW_NO_ACCESS`. `ACCESS_TEAM_DOMAIN` is already `omacus.cloudflareaccess.com` in `relay/worker/wrangler.jsonc`. It is not a secret.
- Turn on point-in-time recovery for the production Durable Object class `Mailboxes` (binding `MAILBOXES`). That class is in `relay/worker/wrangler.jsonc`. It is not a KV namespace, and the file is not `wrangler.toml`. The R2 binding is `BACKUPS`. The production bucket is `omacus-sync-backups` (prefix `nights/`, delete after 30 days). The nightly copy runs at 07:15 UTC (`15 7 * * *`).
- From `relay/worker`, `./build.sh` then `npx wrangler deploy`, and only when you mean to. That publishes the worker `omacus-sync`. The file has no other environment. A release runs that same deploy.
- After it is up, check `GET /v1/health`, that `GET /` redirects to the app, that admin is refused when signed out, and that a phone can create pages, link a second one, and save.

## Pages missing after an older sync

Look in this order. Stop when the page is back. Do not clear website data, unregister the service worker, turn sync off, remove a phone or computer, or restore a backup over a phone that is still syncing.

1. Recently deleted, on the phone and on the computer. Restore puts that page back.
2. The page file on the computer: `pages/<id>.txt` in the Omacus folder, and `trash/<id>.txt` after a delete. The web app keeps a zip of the library from just before a remote copy was applied, in this browser, under the key `sync-remote-backup`.
3. An older sealed copy on the relay. The admin page at `/admin` on the sync host asks first, then writes one account or every account from a chosen day. It does not show page text.
4. A nightly backup in the bucket `omacus-sync-backups`, prefix `nights/`. Those copies are kept for 30 days. The job runs at 07:15 UTC.

## Go live

Do these in this order.

1. Merge to `main`.
2. Deploy the production worker, as above.
3. Smoke test that worker: `GET /v1/health` returns `{"label":"1.0.0"}`, `GET /` redirects to the app, admin is refused when signed out, and a phone can create pages, link a second one, and save.
4. Publish the sync build to `https://app.omacus.com`. The Release workflow does that in the same run as the desktop app, after the tag, from that same commit. The job uses the `app` environment. A push to `main` does not publish the app. To retry that deploy by hand, start `.github/workflows/app.yml` (`workflow_dispatch`) on the merged commit. It runs `.github/scripts/build-web.sh`.
5. Smoke test the app: the same phone opens `https://app.omacus.com`, turns sync on, and reaches the worker from step 3.

`site/_headers` is the content security policy and HSTS text for omacus.com. GitHub Pages does not send that file, so those lines are not the live headers. Each page also carries the policy in a meta tag. The live headers are a Cloudflare response rule, and HSTS is the zone setting. The exact clicks and the API body are in `docs/ops/cloudflare-headers.json` and `docs/ops/release-protection.md`. `/app` on this origin keeps the bridge rules, including the engine. app.omacus.com sends its policy from `site/app/_headers`, including HSTS.

The health workflow below expects the label `1.0.0`. Until this worker is deployed, the first scheduled run opens an issue. Close that issue after the deploy.

## Health

`.github/workflows/sync-health.yml` pings `https://sync.omacus.com/v1/health` at minute 17 of every hour, and when someone starts it by hand. It wants HTTP 200, `"label":"1.0.0"`, and `"push": true`. On a failure it opens one GitHub issue labeled `sync-health`. A later failure leaves that issue open. The schedule runs only after the workflow is on the default branch.

## Updates

Each desktop release publishes `omacus-update.json` and `omacus-update.json.minisig`. The signature is minisign's legacy algorithm, `Ed`: it signs the file itself. The app checks that signature, then checks the file's sha256 from the manifest, before it installs. An unsigned tag is not a version. The one-line installer still checks `SHA256SUMS.txt`.

The signing secret is `MINISIGN_SECRET_KEY` on the GitHub environment `release`, not a plain repository secret. It is three lines: `omacus-update-key-v1`, the 16 hex key id, and the 64 hex seed. It is not in this repository. The public key is `keys/omacus-update.pub`, and that key is compiled into the app. The release stops if the secret is missing. Replacing the public key takes an app release, because copies already installed keep the key they were built with.

Copies from 0.2.x check a SHA-256 published on the same GitHub release. They do not check the minisign signature. A person who can replace that release can ship those copies a new build. 1.0 and later check `omacus-update.json.minisig` before they trust a hash. Those older copies get that check when they install 1.0 or later. The one-line installer checks the signature when `minisign` is installed, and otherwise checks `SHA256SUMS.txt` and says so.

If you suspect the admin key leaked, rotate `ADMIN_KEY` on the worker. The admin key alone cannot read pages. It also needs a Cloudflare Access login, and admin still sees counts and limits only.

Mac builds are still ad-hoc signed (`codesign --force --deep --sign -` in the release workflow) and are not notarized. Notarizing needs an Apple Developer ID Application certificate, notarytool credentials (`APPLE_ID`, `APPLE_TEAM_ID`, and an app-specific password or an App Store Connect API key), then replacing that ad-hoc sign, `xcrun notarytool submit`, and `xcrun stapler staple`.

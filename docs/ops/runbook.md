# Production

This is the short list for the day sync goes live. The longer hosting notes are in `docs/sync-ops.md`. This repository does not deploy the worker.

Production is the worker `omacus-sync` at `https://sync.omacus.com`. The notepad at `https://app.omacus.com` needs no sign-up. Sync is invite-only. The worker's health check answers `GET /v1/health` with `{"label":"1.0.0"}`.

## Before the first production deploy

Follow "Before the first production deploy" in `docs/sync-ops.md`. In short:

- Set `ACCESS_SECRET`, `ADMIN_KEY`, and `ACCESS_AUD` on `omacus-sync`. Do not set `ALLOW_NO_ACCESS`. `ACCESS_TEAM_DOMAIN` is already `omacus.cloudflareaccess.com` in `relay/worker/wrangler.jsonc`. It is not a secret.
- Turn on point-in-time recovery for the production Durable Object class `Mailboxes` (binding `MAILBOXES`). That class is in `relay/worker/wrangler.jsonc`. It is not a KV namespace, and the file is not `wrangler.toml`. The R2 binding is `BACKUPS`. The production bucket is `omacus-sync-backups` (prefix `nights/`, delete after 30 days). The nightly copy runs at 07:15 UTC (`15 7 * * *`).
- From `relay/worker`, `./build.sh` then `npx wrangler deploy --env production`, and only when you mean to. That env is the worker `omacus-sync`. The same file without `--env` deploys `omacus-sync-test`.
- After it is up, check `GET /v1/health`, that `GET /` redirects to the app, that admin is refused when signed out, and that a phone can create pages, link a second one, and save.

## Go live

Do these in this order.

1. Merge to `main`.
2. Deploy the production worker, as above.
3. Smoke test that worker: `GET /v1/health` returns `{"label":"1.0.0"}`, `GET /` redirects to the app, admin is refused when signed out, and a phone can create pages, link a second one, and save.
4. Publish the sync build to `https://app.omacus.com`. The Release workflow does that after the tag, by calling `.github/workflows/app.yml`. A push to `main` does not publish the app. To publish this commit without a desktop release, start the App workflow by hand (`workflow_dispatch`) on the merged commit. `app.yml` runs `.github/scripts/build-web.sh`.
5. Smoke test the app: the same phone opens `https://app.omacus.com`, turns sync on, and reaches the worker from step 3.

`site/_headers` sends HSTS and the content security policy for omacus.com. Each page also carries that policy, so it still applies when the host does not read `_headers`. `/app` on this origin keeps the bridge rules, including the engine.

The health workflow below expects the label `1.0.0`. Until this worker is deployed, the first scheduled run opens an issue. Close that issue after the deploy.

## Health

`.github/workflows/sync-health.yml` pings `https://sync.omacus.com/v1/health` at minute 17 of every hour, and when someone starts it by hand. It wants HTTP 200 and `"label":"1.0.0"`. On a failure it opens one GitHub issue labeled `sync-health`. A later failure leaves that issue open. The schedule runs only after the workflow is on the default branch.

## Updates

Each desktop release publishes `omacus-update.json` and `omacus-update.json.minisig`. The signature is minisign's legacy algorithm, `Ed`: it signs the file itself. The app checks that signature, then checks the file's sha256 from the manifest, before it installs. An unsigned tag is not a version. The one-line installer still checks `SHA256SUMS.txt`.

The signing secret is the GitHub Actions secret `MINISIGN_SECRET_KEY`. It is three lines: `omacus-update-key-v1`, the 16 hex key id, and the 64 hex seed. It is not in this repository. The public key is `keys/omacus-update.pub`, and that key is compiled into the app. The release stops if the secret is missing. Replacing the public key takes an app release, because copies already installed keep the key they were built with.

Mac builds are still ad-hoc signed (`codesign --force --deep --sign -` in the release workflow) and are not notarized. Notarizing needs an Apple Developer ID Application certificate, notarytool credentials (`APPLE_ID`, `APPLE_TEAM_ID`, and an app-specific password or an App Store Connect API key), then replacing that ad-hoc sign, `xcrun notarytool submit`, and `xcrun stapler staple`.

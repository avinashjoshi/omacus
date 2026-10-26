# Production

This is the short list for the day sync goes live. The longer hosting notes are in `docs/sync-ops.md`. This repository does not deploy the worker.

Production is the worker `omacus-sync` at `https://sync.omacus.com`. The notepad at `https://app.omacus.com` needs no sign-up. Sync is invite-only. The worker's health check answers `GET /v1/health` with `{"label":"1.0.0"}`.

## Before the first production deploy

Follow "Before the first production deploy" in `docs/sync-ops.md`. In short:

- Set `ACCESS_SECRET`, `ADMIN_KEY`, and `ACCESS_AUD` on `omacus-sync`. Do not set `ALLOW_NO_ACCESS`.
- Turn on point-in-time recovery for the production `Mailboxes` namespace, and the R2 lifecycle rule on `omacus-sync-backups` (prefix `nights/`, delete after 30 days).
- Deploy with `npx wrangler deploy --env production` from `relay/worker` only when you mean to.
- After it is up, check `GET /v1/health`, that `GET /` redirects to the app, that admin is refused when signed out, and that a phone can create pages, link a second one, and save.

The health workflow below expects the label `1.0.0`. Until this worker is deployed, the first scheduled run opens an issue. Close that issue after the deploy.

## Health

`.github/workflows/sync-health.yml` pings `https://sync.omacus.com/v1/health` at minute 17 of every hour, and when someone starts it by hand. It wants HTTP 200 and `"label":"1.0.0"`. On a failure it opens one GitHub issue labeled `sync-health`. A later failure leaves that issue open. The schedule runs only after the workflow is on the default branch.

## Updates

Each desktop release publishes `omacus-update.json` and `omacus-update.json.minisig`. The signature is minisign's legacy algorithm, `Ed`: it signs the file itself. The app checks that signature, then checks the file's sha256 from the manifest, before it installs. An unsigned tag is not a version. The one-line installer still checks `SHA256SUMS.txt`.

The signing secret is the GitHub Actions secret `MINISIGN_SECRET_KEY`. It is three lines: `omacus-update-key-v1`, the 16 hex key id, and the 64 hex seed. It is not in this repository. The public key is `keys/omacus-update.pub`, and that key is compiled into the app. The release stops if the secret is missing. Replacing the public key takes an app release, because copies already installed keep the key they were built with.

Mac builds are still ad-hoc signed (`codesign --force --deep --sign -` in the release workflow) and are not notarized. Notarizing needs an Apple Developer ID Application certificate, notarytool credentials (`APPLE_ID`, `APPLE_TEAM_ID`, and an app-specific password or an App Store Connect API key), then replacing that ad-hoc sign, `xcrun notarytool submit`, and `xcrun stapler staple`.

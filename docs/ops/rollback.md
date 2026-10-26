# Rollback

Details of deploys, nights, and restores are in `docs/sync-ops.md`. Roll back the piece that broke, and leave the other piece where it is.

## Sync worker

Deploy the previous worker script for `omacus-sync`:

```
npx wrangler deploy --env production
```

Run that from the commit you are returning to, in `relay/worker`. A secret set on the test worker is not the production secret. Do not set `ALLOW_NO_ACCESS`. Pages already on a phone stay on that phone.

`GET /v1/health` has to keep answering with label `1.0.0`. A worker from before that label makes the health check open a `sync-health` issue. Close the issue once the label is `1.0.0` again.

## App

Deploy the previous app assets for app.omacus.com. The release workflow publishes the app from the tagged commit. A phone keeps the pages it already has.

## Desktop

The app installs a newer signed manifest only. It will not move itself back to an older version. To undo a bad desktop release, publish a newer signed release that fixes it. Someone can also install an older file by hand from the releases page. That older file was not signed with this manifest.

## A night's copy

Point-in-time recovery on the `Mailboxes` namespace is the first copy. A night in R2 is the sealed record, not an open page. Restoring one account goes through the same save a phone uses, and it leaves a longer device list in place. See "Nightly copies" in `docs/sync-ops.md`.

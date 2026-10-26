# Sync hosting

This repository does not deploy the worker. Sync answers with `Access-Control-Allow-Origin: *`. It never sets a cookie, and it never takes a cookie as a login.

The app at app.omacus.com stays the app. The published build sends sealed pages to `https://sync.omacus.com`. The source file leaves that address empty, and the build writes it.

## Production

The worker is named `omacus-sync`. It serves the sync API at `sync.omacus.com` and the admin at `sync.omacus.com/admin`. It does not serve the app. A GET or HEAD of exactly `https://sync.omacus.com/` redirects to the app. `APP_URL` is that address, and it defaults to `https://app.omacus.com/`. The config already sets it. Any other unknown path is a plain not-found.

`relay/worker/wrangler.jsonc` is that worker. A bare `npx wrangler deploy` publishes it. The file has no other environment, so that command cannot create a second worker. A local run can still serve the app when `SERVE_APP` is `1`. The deployed config sets `SERVE_APP` to `0`.

Deploy it only when you mean to, from `relay/worker`:

```
./build.sh
npx wrangler deploy
```

`./build.sh` points the built app at `https://sync.omacus.com`. It does not change the source file.

Set the secrets on this worker. The commands prompt for the value. The values are not written in the repository.

```
npx wrangler secret put ACCESS_SECRET
npx wrangler secret put ADMIN_KEY
npx wrangler secret put ACCESS_AUD
```

`ACCESS_SECRET` checks invite codes. `ADMIN_KEY` opens the admin commands. Cloudflare Access is in front of `sync.omacus.com/admin` and `sync.omacus.com/v1/admin` on the same application. The page calls `/v1/admin`. That path has to be in the application, or Cloudflare does not attach `Cf-Access-Jwt-Assertion` and every call is refused.

A missing `ACCESS_AUD` or `ACCESS_TEAM_DOMAIN` refuses admin with "Sign in again." A missing or expired Access sign-in is the same refusal. A wrong admin key is still "That key didn't work." `/v1/admin` requires both the sign-in and the admin key. `ALLOW_NO_ACCESS` is for tests only. Do not set it on the deployed worker.

Production starts at 10,000 accounts, 50 MB (52,428,800 bytes) per account, and 10 devices. Those are the numbers before anyone saves a limit. The admin page can change them. Reads start at 180 a minute per account and 500 per address. A local worker that serves the app starts lower: 3 accounts, 65,536 bytes, 4 devices, 100 reads a minute per account, and 140 per address. Do not copy that storage cap onto production.

Create one Access application that includes both `sync.omacus.com/admin` and `sync.omacus.com/v1/admin`. The page loads from `/admin` and its calls go to `/v1/admin`. A path left out of the application does not get `Cf-Access-Jwt-Assertion`, and the page then says "Sign in again." The worker sets `ACCESS_TEAM_DOMAIN` to `omacus.cloudflareaccess.com`. Set that application's audience tag as `ACCESS_AUD`. If either value is missing, admin is refused. Do not set `ALLOW_NO_ACCESS` on the worker.

## Before the first production deploy

- `ACCESS_AUD` and `ACCESS_TEAM_DOMAIN` are both set. A missing one refuses admin. `ALLOW_NO_ACCESS` is not set.
- The R2 lifecycle rule is on `omacus-sync-backups`. Prefix `nights/`. Delete after 30 days.
- The published build includes the sync engine. `.github/scripts/build-web.sh` builds `omacus-sync-web` and stops if `omacus_sync_web.js` or `omacus_sync_web_bg.wasm` is missing. That build writes `https://sync.omacus.com` into the app's sync address, and `site/app/_headers` allows that host in `connect-src`. The published app shows the Sync tab. Deploy app.omacus.com from that build.
- Point-in-time recovery is on for the production `Mailboxes` class. In the Cloudflare dashboard, open that class and turn it on before you rely on a night's copy.
- After the deploy, check `GET /v1/health`, that `GET /` redirects to the app, that admin returns 403 when signed out, and that a phone can create a set of pages, link a second one, and save.
- To roll back, deploy the previous worker script and the previous app assets. A phone keeps the pages it already has.

## Nightly copies

Point-in-time recovery on the Durable Object is the first copy. In the Cloudflare dashboard, open the `Mailboxes` class for the worker and turn on point-in-time recovery before you rely on a night's copy.

The bucket already exists and is bound as `BACKUPS`. The worker uses `omacus-sync-backups`. A deploy picks up that binding. The worker copies every sealed account at 07:15 UTC. The copy is the sealed record, the sealed page objects, and the signed device list inside that record. It is not decrypted. The worker does not delete old nights.

Keep 30 days with an R2 lifecycle rule. Prefix `nights/`. Delete objects after 30 days.

```json
{
  "rules": [
    {
      "id": "nightly-copies",
      "enabled": true,
      "conditions": { "prefix": "nights/" },
      "deleteObjectsTransition": { "condition": { "maxAge": 2592000 } }
    }
  ]
}
```

Add that rule on `omacus-sync-backups` (R2, the bucket, Settings, Object lifecycle rules). `2592000` is 30 days in seconds. Before the first production deploy, confirm that rule is on `omacus-sync-backups`: prefix `nights/`, delete after 30 days.

The admin page and `admin.mjs restore` write a night back through the same revision a phone uses when it saves. A phone then merges that copy with the pages it already has. One account, or every account from that day. They do not open a page. If the device list already on the server is a longer extension of the list in that night, the restore leaves the current list and the current pages. A phone that was removed does not come back from an older night, and that night is not written over the longer list.

## Cloudflare Access

Access protects `sync.omacus.com/admin` and `sync.omacus.com/v1/admin` on one application. The worker checks `Cf-Access-Jwt-Assertion` on every `/admin` and `/v1/admin` request. A missing `ACCESS_AUD` or `ACCESS_TEAM_DOMAIN` is a refusal, the same as a missing sign-in. `ACCESS_TEAM_DOMAIN` is the team host (`omacus.cloudflareaccess.com`). The worker checks the signature against `https://omacus.cloudflareaccess.com/cdn-cgi/access/certs`, the issuer `https://omacus.cloudflareaccess.com`, and the `ACCESS_AUD` tag. A missing or expired sign-in is HTTP 403, "Sign in again." A wrong admin key is HTTP 401, "That key didn't work." `/v1/admin` requires both. `ALLOW_NO_ACCESS=1` skips that check in tests and is never set on the deployed worker.

Set the audience tag:

```
npx wrangler secret put ACCESS_AUD
```

For the command line, create a service token in Access and export `CF_ACCESS_CLIENT_ID` and `CF_ACCESS_CLIENT_SECRET`. The admin command sends those headers. It does not print them. Access adds the assertion before the request reaches the worker.

## Invite codes

The admin command talks to `https://sync.omacus.com`. It prints that the command is aimed at the production sync host. Set `SYNC_URL` only when you mean to point it somewhere else.

```
read -s ADMIN_KEY && export ADMIN_KEY
node relay/worker/admin.mjs mint --uses 3 --label "Friends"
node relay/worker/admin.mjs mint --uses 1 --expires 2026-12-31
node relay/worker/admin.mjs list
node relay/worker/admin.mjs revoke <hash>
node relay/worker/admin.mjs stats
node relay/worker/admin.mjs pause
node relay/worker/admin.mjs resume
node relay/worker/admin.mjs limits
node relay/worker/admin.mjs limits --max-accounts 10000
node relay/worker/admin.mjs backups
node relay/worker/admin.mjs backup
node relay/worker/admin.mjs restore <account> --date 2026-10-25 --confirm <account>
node relay/worker/admin.mjs restore --all --date 2026-10-25 --confirm 2026-10-25
node relay/worker/admin.mjs delete <account> --confirm <account>
```

Mint prints the code once. The server stores a hash, a label, the uses left, and the times. It does not store the code. An optional expiry is the end of that UTC day.

The same actions are on the admin page. List shows the label, uses left, created time, last-used time, and status. Revoke retires a code. Stats show the account count, how many phones and computers are linked, the total size, how many accounts were used in the last day and the last week, how many accounts were created in the last week, and the last nightly copy: when it ran, whether it saved, and its size. Each account shows when it was created, when it was last used, when its sealed copy last changed, its size, its linked phones and computers, and how many times it was used in the last day. A time reads as how long ago it was, and hovering shows the exact time. The device count is the phones and computers on that account's list. Stats do not show page text or names. Pause and resume apply to new signups. Phones that are already in keep syncing. Limits set how many accounts the server will take, how much storage one account can use, how many devices it can list, and the rate limits. A signup past the account limit is refused, and the invite code is not used. A nightly copy keeps each sealed account. Restoring writes that copy back and does not open a page. A longer device list that already extends the restored one stays, and so do its pages. Removing a sealed copy asks for the account id to be typed again.

`node relay/worker/mint.mjs` does the same as `admin.mjs mint`.

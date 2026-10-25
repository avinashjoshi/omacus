# Sync hosting

The test worker and the production worker are separate. This repository does not deploy either one. Sync hosts answer with `Access-Control-Allow-Origin: *`. They never set a cookie, and they never take a cookie as a login.

The app at app.omacus.com stays the app. A production build sends sealed pages to `https://sync.omacus.com`. A test build sends them to `https://sync-test.omacus.com`. The source file leaves that address empty, and the build writes it.

## Test

The test worker is named `omacus-sync-test`. It serves the sync API and the test app at `sync-test.omacus.com`, and the admin at `sync-test.omacus.com/admin`.

From `relay/worker`, build the test app, then deploy with no `--env`. That is the test worker.

```
./build.sh
npx wrangler deploy
```

`build.sh` points the built app at the test host. It does not change the source file.

Set the secrets on this worker. The commands prompt for the value. The values are not written in the repository.

```
npx wrangler secret put ACCESS_SECRET
npx wrangler secret put ADMIN_KEY
```

`ACCESS_SECRET` checks invite codes. `ADMIN_KEY` opens the admin commands. Cloudflare Access is in front of `sync-test.omacus.com/admin` and `sync-test.omacus.com/v1/admin` on the same application. The page calls `/v1/admin`. That path has to be in the application, or Cloudflare does not attach `Cf-Access-Jwt-Assertion` and every call is refused. Set the audience tag so the worker checks that sign-in:

```
npx wrangler secret put ACCESS_AUD
```

A missing `ACCESS_AUD` or `ACCESS_TEAM_DOMAIN` refuses admin with "Sign in again." A missing or expired Access sign-in is the same refusal. A wrong admin key is still "That key didn't work." `/v1/admin` requires both the sign-in and the admin key. `ALLOW_NO_ACCESS` is for tests only. Do not set it on a deployed worker.

## Production

The production worker is named `omacus-sync`. It has its own stored data and its own secrets. It serves the sync API at `sync.omacus.com` and the admin at `sync.omacus.com/admin`. It does not serve the app. A GET or HEAD of exactly `https://sync.omacus.com/` redirects to the app. `APP_URL` is that address, and it defaults to `https://app.omacus.com/`. The production config already sets it. Any other unknown path is a plain not-found. The test worker (`SERVE_APP` is `1`) still serves the app at `/`.

Deploy it only when you mean to:

```
./build.sh
npx wrangler deploy --env production
```

Set the secrets on that worker. A secret set on the test worker is not copied over.

```
npx wrangler secret put ACCESS_SECRET --env production
npx wrangler secret put ADMIN_KEY --env production
npx wrangler secret put ACCESS_AUD --env production
```

Production starts at 10,000 accounts, 50 MB (52,428,800 bytes) per account, and 10 devices. The test worker starts at 3 accounts, 65,536 bytes, and 4 devices. Those are the numbers before anyone saves a limit. The admin page can change them. Do not copy the test storage cap onto production.

Create one Access application that includes both `sync.omacus.com/admin` and `sync.omacus.com/v1/admin`. The page loads from `/admin` and its calls go to `/v1/admin`. A path left out of the application does not get `Cf-Access-Jwt-Assertion`, and the page then says "Sign in again." Both workers set `ACCESS_TEAM_DOMAIN` to `omacus.cloudflareaccess.com`. Set that application's own audience tag as `ACCESS_AUD` on the production worker. If either value is missing, admin is refused. Do not set `ALLOW_NO_ACCESS` on the production worker.

## Before the first production deploy

- `ACCESS_AUD` and `ACCESS_TEAM_DOMAIN` are both set. A missing one refuses admin. `ALLOW_NO_ACCESS` is not set.
- The R2 lifecycle rule is on `omacus-sync-backups`. Prefix `nights/`. Delete after 30 days. The same rule is on `omacus-sync-test-backups`.

## Nightly copies

Point-in-time recovery on the Durable Object is the first copy. In the Cloudflare dashboard, open the `Mailboxes` namespace for the worker and turn on point-in-time recovery. Do that for the test worker and for the production worker before you rely on a night's copy.

The buckets already exist and are bound as `BACKUPS`. The test worker uses `omacus-sync-test-backups`. The production worker uses `omacus-sync-backups`. A deploy picks up that binding. The worker copies every sealed account at 07:15 UTC. The copy is the sealed record, the sealed page objects, and the signed device list inside that record. It is not decrypted. The worker does not delete old nights.

Keep 30 days with an R2 lifecycle rule on each bucket. Prefix `nights/`. Delete objects after 30 days.

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

Add that rule on `omacus-sync-test-backups` and on `omacus-sync-backups` (R2, the bucket, Settings, Object lifecycle rules). `2592000` is 30 days in seconds. Before the first production deploy, confirm that rule is on `omacus-sync-backups`: prefix `nights/`, delete after 30 days.

The admin page and `admin.mjs restore` write a night back through the same revision a phone uses when it saves. A phone then merges that copy with the pages it already has. One account, or every account from that day. They do not open a page. If the device list already on the server is a longer extension of the list in that night, the restore leaves the current list and the current pages. A phone that was removed does not come back from an older night, and that night is not written over the longer list.

## Cloudflare Access

Access protects `sync-test.omacus.com/admin` and `sync-test.omacus.com/v1/admin` on one application. The worker checks `Cf-Access-Jwt-Assertion` on every `/admin` and `/v1/admin` request. A missing `ACCESS_AUD` or `ACCESS_TEAM_DOMAIN` is a refusal, the same as a missing sign-in. `ACCESS_TEAM_DOMAIN` is the team host (`omacus.cloudflareaccess.com` on both workers). The worker checks the signature against `https://omacus.cloudflareaccess.com/cdn-cgi/access/certs`, the issuer `https://omacus.cloudflareaccess.com`, and the `ACCESS_AUD` tag. A missing or expired sign-in is HTTP 403, "Sign in again." A wrong admin key is HTTP 401, "That key didn't work." `/v1/admin` requires both. `ALLOW_NO_ACCESS=1` skips that check in tests and is never set on a deployed worker.

Production needs its own Access application covering both `sync.omacus.com/admin` and `sync.omacus.com/v1/admin`, then:

```
npx wrangler secret put ACCESS_AUD --env production
```

For the command line, create a service token in Access and export `CF_ACCESS_CLIENT_ID` and `CF_ACCESS_CLIENT_SECRET`. The admin command sends those headers. It does not print them. Access adds the assertion before the request reaches the worker.

## Desktop test

omacus Test is a separate app for trying sync at `https://sync-test.omacus.com`. It is not a release of Omacus, and it does not replace the installed app. Desktop sync in Omacus itself stays a later version.

The app is named omacus Test. Its id is `dev.omacus.OmacusTest`. On Mac its sync name is omacus Test on Mac. On Linux it is omacus Test on Linux. A browser on the same computer keeps its own name, such as Chrome on Linux. It does not register the `omacus://` link, and it does not write `~/.local/share/applications/omacus.desktop`. Pages stay in this app's own WebKit store. On Linux that is `~/.local/share/omacus-test` and `~/.cache/omacus-test`. They are not the pages in Omacus (`~/.local/share/omacus`, `~/.config/omacus`).

On a Mac, the build is unsigned. It runs on Apple silicon and Intel. Download `omacus-test-mac.zip` from the `omacus-test-desktop` prerelease, unzip it, then right-click omacus Test, choose Open, and choose Open again. A double-click is blocked. The window opens the sync test.

On Linux (x86_64, including Arch with Wayland), the build is `omacus-test-linux-x86_64.tar.gz` on that same prerelease. It uses the system WebKit. `webkit2gtk-4.1` pulls in gtk3. Install that, then download and run:

```
sudo pacman -S --needed webkit2gtk-4.1 && curl -fsSL -o /tmp/omacus-test-linux-x86_64.tar.gz https://github.com/avinashjoshi/omacus/releases/download/omacus-test-desktop/omacus-test-linux-x86_64.tar.gz && mkdir -p "$HOME/omacus-test-linux" && tar -xzf /tmp/omacus-test-linux-x86_64.tar.gz -C "$HOME/omacus-test-linux" && "$HOME/omacus-test-linux/omacus-test"
```

The window uses Wayland on its own. If it stays blank on Hyprland, quit and run `WEBKIT_DISABLE_DMABUF_RENDERER=1 "$HOME/omacus-test-linux/omacus-test"`.

The tarball includes `dev.omacus.OmacusTest.desktop`. That file has no `omacus://` handler. Copying it to `~/.local/share/applications/dev.omacus.OmacusTest.desktop` does not replace the Omacus entry. Set `Exec` to the full path of `omacus-test` if you want a launcher.

Remove the test app, its pages, and that launcher:

```
rm -rf "$HOME/omacus-test-linux" "$HOME/.local/share/omacus-test" "$HOME/.cache/omacus-test"
rm -f "$HOME/.local/share/applications/dev.omacus.OmacusTest.desktop"
```

That leaves the installed Omacus app, its settings, and its `omacus://` handler in place.

## Invite codes

The admin command defaults to the test host. Set `SYNC_URL` to `https://sync.omacus.com` only when you mean to change production.

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

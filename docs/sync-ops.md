# Sync hosting

The test worker and the production worker are separate. This repository does not deploy either one.

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

`ACCESS_SECRET` checks invite codes. `ADMIN_KEY` opens the admin page and the admin commands. Leave Cloudflare Access unset until it is in front of the admin paths. While it is unset, the admin key is enough.

## Production

The production worker is named `omacus-sync`. It has its own stored data and its own secrets. It serves the sync API at `sync.omacus.com` and the admin at `sync.omacus.com/admin`. It does not serve the app.

Deploy it only when you mean to:

```
./build.sh
npx wrangler deploy --env production
```

Set the secrets on that worker. A secret set on the test worker is not copied over.

```
npx wrangler secret put ACCESS_SECRET --env production
npx wrangler secret put ADMIN_KEY --env production
```

## Cloudflare Access

Put Access in front of `/admin` and `/v1/admin` on the host you are protecting. Create an Access application for those paths and copy its audience tag.

When both of these are set, admin calls also require a valid Access token. The worker checks the `Cf-Access-Jwt-Assertion` header. Leave them unset until Access is in place, or every admin call is refused.

```
npx wrangler secret put CF_ACCESS_TEAM
npx wrangler secret put CF_ACCESS_AUD
```

`CF_ACCESS_TEAM` is the team name in `https://<team>.cloudflareaccess.com`. `CF_ACCESS_AUD` is the application's audience tag. Add `--env production` for the production worker.

For the command line, create a service token in Access and export `CF_ACCESS_CLIENT_ID` and `CF_ACCESS_CLIENT_SECRET`. The admin command sends those headers. It does not print them.

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
node relay/worker/admin.mjs delete <account> --confirm <account>
```

Mint prints the code once. The server stores a hash, a label, the uses left, and the times. It does not store the code. An optional expiry is the end of that UTC day.

The same actions are on the admin page. List shows the label, uses left, created time, last-used time, and status. Revoke retires a code. Stats show the account count, the device count, the size of each sealed copy, and recent error counts. They do not show page text or names. Pause and resume apply to new signups. Phones that are already in keep syncing. Limits set how many accounts the server will take, how much storage one account can use, how many devices it can list, and the rate limits. A signup past the account limit is refused, and the invite code is not used. Removing a sealed copy asks for the account id to be typed again.

`node relay/worker/mint.mjs` does the same as `admin.mjs mint`.

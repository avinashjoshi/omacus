# Release and hosting protection

The environments `app` and `release` exist. Neither has a required reviewer. Each one allows deployments only from the branch `main` and from tags matching `v*`.

Hardware 2FA is not part of this change.

The deployment policy on both environments is:

```json
{
  "wait_timer": 0,
  "prevent_self_review": false,
  "reviewers": [],
  "deployment_branch_policy": {
    "protected_branches": false,
    "custom_branch_policies": true
  }
}
```

The custom policies are a branch named `main` and a tag pattern `v*`.

## Web app deploy

The App workflow's deploy job uses the environment `app`.

`CLOUDFLARE_API_TOKEN` is a secret on that environment. `CLOUDFLARE_ACCOUNT_ID` stays a repository secret. The deploy job can still read it. An environment secret of the same name would override the repository secret, so the account id is not copied onto `app`.

Deleting a repository copy of `CLOUDFLARE_API_TOKEN` is optional. The environment secret is the one this job uses.

## Desktop release signing

The publish job uses the environment `release`. `MINISIGN_SECRET_KEY` is a secret on that environment. The job still stops if that secret is missing.

The version tag is pushed with the job's `GITHUB_TOKEN`. The checkout keeps its credentials, and the job has `contents: write`. A missing `RELEASE_TOKEN` does not stop the tag. The tag step does not read `RELEASE_TOKEN`, so a stale token cannot take the push.

An optional repository `RELEASE_TOKEN` can still open the prepare pull request as that account. The prepare job falls back to `GITHUB_TOKEN` when it is absent. Do not put `RELEASE_TOKEN` on the `release` environment.

## Version tags

The ruleset `Protect v tags` is active. It covers `refs/tags/v*`. It blocks updating a tag, deleting a tag, and a force-push (non-fast-forward). Creating a tag is allowed, so the publish job can create `v*` with `GITHUB_TOKEN`. Repository admins bypass the ruleset. The publish job still refuses to move a tag that already points at a different commit.

```json
{
  "name": "Protect v tags",
  "target": "tag",
  "enforcement": "active",
  "conditions": {
    "ref_name": {
      "include": ["refs/tags/v*"],
      "exclude": []
    }
  },
  "rules": [
    { "type": "update" },
    { "type": "deletion" },
    { "type": "non_fast_forward" }
  ]
}
```

The bypass actor is the repository admin role.

## HSTS

Do this in the Cloudflare dashboard for the zone `232a20ef78c41a58eb56f2d7ae03d15d` (omacus.com). One zone covers `omacus.com` and `app.omacus.com` when include-subdomains is on. If `app.omacus.com` is ever a separate zone, repeat these clicks there.

1. SSL/TLS → Edge Certificates → Always Use HTTPS: On.
2. SSL/TLS → Edge Certificates → HTTP Strict Transport Security (HSTS) → Enable HSTS.
3. Max age: 12 months.
4. Include subdomains: On.
5. Preload: Off. Preload is hard to undo. Turn it on later, on purpose.
6. No-Sniff: On.

The response-header rule that sends the content security policy is `docs/ops/cloudflare-headers.json`. Apply it with:

```
gh api --method PUT zones/232a20ef78c41a58eb56f2d7ae03d15d/rulesets/phases/http_response_headers_transform/entrypoint \
  --input docs/ops/cloudflare-headers.json
```

If that phase already has rules, merge these rules into the existing entrypoint instead of replacing them. The request body Cloudflare expects is `{ "rules": [ ... ] }`. Drop `zone_id`, `phase`, and `description` from the file before sending, or the API rejects the extra fields.

## An old update key file

A cloud agent store may still hold a file named `omacus-update.key`. This change does not open it. If that file is a private key, delete it from the store. Beta and release-candidate copies that trust it are the ones a leaked copy could sign for.

# Release and hosting protection

These settings are not in the workflow files. Create them before the next release or web deploy. The workflows name the environments `app` and `release`. The first run of a missing environment creates it with no reviewer, so create the rules first.

Hardware 2FA is not part of this change.

The account that approves is Avi (`avinashjoshi`, user id `172230`).

## Web app deploy

The App workflow's deploy job uses the environment `app`. Put the Cloudflare token and account id on that environment, then remove the repository copies after one approved deploy has succeeded. Until the repository copies are removed, another workflow can still read them.

```
gh api --method PUT repos/avinashjoshi/omacus/environments/app --input - <<'JSON'
{
  "wait_timer": 0,
  "prevent_self_review": false,
  "reviewers": [{ "type": "User", "id": 172230 }],
  "deployment_branch_policy": {
    "protected_branches": true,
    "custom_branch_policies": false
  }
}
JSON

gh secret set CLOUDFLARE_API_TOKEN --env app --repo avinashjoshi/omacus
gh secret set CLOUDFLARE_ACCOUNT_ID --env app --repo avinashjoshi/omacus
```

After a deploy has been approved and has finished:

```
gh secret delete CLOUDFLARE_API_TOKEN --repo avinashjoshi/omacus
gh secret delete CLOUDFLARE_ACCOUNT_ID --repo avinashjoshi/omacus
```

## Desktop release signing

The publish job uses the environment `release`. `MINISIGN_SECRET_KEY` has to live there. The version tag is pushed with `RELEASE_TOKEN`, which has to be a token for Avi, with permission to write contents. A missing token stops the tag.

```
gh api --method PUT repos/avinashjoshi/omacus/environments/release --input - <<'JSON'
{
  "wait_timer": 0,
  "prevent_self_review": false,
  "reviewers": [{ "type": "User", "id": 172230 }],
  "deployment_branch_policy": {
    "protected_branches": true,
    "custom_branch_policies": false
  }
}
JSON

gh secret set MINISIGN_SECRET_KEY --env release --repo avinashjoshi/omacus
gh secret set RELEASE_TOKEN --env release --repo avinashjoshi/omacus
```

After a signed release has been approved and has finished:

```
gh secret delete MINISIGN_SECRET_KEY --repo avinashjoshi/omacus
```

Leave a repository `RELEASE_TOKEN` in place only if the prepare job should still open the release pull request without an approval. The tag push reads the environment secret of the same name, which wins over the repository secret.

## Version tags

This ruleset blocks creating, moving, and deleting `v*` tags except for Avi. It does not grant a bypass to GitHub Actions. The publish job pushes the tag as `RELEASE_TOKEN`, so that token has to be Avi's.

```
gh api --method POST repos/avinashjoshi/omacus/rulesets --input - <<'JSON'
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
    { "type": "creation" },
    { "type": "update" },
    { "type": "deletion" }
  ],
  "bypass_actors": [
    { "actor_id": 172230, "actor_type": "User", "bypass_mode": "always" }
  ]
}
JSON
```

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

# Sync protocol

This is the wire format for the private sync test. The server stores opaque bytes. It does not receive recovery words, vault keys, content keys, page ids, page names, or page text.

Integers are big-endian. A chunk is a `u32` length followed by that many bytes. Text is UTF-8. Binary fields in JSON are standard base64. Object ids in JSON are 32 lowercase hex characters (16 random bytes). The account id is 64 lowercase hex characters (the account public key).

## Versioning

| Marker | Where | Meaning |
| --- | --- | --- |
| `OMACUS-SYNC/1\n` | Record, 14 bytes | One sealed page blob. At most 256 KiB. |
| `OMACUS-SYNC/2\n` | Record, 14 bytes | A sealed manifest. Page bodies are separate objects. |
| `OMACUS-MANIFEST/1\n` | Manifest plaintext | Membership, names, and one row per page. |
| `OMACUS-PAGE/1\n` | Object plaintext | One page's library bytes. |
| `OMACUS-DEVICES/1` | Device list | Signed device log. |
| `OMACUS-META/1\n` | Inside the manifest | Vault name, device names, membership. |
| `OMACUS-PAGES/1\n` | Version 1 page blob only | Every page in one plaintext, before it is sealed. |
| `PROTOCOL` = 1 | Associated data | Mixed into every wrap and page seal. |

A record that starts with neither magic is refused (`Unreadable`, HTTP 400) and not stored. A version 2 record may be up to 1 MiB. That cap is the manifest seal, not the pages.

The associated-data prefix on wraps and on the manifest seal is:

```text
"omacus/v1" || protocol(u16) || "personal" || epoch(u64) || key_id(16) || kind || recipient?
```

`kind` is `vault-key`, `phrase-key`, `recovery-key`, or `page`. `recipient` is present only for a device wrap.

A phone stores one local entry per account: the device file, the revision, the last synced pages, whether sync is paused, and the name. An older phone that stored a single slot is copied into that list once. The account id is still derived with info `omacus/v1/account-sign`. The `personal` marker above is not a vault name, and it stays when another account is stored beside the first.

## Record

Both versions share one envelope. `page` is the only field that changes meaning.

```text
magic(14)
vault_len(u8) || "personal"
epoch(u64)
account(32)
key_id(16)
chain(chunk)          device list
phrase_wrap(chunk)    vault key sealed for the recovery words
wrap_count(u16)
device_wrap(chunk) × wrap_count
page(chunk)           v1: sealed page blob. v2: sealed manifest.
signer(16) || signature(64)   present when a linked device signed the record
```

`key_id` is a random 16-byte id for the current vault key. It is not a page id.

A signed record ends with exactly 80 bytes: the signer id, then an Ed25519 signature over `omacus-record/1` concatenated with every byte before those 80. The signer is a device on the latest list, or the recovery signer. Anything else left over after the page chunk is corrupt. An older record may omit the 80 bytes. A device that can open that copy publishes a signed record. After this device has seen a signature, it rejects a copy that has none.

### Version 1 page blob

Plaintext magic `OMACUS-PAGES/1\n`, then the library document (chunk), then a `u32` page count, then each page as a `u16` id length, the id, and a chunk of text. Names and membership follow when they are present (`OMACUS-META/1`). The whole plaintext is sealed under the vault key. A save uploads that seal again, even when one line changed. The server cap is 256 KiB.

### Version 2 manifest

The `page` chunk is `seal_page` under the vault key with associated data for the page id `manifest` (the nine bytes, not a real page). Plaintext:

```text
"OMACUS-MANIFEST/1\n"
meta_len(u32) || meta
folder_count(u16)
  id(u16-len) name(u16-len) parent_flag(u8) [parent(u16-len) if flag is 1]
page_count(u32)
  object_id(16) page_id(u16-len) revision(u64) sha256(32) content_key(32)
generation(u64)
```

A string is a `u16` length and at most 200 bytes, with no control characters. Folder flag 0 means no parent. Meta is `OMACUS-META/1\n`: vault name (chunk), name revision (`u64`), device-name count (`u16`) of id(16) + rev(`u64`) + name(chunk), then membership count (`u32`) of page id (`u16` length) + rev(`u64`) + one byte (1 if the page is in the synced set).

The manifest is how a device learns which objects to fetch. The server never sees this plaintext. The last eight bytes are a counter. A manifest written before the counter is generation 0. Each new seal uses the next number. A new seal starts with `OMACUS-SEAL/2` and that generation, and the same generation is in the associated data with the page id and the epoch. A generation lower than the last one this phone opened is refused before the seal is opened. An older seal, without that header, still opens. The mailbox revision stays outside the seal and is not that counter.

### Page object

Each object is one page, or one page after a merge. Plaintext before padding:

```text
"OMACUS-PAGE/1\n" || doc_len(u32) || doc
```

`doc` is that page's library bytes. The plaintext is padded with zero bytes up to the smallest bucket that holds it, then sealed. Buckets, in bytes: 256, 1024, 4096, 16384, 65536, 262144, 1048576, 4194304. A page that does not fit the largest bucket is refused by the client.

The seal is XChaCha20-Poly1305 under a random 32-byte content key. The stored body is the 24-byte nonce, then the ciphertext and 16-byte tag. Associated data is `omacus-object-v1` concatenated with the 16-byte object id. The real length is the `u32` inside the seal. The stored length is 40 plus the bucket (nonce and tag). The server cap on one body is 4 MiB + 64.

Object ids are random. They are not page ids. An unchanged page keeps its object id, content key, and body, so a save does not upload it. A changed page keeps the object id, mints a new content key, and is uploaded with `base` set to the last revision this device saw for that object (`0` creates it). A page that leaves the synced set is omitted from the manifest and its object id is listed in `drop`.

## Keys

Recovery words are 12 words from a fixed list. They encode 128 bits and a checksum nibble. Argon2id (version `0x13`, 19 MiB, 2 iterations, 1 lane, 32-byte output, salt `omacus-v1`) derives the root from those 16 bytes, a zero byte, and an optional extra word. The salt is that fixed string. It separates this use from any other hash of the same bytes. The 128 random bits in the words are the secret, so a different salt per account would not hide them. The root is wiped after the device keys exist.

HKDF-SHA256, salt `omacus-v1`, expands the root:

| Info | Use |
| --- | --- |
| `omacus/v1/account-sign` | Ed25519 account key. The public key is the account id. The same key signs requests during recovery. |
| `omacus/v1/vault-wrap/0` | 32-byte key for an older phrase wrap. |
| `omacus/v1/recovery-receive/0` | X25519 receive key for a recovery wrap. |

Each device keeps its own Ed25519 signing key and its own X25519 receive key. Neither is derived from the recovery words.

The vault key is 32 random bytes plus the 16-byte `key_id`. It seals the manifest (and, on version 1, the page blob) with XChaCha20-Poly1305. It does not seal page objects. Those use the content keys stored inside the manifest.

Device wrap, HPKE (RFC 9180 base mode): DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305. Info and associated data are both the `vault-key` binding, including the recipient device id. The stored wrap is device id (16) || encapsulated key (32) || ciphertext. The plaintext is the vault key, and when present the recovery X25519 public key after it.

Recovery wrap, current form: `R2` || key id (16) || HPKE encapsulation of the vault key to the recovery receive key. Associated data is the `recovery-key` binding. An older phrase wrap is XChaCha20-Poly1305 under the phrase key, nonce || ciphertext, associated data `phrase-key`. A device can read the key id out of an `R2` wrap without the words. It cannot read the vault key.

Removing a device builds a new vault key and new wraps. The manifest is sealed again under that key. Page objects whose text did not change are not uploaded: the new manifest still holds their content keys. The next edit of a page uses a new content key, so a removed device that kept an old content key cannot read that later body. It can still read a body it already unwrapped.

## Device list

The list is a chain of signed snapshots. Each snapshot covers:

```text
"omacus-devices/1" || version(u64) || prev(32) || signer(16) || count(u16)
  || (id(16) || sign(32) || receive(32)) × count
```

The signature is Ed25519, 64 bytes, by the device named `signer`. `prev` is the hash of the previous snapshot. Version 1 has an empty `prev`. Each later version is exactly one device added or removed, and the signer is on the previous list. The signature does not cover the vault key. The stored chain is `OMACUS-DEVICES/1`, a `u16` count, then each snapshot as version, prev, signer, signature, device count, and the public keys. At most 64 devices.

A phone keeps the list it already checked. It takes a longer list that extends that one. It takes a list that starts over only when the first snapshot is signed by the account key, the same key as the recovery signer, and that key matches the one this phone already pinned. A vault key it has not seen is refused unless it arrives with one of those lists. The first list, on a phone that has none yet, is stored only after the six numbers are checked.

One list entry has the 16-byte id `recovery-signer` followed by a zero byte. Its verify key is the account public key. It is not a phone. A linked device adds it when publishing, so recovery can sign later requests. Removing a phone does not remove this entry. When the words open the latest pages, that signer adds the phone that typed them. The vault key stays the same, and the other phones see the longer list. An older phone adopts that longer recovery-signed list before it publishes. A shorter list is refused.

## Invite and join

A linked device picks a one-time code and sends SHA-256 of that code to `POST /v1/mailboxes/:account/invites`. The code stays on the device. The server keeps the hash for 5 minutes.

The other device sends `POST /v1/join` with the code, its public device bytes, a display name, and HMAC-SHA256 of those device bytes under the code. The server checks the hash, marks the code used, and stores a pending request. A second use of the same code fails while that phone is still waiting. If it stops waiting, the same code works again until the code expires. If it never finishes after approval, the other phone can release the approval, and the same code works again until it expires. Once that phone finishes, the same code does not work again. A code that has expired does not work. The phone that showed the code keeps looking until the code expires, so the next phone is asked about.

The picture is a link to this app, `https://app.omacus.com/#join=` on the published app and `https://sync-test.omacus.com/#join=` on the test. The part after `#` is the four lines, encoded: `omacus-join`, the origin, the account id, and the code. A browser does not send that part. An older picture of those four lines still works in the app.

Both screens show the same six digits. They are `SHA-256("omacus/v1/pair" || sign || receive)` reduced mod 1,000,000, printed as two groups of three. The digits are not a key.

The linked device checks the HMAC, then `POST .../requests/:id` with the new device list (`chain`), a device wrap (`wrap`), and the sealed record that already includes that phone. The record is stored in that same step, before the approval can be read. `reject: true` declines. The waiting device reads `GET .../requests/:id` with `X-Omacus-Proof` set to the same HMAC. That proof does not read the record, list requests, or approve anyone. It can read the approval again until it sends `{ done: true }`, the other phone sends `{ release: true }`, or the 5 minutes pass. A missing proof, a wrong proof, and an unknown request all return the same 404.

Starting a mailbox also spends an invite code (`POST /v1/access` checks it, `POST /v1/mailboxes` spends it). The server stores a hash, not the code.

## Endpoints

All bodies are JSON. `/v1/` answers are JSON. A missing mailbox is 404.

| Method | Path | Body | Response |
| --- | --- | --- | --- |
| GET | `/v1/health` | | `{ label }` |
| POST | `/v1/access` | `{ code }` | `{ ok: true }` |
| POST | `/v1/mailboxes` | `{ account, record, code, objects? }` | `{ revision, objects }` |
| GET | `/v1/mailboxes/:account` | `If-None-Match: "<revision>"` optional | `{ revision, record, objects: [{ id, revision, bytes }] }`, or `304` with an empty body when that revision is still current |
| PUT | `/v1/mailboxes/:account` | `{ revision, record, objects?: [{ id, base, body }], drop?: [id] }` | `{ revision, objects: [{ id, revision }] }` |
| POST | `/v1/mailboxes/:account/objects` | `{ ids }` | `{ objects: [{ id, revision, body }] }` |
| POST | `/v1/mailboxes/:account/invites` | `{ hash }` | `{ ok: true }` |
| POST | `/v1/join` | `{ code, device, name, proof }` | `{ id, account }` |
| GET | `/v1/mailboxes/:account/requests` | | `{ requests: [{ id, device, name, proof }] }` |
| GET | `/v1/mailboxes/:account/requests/:id` | | `{ state, name, device, chain, wrap }` |
| POST | `/v1/mailboxes/:account/requests/:id` | `{ chain, wrap, record, revision, objects?, drop? }`, `{ reject: true }`, `{ done: true }`, or `{ release: true }` | `{ ok: true, revision?, objects? }` |

GET returns an index. `bytes` is the stored length. Bodies are not in that response. POST `/objects` returns only the ids that were asked for, at most 2000. A missing id is 404 and the call returns no bodies.

PUT with an empty `objects` and an empty `drop` is a version 1 replace of the record. Otherwise the record, the object writes, and the drops commit together.

One HTTP body is at most 8 MiB. One account holds at most 100,000 objects. The manifest cap is 1 MiB. There is no 256 KiB cap on the set of pages.

Creating a mailbox may include objects. Each `base` is 0. The revision comes back as 1.

A quiet poll sends `If-None-Match` with the revision it already has. When that revision is still current the server answers `304` and does not send the record or the object list. The client downloads those only after the revision changes. The tag is the quoted decimal revision, such as `"4"`. A missing header, a different revision, or `*` still returns the record.

The admin routes are not part of this protocol. They are described in `docs/sync-ops.md`. They do not return page text, names, or keys.

## Requests

After a mailbox exists, every request except `GET .../requests/:id` is signed by a device on the latest list, or by the recovery signer. The message is:

```text
"omacus-auth/1\n" || METHOD || "\n" || path || "\n" || time_ms(u64) || nonce(16) || SHA-256(body)
```

`METHOD` is uppercase. `path` has no query. An empty body hashes as SHA-256 of zero bytes. The signature is over those exact bytes, not a re-parsed JSON value. Headers are `X-Omacus-Device` (32 hex), `X-Omacus-Time` (decimal unix milliseconds), `X-Omacus-Nonce` (32 hex), and `X-Omacus-Signature` (128 hex).

The server checks the signature against the latest device list inside the stored record. A device that was removed is on an older snapshot only, and is rejected. The recovery signer verifies with the account public key. If that entry is already on the list, its verify key must equal the account id in the record.

`time_ms` must be within 5 minutes of the server clock. Each nonce is remembered for that account and rejected the second time. The server keeps at most 64 recent nonces.

`POST /v1/mailboxes` stays on the invite code and does not need this signature. `POST /v1/join` stays on the invite HMAC. `GET /v1/health` is open.

A PUT must also carry a signed record. The new device list shares a prefix with the stored list and is the same length or longer, unless the request was signed by the recovery signer. Recovery may publish a new list. A device that is not the recovery signer cannot fork the list.

The server checks the signature before compare-and-swap. A PUT also names the revision it is replacing, in `If-Match`, as the same quoted number the GET returns in `ETag`. A missing or different tag is 409, the same as a stale `revision` in the body. A stale `revision` or object `base` is still 409, and only after the signature and the new record check out. The server keeps 64 nonces from the last 5 minutes. A new nonce is refused while those 64 are still inside the window. It does not drop an older one to make room. A missing account, a bad signature, a removed device, a full nonce table, and a replay are the same 404. The body does not say which.

## Compare-and-swap

A PUT names the mailbox `revision` it read. Each object write names `base`:

- `base` 0 creates the object. If that id already exists, nothing is written.
- Any other `base` must equal that object's current revision. A missing id, or a different revision, writes nothing.
- `drop` lists ids to remove. An id cannot be both written and dropped in one call.
- The mailbox revision and every object change commit together, or not at all. The mailbox revision then increases by 1. A created object's revision is 1. An updated object's revision is `base + 1`.

HTTP 409 means the client should read again, merge, and send only the pages that still differ. The client merges one page by merging that page's library bytes with the other copy of the same page. Two devices editing different pages do not rewrite each other's objects. Two devices editing the same page both keep their lines.

A save uploads the sealed objects whose pages changed. An unchanged page keeps its object id, its content key, and its body, and is not sent again.

## Members

The signed device list is the member list. Each entry is a phone or computer that may open the set. A save that adds or removes a phone changes that list and keeps the same record shape. A later collaboration change adds a person by adding an entry. It does not need a new envelope.

## Attachments

Not in this version. An attachment will be a sealed blob in R2. The account's Durable Object is the only writer. It checks the same signature as a page write, then stores the blob under an object id. The manifest will name that id the way it names a page. The server still cannot read the blob. The nightly copy will include it once that write exists. Nothing in this version stores an attachment.

## Nightly copies

Point-in-time recovery on the account's Durable Object is the first copy. The nightly copy is the second. A backup store is `put`, `get`, and `list`. R2 is one store. The test uses an in-memory store of the same shape. The worker writes each sealed account under `nights/YYYY-MM-DD/accounts/{account}/`. That is the sealed record (the manifest and the signed member list) and one file per sealed page object. The index for the night is counts, sizes, and which accounts did not save. It does not hold a record or a page. The bucket lifecycle rule deletes a night after 30 days. The worker does not.

Restore reads that night and writes it back through the same compare-and-swap as a PUT. The mailbox revision increases by 1. Each object from the night is created or updated. An object that was not in the night is dropped in that same commit. A phone that still has a page the night did not include sends that page on its next save, and the merge keeps one copy of each page. A page that was only in the night comes back. The restore is not a way to open a page.

A device keeps a local slot per page: object id, content hash, content key, server revision, and the page's library bytes. Those slots are not uploaded. They are how the next save knows which objects to skip.

## Migration

A version 1 record still opens. A record with no signature still opens once, on a device that has not yet seen a signature, and that device then publishes a signed record. The device that opens a version 1 record seals a version 2 manifest and one object per page on its next save, using the same page ids. A second device that loses that race gets 409, opens the version 2 record, and merges by page id, so a page is not stored twice and a page is not dropped. Recovery that can only open an older seal publishes a fresh version 2 manifest after it combines the pages already on the phone, and drops object ids the new manifest does not use. Nothing in that combine is deleted.

## Errors

The JSON body is `{ "message": "..." }`. The name is the server's name for the case. The sentence is what the client shows.

| Name | HTTP | When |
| --- | --- | --- |
| Unknown | 404 | No mailbox, a missing object, a bad or missing signature, a removed device, a replay, or a request proof that does not match. The sentence is the same in every case. |
| Conflict | 409 | That account already has a mailbox. |
| Revision | 409 | Mailbox revision or an object `base` did not match. Nothing was written. |
| Full | 429 | Too many join requests are waiting, or that account's object cap is full. |
| Unreadable | 400 | The body, the record magic, an id, or a size is not valid. |
| BadInvite | 400 | The join code did not match a live invite. |
| BadAccess | 400 | The invite code for a new mailbox did not match. |
| SignupFull | 403 | New signups are full. The invite code is not used. |
| Room | 429 | That account is out of room. |
| Devices | 429 | That account has as many phones and computers as it can. |
| Limited | 429 | Too many requests for one account (240 in 60 seconds) or one address (800 in 60 seconds). A code guess is 10 per address, a join attempt is 10 per address, and a read is 180 per account or 500 per address, each in 60 seconds. These are the production starting numbers. An admin can change them. The sentence is the same whether or not the account exists. The cap is checked before the account is looked up. |

A body over 8 MiB is HTTP 400 with the unreadable sentence, before it is parsed.

## What the server can see

The server can see the account id, the request time, the nonce, and the client address. It can see mailbox revisions, object ids, object revisions, and the stored length of each object. Those lengths fall on the buckets above, plus the 40-byte nonce and tag, so the size is a bucket, not the exact page length. It can see invite hashes, access-code hashes, join proofs, the display name on a join request, and device public keys if it parses the record envelope. The envelope is not encrypted as a whole. The chain, the key id, the epoch, and the wrap ciphertexts are in the record bytes.

The server cannot see recovery words, the root, a vault key, a content key, a page id, a page name, a folder name, the vault name, or page text. Those sit inside the sealed manifest or a sealed object. A removed device is not wrapped for the new vault key, so it cannot open a later manifest. Reading the index requires a signature from a device still on the list, or from the recovery signer. Knowing the account id is not enough to read a page, replace the record, or approve a phone. A client also checks that signature itself and rejects a record or manifest signed by anyone else, including a server that substitutes one.

## Storage

The mailbox does not require Durable Objects. A `Kv` stores opaque values by string key: get, put, delete, and list by prefix. The in-memory map and a directory of files both implement it. The file layout is a mailbox header at `box/{account}` and one object file at `obj/{account}/{id}` (revision as 8 bytes, then the raw body). A second process opening that directory reads the same bytes. The worker keeps the same shape inside an account's object: the mailbox key `m:{account}`, and one key `o:{account}:{id}` per object. Tests pass an in-memory store or a file store in place of the Durable Object storage. The worker source does not import the file adapter.

The worker places each account in its own object. A directory object stores invite hashes, access-code hashes, and one row per account (sizes and device counts, not page text). A config object named `index` stores the account cap, the storage cap, the device cap, and the rate limits. Production starts at 10,000 accounts, 50 MB (52,428,800 bytes) per account, 10 devices, and the rate numbers in the table above. The test worker starts at 3 accounts, 65,536 bytes, and 4 devices. The admin page and `admin.mjs limits` can change the saved numbers. The reference server keeps the production numbers and does not read that object. Once a night the worker copies each sealed account into the backup store. Point-in-time recovery on the Durable Object is the first copy. The bucket keeps a night for 30 days. Restoring writes that night back through the same revision a phone uses when it saves. The copy is not a way to open a page. A join reads the invite hash and opens that account. It does not list the other accounts. There is no fixed cap of 32 accounts. An account still on the older shared object is copied into its own object the first time it is used, then removed from the shared object. The reference server keeps every account in one process and uses the same invite index.

//! The mailbox is what a server would keep. It must not contain a key or a page.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Mutex};

use base64::Engine;
use omacus_engine::sync_keys::{PageSet, RecoveryKey};
use omacus_relay::store::Store;
use omacus_sync_web::desk::{self, Desk, SAMPLE_PAGE};

fn holds(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|window| window == needle)
}

/// A word sitting in the middle of a sealed blob is not the word itself.
fn holds_word(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return false;
    }
    let token = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=');
    hay.windows(needle.len())
        .enumerate()
        .any(|(index, window)| {
            if window != needle {
                return false;
            }
            let before = index.checked_sub(1).and_then(|at| hay.get(at)).copied();
            let after = hay.get(index + needle.len()).copied();
            !matches!((before, after), (Some(left), Some(right)) if token(left) && token(right))
        })
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

struct Started {
    port: u16,
    store: Arc<Mutex<Store>>,
}

fn start() -> Started {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut fresh = Store::new();
    fresh.set_access_secret(b"omacus-access-test-secret");
    let store = Arc::new(Mutex::new(fresh));
    let shared = Arc::clone(&store);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let www = root.join("www");
    let site = root.join("../site");
    std::thread::spawn(move || {
        omacus_relay::http::serve(listener, www, site, shared);
    });
    Started { port, store }
}

fn access_code(uses: u8, id: [u8; 6]) -> String {
    omacus_relay::access::mint(b"omacus-access-test-secret", uses, &id).expect("access code")
}

fn exchange_with(
    port: u16,
    method: &str,
    path: &str,
    body: &[u8],
    extra: &str,
    wire: &mut Vec<u8>,
) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    wire.extend_from_slice(head.as_bytes());
    wire.extend_from_slice(body);
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).unwrap();
    wire.extend_from_slice(&buf);
    let head_end = buf
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .unwrap();
    let head = String::from_utf8_lossy(&buf[..head_end]);
    let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, buf[head_end..].to_vec())
}

fn exchange(
    port: u16,
    method: &str,
    path: &str,
    body: &[u8],
    wire: &mut Vec<u8>,
) -> (u16, Vec<u8>) {
    exchange_with(port, method, path, body, "", wire)
}

fn signed(
    port: u16,
    desk: &Desk,
    method: &str,
    path: &str,
    body: &[u8],
    wire: &mut Vec<u8>,
) -> (u16, Vec<u8>) {
    let auth = desk.sign_request(method, path, body).unwrap();
    exchange_with(port, method, path, body, &auth.headers(), wire)
}

fn json_body(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap()
}

struct Joined {
    id: String,
    code: String,
    proof: String,
}

/// Opens a one-time code on `account`, rejects a join with no code and a join
/// with the wrong code, then joins once. The code is not kept afterwards.
fn post_join(
    port: u16,
    desk: &Desk,
    account: &str,
    device: &[u8],
    name: &str,
    wire: &mut Vec<u8>,
) -> Joined {
    let secret = [0x11u8, 0x22, 0x33, 0x44, 0x55];
    let code = omacus_relay::invite::encode_code(&secret);
    let hash = omacus_relay::invite::hash_hex(&secret);
    let mac = omacus_relay::invite::proof(&secret, device);
    let proof = b64(&mac);
    let invite = serde_json::json!({ "hash": hash }).to_string();
    let (status, _) = signed(
        port,
        desk,
        "POST",
        &format!("/v1/mailboxes/{account}/invites"),
        invite.as_bytes(),
        wire,
    );
    assert_eq!(status, 200);
    let (status, body) = exchange(
        port,
        "POST",
        "/v1/join",
        serde_json::json!({ "device": b64(device), "name": name, "proof": b64(&mac) })
            .to_string()
            .as_bytes(),
        wire,
    );
    assert_eq!(status, 400, "a join with no code is dropped");
    assert_eq!(
        json_body(&body)["message"],
        "That code didn't work. Check it on the other device."
    );
    let (status, body) = exchange(
        port,
        "POST",
        "/v1/join",
        serde_json::json!({ "code": "0000-0001", "device": b64(device), "name": name, "proof": b64(&mac) })
            .to_string()
            .as_bytes(),
        wire,
    );
    assert_eq!(status, 400);
    assert_eq!(
        json_body(&body)["message"],
        "That code didn't work. Check it on the other device."
    );
    let (status, _) = exchange(
        port,
        "POST",
        &format!("/v1/mailboxes/{account}/requests"),
        serde_json::json!({ "device": b64(device), "name": name })
            .to_string()
            .as_bytes(),
        wire,
    );
    assert_eq!(status, 404);
    let (status, body) = signed(
        port,
        desk,
        "GET",
        &format!("/v1/mailboxes/{account}/requests"),
        b"",
        wire,
    );
    assert_eq!(status, 200);
    assert!(json_body(&body)["requests"].as_array().unwrap().is_empty());
    let (status, _) = exchange(port, "GET", "/v1/mailboxes", b"", wire);
    assert_eq!(status, 404);
    let (status, body) = exchange(
        port,
        "POST",
        "/v1/join",
        serde_json::json!({ "code": code, "device": b64(device), "name": name, "proof": b64(&mac) })
            .to_string()
            .as_bytes(),
        wire,
    );
    assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));
    let joined = json_body(&body);
    assert_eq!(joined["account"], account);
    let id = joined["id"].as_str().unwrap().to_string();
    let (status, _) = exchange(
        port,
        "POST",
        "/v1/join",
        serde_json::json!({ "code": code, "device": b64(device), "name": name, "proof": b64(&mac) })
            .to_string()
            .as_bytes(),
        wire,
    );
    assert_eq!(status, 400, "a code works once");
    Joined { id, code, proof }
}

#[test]
fn the_relay_holds_no_key_and_no_page() {
    let Started { port, store } = start();
    let mut wire = Vec::new();

    let mut founder = Desk::start_new().unwrap();
    let words = founder.words().unwrap();
    let picks = [(3, words[2]), (7, words[6]), (11, words[10])];
    let saved = RecoveryKey::parse(&words.join(" ")).unwrap();
    let confirmed = saved.confirm(&picks).unwrap();
    let entropy = *confirmed.entropy_bytes();
    let root = confirmed.root("").unwrap();
    let root_bytes = *root.as_bytes();
    let sign = *founder.device().signing_secret();
    let receive = *founder.device().receive_secret();

    let record = founder.confirm_saved(&picks).unwrap();
    let account = founder.account_hex().unwrap();
    let vault = *founder.vault().unwrap().secret();
    let (status, _) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        serde_json::json!({"account": account, "record": b64(&record), "code": access_code(1, [1, 0, 0, 0, 0, 1])})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 201);

    let mut joiner = Desk::start_join().unwrap();
    let join_sign = *joiner.device().signing_secret();
    let join_receive = *joiner.device().receive_secret();
    let device = joiner.public_device();
    let code = joiner.pairing_code();
    assert_eq!(code, desk::code_for_device(&device).unwrap());
    let joined = post_join(port, &founder, &account, &device, "This phone", &mut wire);
    let id = joined.id.clone();

    let (status, body) = signed(
        port,
        &founder,
        "GET",
        &format!("/v1/mailboxes/{account}/requests"),
        b"",
        &mut wire,
    );
    assert_eq!(status, 200);
    let pending = &json_body(&body)["requests"][0];
    assert_eq!(pending["name"], "This phone");
    let posted = base64::engine::general_purpose::STANDARD
        .decode(pending["device"].as_str().unwrap())
        .unwrap();
    assert_eq!(desk::code_for_device(&posted).unwrap(), code);

    let approval = founder.approve_new(&device).unwrap();
    let chain = approval.chain;
    let wrap = approval.wrap;
    let updated = approval.record;
    let put = serde_json::json!({"revision": 1, "record": b64(&updated)}).to_string();
    let (status, _) = signed(
        port,
        &founder,
        "PUT",
        &format!("/v1/mailboxes/{account}"),
        put.as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 200);
    let approve = serde_json::json!({"chain": b64(&chain), "wrap": b64(&wrap)}).to_string();
    let (status, _) = signed(
        port,
        &founder,
        "POST",
        &format!("/v1/mailboxes/{account}/requests/{id}"),
        approve.as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 200);

    let (status, body) = exchange_with(
        port,
        "GET",
        &format!("/v1/mailboxes/{account}/requests/{id}"),
        b"",
        &format!("X-Omacus-Proof: {}\r\n", joined.proof),
        &mut wire,
    );
    assert_eq!(status, 200);
    let decision = json_body(&body);
    assert_eq!(decision["state"], "approved");
    let (status, body) = exchange(
        port,
        "GET",
        &format!("/v1/mailboxes/{account}/requests/{id}"),
        b"",
        &mut wire,
    );
    assert_eq!(status, 404, "an approved request is not kept");
    assert_eq!(
        json_body(&body)["message"],
        "This test server has no pages for that yet."
    );
    let (status, body) = signed(
        port,
        &founder,
        "GET",
        &format!("/v1/mailboxes/{account}"),
        b"",
        &mut wire,
    );
    assert_eq!(status, 200);
    let stored_record = base64::engine::general_purpose::STANDARD
        .decode(json_body(&body)["record"].as_str().unwrap())
        .unwrap();
    let chain_bytes = base64::engine::general_purpose::STANDARD
        .decode(decision["chain"].as_str().unwrap())
        .unwrap();
    let wrap_bytes = base64::engine::general_purpose::STANDARD
        .decode(decision["wrap"].as_str().unwrap())
        .unwrap();
    let page = joiner
        .open_approval(&chain_bytes, &wrap_bytes, &stored_record)
        .unwrap();
    assert_eq!(page, SAMPLE_PAGE);
    assert_eq!(joiner.vault().unwrap().secret(), &vault);

    let pasted = format!(
        "1. {}\n2. {}, {}\n{}\n{} {}\n6. {}\n{}\n{}\n{}\n10. {}\n{}",
        words[0].to_uppercase(),
        words[1],
        words[2],
        words[3],
        words[4],
        words[5],
        words[6],
        words[7],
        words[8],
        words[9],
        words[10],
        words[11],
    );
    let mut recovered = Desk::start_recovery(&pasted).unwrap();
    let found = recovered.recovery_account(&picks).unwrap();
    assert_eq!(found, account);
    let again = recovered.recovery_open(&stored_record).unwrap();
    assert_eq!(again, SAMPLE_PAGE);

    let kept = store.lock().unwrap().stored_bytes();
    assert!(
        !holds(&kept, joined.code.as_bytes()),
        "the stored mailbox kept the join code"
    );
    let mut secrets = vec![
        SAMPLE_PAGE.as_bytes().to_vec(),
        root_bytes.to_vec(),
        entropy.to_vec(),
        sign.to_vec(),
        receive.to_vec(),
        vault.to_vec(),
        join_sign.to_vec(),
        join_receive.to_vec(),
        pasted.into_bytes(),
    ];
    for word in words {
        secrets.push(word.as_bytes().to_vec());
    }
    for secret in &secrets {
        assert!(!holds(&kept, secret), "stored copy holds a secret");
        let on_wire = if secret.iter().all(|byte| byte.is_ascii_alphabetic()) {
            holds_word(&wire, secret)
        } else {
            holds(&wire, secret)
        };
        assert!(!on_wire, "the wire holds a secret");
    }

    let attack = b64(words.join(" ").as_bytes());
    let (status, _) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        serde_json::json!({"account": "cd".repeat(32), "record": attack, "code": access_code(1, [1, 0, 0, 0, 0, 2])})
            .to_string()
            .as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 400);
    let kept = store.lock().unwrap().stored_bytes();
    for word in words {
        assert!(!holds(&kept, word.as_bytes()));
    }
}

#[test]
fn a_full_page_set_stays_sealed_and_a_reload_stays_linked() {
    let Started { port, store } = start();
    let mut wire = Vec::new();

    let mut founder = Desk::start_new().unwrap();
    let words = founder.words().unwrap();
    let picks = [(3, words[2]), (7, words[6]), (11, words[10])];
    founder.confirm_saved(&picks).unwrap();
    let account = founder.account_hex().unwrap();
    let sign = *founder.device().signing_secret();
    let vault = *founder.vault().unwrap().secret();
    let trip = "Trip Budget\nferry = 40\n";
    let market = "Market note\nmilk $3\n";
    let packed = PageSet {
        doc: b"LOCAL-DOC-MARKER".to_vec(),
        pages: vec![
            ("trip".to_string(), trip.to_string()),
            ("market".to_string(), market.to_string()),
        ],
        meta: Default::default(),
    }
    .to_bytes()
    .unwrap();
    let record = founder.seal_pages(&packed).unwrap();
    let file = founder.export_local().unwrap();
    let (status, _) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        serde_json::json!({"account": account, "record": b64(&record), "code": access_code(1, [1, 0, 0, 0, 0, 3])})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 201);

    let mut reloaded = Desk::import_local(&file).unwrap();
    assert_eq!(reloaded.account_hex().as_deref(), Some(account.as_str()));
    let opened = reloaded.absorb_record(&record).unwrap();
    assert!(opened.member);
    let again = PageSet::from_bytes(&opened.pages).unwrap();
    assert!(
        again
            .pages
            .iter()
            .any(|(id, text)| id == "trip" && text == trip)
    );
    assert!(
        again
            .pages
            .iter()
            .any(|(id, text)| id == "market" && text == market)
    );

    let mut joiner = Desk::start_join().unwrap();
    let device = joiner.public_device();
    let joined = post_join(port, &founder, &account, &device, "Other device", &mut wire);
    let id = joined.id;
    assert!(!holds(
        &store.lock().unwrap().stored_bytes(),
        joined.code.as_bytes()
    ));
    let approval = founder.approve_new(&device).unwrap();
    let updated = founder.seal_pages(&packed).unwrap();
    let put = serde_json::json!({"revision": 1, "record": b64(&updated)}).to_string();
    let (status, _) = signed(
        port,
        &founder,
        "PUT",
        &format!("/v1/mailboxes/{account}"),
        put.as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 200);
    let approve =
        serde_json::json!({"chain": b64(&approval.chain), "wrap": b64(&approval.wrap)}).to_string();
    let (status, _) = signed(
        port,
        &founder,
        "POST",
        &format!("/v1/mailboxes/{account}/requests/{id}"),
        approve.as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 200);
    let (status, body) = signed(
        port,
        &founder,
        "GET",
        &format!("/v1/mailboxes/{account}"),
        b"",
        &mut wire,
    );
    assert_eq!(status, 200);
    let stored_record = base64::engine::general_purpose::STANDARD
        .decode(json_body(&body)["record"].as_str().unwrap())
        .unwrap();
    let (status, body) = exchange_with(
        port,
        "GET",
        &format!("/v1/mailboxes/{account}/requests/{id}"),
        b"",
        &format!("X-Omacus-Proof: {}\r\n", joined.proof),
        &mut wire,
    );
    assert_eq!(status, 200);
    let decision = json_body(&body);
    let chain_bytes = base64::engine::general_purpose::STANDARD
        .decode(decision["chain"].as_str().unwrap())
        .unwrap();
    let wrap_bytes = base64::engine::general_purpose::STANDARD
        .decode(decision["wrap"].as_str().unwrap())
        .unwrap();
    let joined = joiner
        .open_approval_pages(&chain_bytes, &wrap_bytes, &stored_record)
        .unwrap();
    let joined_pages = PageSet::from_bytes(&joined).unwrap();
    assert!(
        joined_pages
            .pages
            .iter()
            .any(|(_, text)| text.contains("ferry = 40"))
    );
    assert!(
        joined_pages
            .pages
            .iter()
            .any(|(_, text)| text.contains("milk $3"))
    );

    let mut origin = omacus_doc::Library::new();
    origin.reconcile("trip", trip, None);
    origin.note_written("trip", trip);
    origin.reconcile("market", market, None);
    origin.note_written("market", market);
    let shared = origin.save();
    let cake = "Trip Budget\nferry = 40\ncake $5\n";
    let bread = "Trip Budget\nferry = 40\nbread $2\n";
    let mut left_lib = omacus_doc::Library::load(&shared).unwrap();
    left_lib.reconcile("trip", cake, None);
    left_lib.note_written("trip", cake);
    let mut right_lib = omacus_doc::Library::load(&shared).unwrap();
    right_lib.reconcile("trip", bread, None);
    right_lib.note_written("trip", bread);
    let left = PageSet {
        doc: left_lib.save(),
        pages: vec![
            ("trip".to_string(), cake.to_string()),
            ("market".to_string(), market.to_string()),
        ],
        meta: Default::default(),
    };
    let right = PageSet {
        doc: right_lib.save(),
        pages: vec![
            ("trip".to_string(), bread.to_string()),
            ("market".to_string(), market.to_string()),
        ],
        meta: Default::default(),
    };
    let merged = left.merge(right);
    let trip_text = merged
        .pages
        .iter()
        .find(|(id, _)| id == "trip")
        .unwrap()
        .1
        .as_str();
    assert!(trip_text.contains("ferry = 40"), "{trip_text}");
    assert!(trip_text.contains("cake $5"), "{trip_text}");
    assert!(trip_text.contains("bread $2"), "{trip_text}");
    assert!(merged.pages.iter().any(|(id, _)| id == "market"));
    let merged_record = founder.seal_pages(&merged.to_bytes().unwrap()).unwrap();
    let merged_put = serde_json::json!({"revision": 2, "record": b64(&merged_record)}).to_string();
    let (status, _) = signed(
        port,
        &founder,
        "PUT",
        &format!("/v1/mailboxes/{account}"),
        merged_put.as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 200);

    let pasted = words.join(" ");
    let mut recovered = Desk::start_recovery(&pasted).unwrap();
    assert_eq!(recovered.recovery_account(&picks).unwrap(), account);
    let (status, body) = signed(
        port,
        &recovered,
        "GET",
        &format!("/v1/mailboxes/{account}"),
        b"",
        &mut wire,
    );
    assert_eq!(status, 200);
    let latest = base64::engine::general_purpose::STANDARD
        .decode(json_body(&body)["record"].as_str().unwrap())
        .unwrap();
    let opened = recovered.recovery_open_pages(&latest).unwrap();
    assert!(!opened.partial);
    let recovered_pages = PageSet::from_bytes(&opened.pages).unwrap();
    let recovered_trip = recovered_pages
        .pages
        .iter()
        .find(|(id, _)| id == "trip")
        .unwrap()
        .1
        .as_str();
    assert!(recovered_trip.contains("cake $5"));
    assert!(recovered_trip.contains("bread $2"));
    assert!(
        recovered_trip.contains("milk $3")
            || recovered_pages
                .pages
                .iter()
                .any(|(_, text)| text.contains("milk $3"))
    );

    let join_id = *joiner.device().public().id();
    founder.remove_listed(&join_id).unwrap();
    let listed = founder.listed_devices().unwrap();
    assert!(listed.iter().any(|(_, _, mine)| *mine));
    assert!(!listed.iter().any(|(id, _, _)| id == &hex_id(&join_id)));

    let (status, _) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        serde_json::json!({"account": "ef".repeat(32), "record": b64(&file), "code": access_code(1, [1, 0, 0, 0, 0, 4])})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 400);

    let kept = store.lock().unwrap().stored_bytes();
    for secret in [
        trip.as_bytes(),
        market.as_bytes(),
        b"LOCAL-DOC-MARKER".as_slice(),
        b"cake $5".as_slice(),
        b"bread $2".as_slice(),
        sign.as_slice(),
        vault.as_slice(),
        file.as_slice(),
        pasted.as_bytes(),
    ] {
        assert!(!holds(&kept, secret), "stored copy holds a page or a key");
        assert!(!holds(&wire, secret), "the wire holds a page or a key");
    }
    for word in words {
        assert!(!holds(&kept, word.as_bytes()));
        assert!(!holds(&wire, word.as_bytes()));
    }
}

fn hex_id(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[test]
fn the_server_crate_has_no_opener() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let deps = toml.split("[dev-dependencies]").next().unwrap();
    assert!(
        !deps.contains("omacus-engine"),
        "the server must not link the key library"
    );
    assert!(
        toml.split("[dev-dependencies]")
            .nth(1)
            .unwrap()
            .contains("omacus-engine")
    );
    for source in [
        "src/lib.rs",
        "src/store.rs",
        "src/http.rs",
        "src/invite.rs",
        "src/access.rs",
        "src/auth.rs",
        "src/main.rs",
    ] {
        let text = std::fs::read_to_string(root.join(source)).unwrap();
        for word in [
            "open_page",
            "open_phrase",
            "open_device",
            "RecoveryKey",
            "VaultKey",
        ] {
            assert!(!text.contains(word), "{source} mentions {word}");
        }
    }
}

#[test]
fn the_test_pages_do_not_name_private_parts_or_the_live_app() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut text = String::new();
    for name in ["www/index.html", "www/app.css", "www/app.js", "README.md"] {
        text.push_str(
            &std::fs::read_to_string(root.join(name))
                .unwrap()
                .to_lowercase(),
        );
    }
    for word in [
        "root key",
        "hpke",
        "vault key",
        "receive key",
        "signing key",
        "ciphertext",
        "argon",
        "entropy",
        "private key",
        "x25519",
        "chacha",
        "plaintext",
        "indexeddb",
        "app.omacus.com",
    ] {
        assert!(!text.contains(word), "the test pages say {word}");
    }
    assert!(text.contains("1.0.0-beta.22"));
    assert!(
        !std::fs::read_to_string(root.join("../site/app/index.html"))
            .unwrap()
            .contains("1.0.0-beta")
    );
}

#[test]
fn an_invite_code_is_spent_once_and_is_not_stored() {
    let Started { port, store } = start();
    let record = b64(b"OMACUS-SYNC/1\nsealed-test-record");
    let wrong = serde_json::json!({
        "account": "aa".repeat(32),
        "record": record,
        "code": "nope",
    })
    .to_string();
    let (status, body) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        wrong.as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 400);
    assert_eq!(
        json_body(&body)["message"],
        "That invite code didn't work. Check it and try again."
    );
    let (status, _) = exchange(
        port,
        "GET",
        &format!("/v1/mailboxes/{}", "aa".repeat(32)),
        b"",
        &mut Vec::new(),
    );
    assert_eq!(status, 404, "a wrong code creates no pages");

    let code = access_code(1, [1, 2, 3, 4, 5, 6]);
    assert_eq!(code, "0410-6105-0R0N-S1ZK-97T3-78B1");
    let peek = serde_json::json!({ "code": code }).to_string();
    let (status, _) = exchange(port, "POST", "/v1/access", peek.as_bytes(), &mut Vec::new());
    assert_eq!(status, 200);
    let (status, _) = exchange(port, "POST", "/v1/access", peek.as_bytes(), &mut Vec::new());
    assert_eq!(status, 200, "looking at a code does not use it up");

    let first = "ab".repeat(32);
    let create =
        serde_json::json!({ "account": first, "record": record, "code": code }).to_string();
    let (status, _) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        create.as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 201);
    let again = serde_json::json!({
        "account": "ac".repeat(32),
        "record": record,
        "code": code,
    })
    .to_string();
    let (status, body) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        again.as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 400, "a code works once");
    assert_eq!(
        json_body(&body)["message"],
        "That invite code didn't work. Check it and try again."
    );
    let (status, body) = exchange(
        port,
        "PUT",
        &format!("/v1/mailboxes/{first}"),
        serde_json::json!({ "revision": 1, "record": record })
            .to_string()
            .as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 404, "an unsigned update is refused");
    assert_eq!(
        json_body(&body)["message"],
        "This test server has no pages for that yet."
    );

    let mut founder = Desk::start_new().unwrap();
    let words = founder.words().unwrap();
    let picks = [(3, words[2]), (7, words[6]), (11, words[10])];
    let sealed = founder.confirm_saved(&picks).unwrap();
    let live = founder.account_hex().unwrap();
    let again_code = access_code(1, [3, 3, 3, 3, 3, 3]);
    let create = serde_json::json!({
        "account": live,
        "record": b64(&sealed),
        "code": again_code,
    })
    .to_string();
    let (status, _) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        create.as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 201);
    let update = serde_json::json!({ "revision": 1, "record": b64(&sealed) }).to_string();
    let (status, _) = signed(
        port,
        &founder,
        "PUT",
        &format!("/v1/mailboxes/{live}"),
        update.as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 200, "pages that already exist do not ask again");

    let twice = access_code(2, [9, 8, 7, 6, 5, 4]);
    for (n, account) in ["ad", "ae"].iter().enumerate() {
        let body = serde_json::json!({
            "account": account.repeat(32),
            "record": record,
            "code": twice,
        })
        .to_string();
        let (status, _) = exchange(
            port,
            "POST",
            "/v1/mailboxes",
            body.as_bytes(),
            &mut Vec::new(),
        );
        assert_eq!(status, 201, "use {}", n + 1);
    }
    let third = serde_json::json!({
        "account": "af".repeat(32),
        "record": record,
        "code": twice,
    })
    .to_string();
    let (status, _) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        third.as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 400, "the third use is refused");

    let kept = store.lock().unwrap().stored_bytes();
    let secret = b"omacus-access-test-secret";
    assert!(!holds(&kept, secret), "the stored copy kept the secret");
    assert!(
        !holds(&kept, code.as_bytes()),
        "the stored copy kept the code"
    );
    assert!(
        !holds(&kept, code.replace('-', "").as_bytes()),
        "the stored copy kept the code"
    );
}

#[test]
fn an_unsigned_request_a_removed_phone_and_a_replay_are_refused() {
    let Started { port, .. } = start();
    let mut founder = Desk::start_new().unwrap();
    let words = founder.words().unwrap();
    let picks = [(3, words[2]), (7, words[6]), (11, words[10])];
    let record = founder.confirm_saved(&picks).unwrap();
    let account = founder.account_hex().unwrap();
    let path = format!("/v1/mailboxes/{account}");
    let missing = "This test server has no pages for that yet.";
    let create = serde_json::json!({
        "account": account,
        "record": b64(&record),
        "code": access_code(1, [4, 4, 4, 4, 4, 4]),
    })
    .to_string();
    let (status, _) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        create.as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 201);

    let (status, body) = exchange(port, "GET", &path, b"", &mut Vec::new());
    assert_eq!(status, 404);
    assert_eq!(json_body(&body)["message"], missing);
    let refused = serde_json::json!({"revision": 1, "record": b64(&record)}).to_string();
    let (status, body) = exchange(port, "PUT", &path, refused.as_bytes(), &mut Vec::new());
    assert_eq!(status, 404);
    assert_eq!(json_body(&body)["message"], missing);
    let (status, body) = signed(port, &founder, "GET", &path, b"", &mut Vec::new());
    assert_eq!(status, 200);
    let kept = base64::engine::general_purpose::STANDARD
        .decode(json_body(&body)["record"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        kept, record,
        "an unsigned update does not replace the pages"
    );

    let auth = founder.sign_request("GET", &path, b"").unwrap();
    let (status, _) = exchange_with(port, "GET", &path, b"", &auth.headers(), &mut Vec::new());
    assert_eq!(status, 200);
    let (status, body) = exchange_with(port, "GET", &path, b"", &auth.headers(), &mut Vec::new());
    assert_eq!(status, 404, "a repeated request is refused");
    assert_eq!(json_body(&body)["message"], missing);

    let joiner = Desk::start_join().unwrap();
    let device = joiner.public_device();
    let joined = post_join(
        port,
        &founder,
        &account,
        &device,
        "Harbor phone",
        &mut Vec::new(),
    );
    let approval = founder.approve_new(&device).unwrap();
    let put = serde_json::json!({"revision": 1, "record": b64(&approval.record)}).to_string();
    let (status, _) = signed(
        port,
        &founder,
        "PUT",
        &path,
        put.as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 200);
    let approve =
        serde_json::json!({"chain": b64(&approval.chain), "wrap": b64(&approval.wrap)}).to_string();
    let (status, _) = signed(
        port,
        &founder,
        "POST",
        &format!("/v1/mailboxes/{account}/requests/{}", joined.id),
        approve.as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 200);
    let (status, _) = signed(port, &joiner, "GET", &path, b"", &mut Vec::new());
    assert_eq!(status, 200, "a linked phone can read");

    let removed = founder
        .remove_listed(joiner.device().public().id())
        .unwrap();
    let drop = serde_json::json!({"revision": 2, "record": b64(&removed)}).to_string();
    let (status, _) = signed(
        port,
        &founder,
        "PUT",
        &path,
        drop.as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(status, 200, "removing a phone still saves");
    let (status, body) = signed(port, &joiner, "GET", &path, b"", &mut Vec::new());
    assert_eq!(status, 404, "a removed phone is refused");
    assert_eq!(json_body(&body)["message"], missing);
    let (status, _) = signed(port, &founder, "GET", &path, b"", &mut Vec::new());
    assert_eq!(status, 200);

    let unknown = "ee".repeat(32);
    let (status, body) = exchange(
        port,
        "GET",
        &format!("/v1/mailboxes/{unknown}"),
        b"",
        &mut Vec::new(),
    );
    assert_eq!(status, 404);
    assert_eq!(
        json_body(&body)["message"],
        missing,
        "a missing account reads the same"
    );
}

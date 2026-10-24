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
    let store = Arc::new(Mutex::new(Store::new()));
    let shared = Arc::clone(&store);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let www = root.join("www");
    let site = root.join("../site");
    std::thread::spawn(move || {
        omacus_relay::http::serve(listener, www, site, shared);
    });
    Started { port, store }
}

fn exchange(
    port: u16,
    method: &str,
    path: &str,
    body: &[u8],
    wire: &mut Vec<u8>,
) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

fn json_body(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap()
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
        serde_json::json!({"account": account, "record": b64(&record)})
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
    let (status, body) = exchange(
        port,
        "POST",
        &format!("/v1/mailboxes/{account}/requests"),
        serde_json::json!({"device": b64(&device), "name": "This phone"})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));
    let id = json_body(&body)["id"].as_str().unwrap().to_string();

    let (status, body) = exchange(
        port,
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
    let (status, _) = exchange(
        port,
        "PUT",
        &format!("/v1/mailboxes/{account}"),
        serde_json::json!({"revision": 1, "record": b64(&updated)})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 200);
    let (status, _) = exchange(
        port,
        "POST",
        &format!("/v1/mailboxes/{account}/requests/{id}"),
        serde_json::json!({"chain": b64(&chain), "wrap": b64(&wrap)})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 200);

    let (status, body) = exchange(
        port,
        "GET",
        &format!("/v1/mailboxes/{account}/requests/{id}"),
        b"",
        &mut wire,
    );
    assert_eq!(status, 200);
    let decision = json_body(&body);
    assert_eq!(decision["state"], "approved");
    let (status, body) = exchange(
        port,
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
        assert!(!holds(&wire, secret), "the wire holds a secret");
    }

    let attack = b64(words.join(" ").as_bytes());
    let (status, _) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        serde_json::json!({"account": "cd".repeat(32), "record": attack})
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
    }
    .to_bytes()
    .unwrap();
    let record = founder.seal_pages(&packed).unwrap();
    let file = founder.export_local().unwrap();
    let (status, _) = exchange(
        port,
        "POST",
        "/v1/mailboxes",
        serde_json::json!({"account": account, "record": b64(&record)})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 201);

    let mut reloaded = Desk::import_local(&file).unwrap();
    assert_eq!(reloaded.account_hex().as_deref(), Some(account.as_str()));
    let (opened, member) = reloaded.absorb_record(&record).unwrap();
    assert!(member);
    let again = PageSet::from_bytes(&opened).unwrap();
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
    let (status, body) = exchange(
        port,
        "POST",
        &format!("/v1/mailboxes/{account}/requests"),
        serde_json::json!({"device": b64(&device), "name": "Other device"})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 201);
    let id = json_body(&body)["id"].as_str().unwrap().to_string();
    let approval = founder.approve_new(&device).unwrap();
    let updated = founder.seal_pages(&packed).unwrap();
    let (status, _) = exchange(
        port,
        "PUT",
        &format!("/v1/mailboxes/{account}"),
        serde_json::json!({"revision": 1, "record": b64(&updated)})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 200);
    let (status, _) = exchange(
        port,
        "POST",
        &format!("/v1/mailboxes/{account}/requests/{id}"),
        serde_json::json!({"chain": b64(&approval.chain), "wrap": b64(&approval.wrap)})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 200);
    let (status, body) = exchange(
        port,
        "GET",
        &format!("/v1/mailboxes/{account}"),
        b"",
        &mut wire,
    );
    assert_eq!(status, 200);
    let stored_record = base64::engine::general_purpose::STANDARD
        .decode(json_body(&body)["record"].as_str().unwrap())
        .unwrap();
    let (status, body) = exchange(
        port,
        "GET",
        &format!("/v1/mailboxes/{account}/requests/{id}"),
        b"",
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

    let mut left = joined_pages.clone();
    left.pages
        .iter_mut()
        .find(|(id, _)| id == "trip")
        .unwrap()
        .1 = "Trip Budget\nferry = 40\ncake $5\n".to_string();
    let mut right = again.clone();
    right
        .pages
        .iter_mut()
        .find(|(id, _)| id == "trip")
        .unwrap()
        .1 = "Trip Budget\nferry = 40\nbread $2\n".to_string();
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
    let (status, _) = exchange(
        port,
        "PUT",
        &format!("/v1/mailboxes/{account}"),
        serde_json::json!({"revision": 2, "record": b64(&merged_record)})
            .to_string()
            .as_bytes(),
        &mut wire,
    );
    assert_eq!(status, 200);

    let pasted = words.join(" ");
    let mut recovered = Desk::start_recovery(&pasted).unwrap();
    assert_eq!(recovered.recovery_account(&picks).unwrap(), account);
    let (status, body) = exchange(
        port,
        "GET",
        &format!("/v1/mailboxes/{account}"),
        b"",
        &mut wire,
    );
    assert_eq!(status, 200);
    let latest = base64::engine::general_purpose::STANDARD
        .decode(json_body(&body)["record"].as_str().unwrap())
        .unwrap();
    let recovered_pages =
        PageSet::from_bytes(&recovered.recovery_open_pages(&latest).unwrap()).unwrap();
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
        serde_json::json!({"account": "ef".repeat(32), "record": b64(&file)})
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
    for source in ["src/lib.rs", "src/store.rs", "src/http.rs", "src/main.rs"] {
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
    assert!(text.contains("1.0.0-beta.1"));
    assert!(
        !std::fs::read_to_string(root.join("../site/app/index.html"))
            .unwrap()
            .contains("1.0.0-beta.1")
    );
}

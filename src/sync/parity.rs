//! One pass through the parity matrix against a relay on this machine.
//! Confirming the words and opening them again are the slow steps, so this
//! test does each of those once.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::keys;
use super::qr;
use super::session::{
    CLIPBOARD_SECS, Chip, LocalPages, PARITY_ROWS, Screen, Session, should_clear,
};

#[test]
fn parity_matrix_runs_end_to_end() {
    let mut covered = Vec::new();
    let (origin, invite_code) = start_relay();
    let mut a = Session::open(scratch("a"), &origin);

    a.open_intro();
    let intro = super::session::intro_lines().join(" ");
    assert!(intro.contains("encrypted on this computer"));
    assert!(intro.contains("12 recovery words"));
    assert!(intro.contains("link another phone or computer"));
    assert!(intro.contains("stay as files on this computer"));
    assert!(!intro.to_lowercase().contains("paused"));
    assert_eq!(a.chip(Instant::now()), Chip::Hidden);
    covered.push("explainer");

    a.begin_setup();
    assert_eq!(a.screen, Screen::Choice);
    covered.push("choice");

    a.choose_first();
    assert_eq!(a.screen, Screen::Start);
    assert_eq!(a.chip(Instant::now()), Chip::Hidden);
    covered.push("start");

    a.set_name("Harbor desk");
    a.set_access("not-a-code");
    let missed = a.continue_access().unwrap_err();
    assert_eq!(
        missed,
        "That invite code didn't work. Check it and try again."
    );
    covered.push("invite-only");

    a.set_access(&invite_code);
    a.continue_access().unwrap();
    assert_eq!(a.screen, Screen::Words);
    let words = a.words_grid().unwrap().clone();
    assert_eq!(words.len(), 12);
    let job = a.print_words().unwrap();
    assert!(job.path.is_none());
    assert_eq!(CLIPBOARD_SECS, 60);
    assert!(should_clear("one two", "one two"));
    assert!(!should_clear("one two", "one two three"));
    assert!(!should_clear("kept", ""));
    assert!(should_clear("K7QM-4R2P-9HNX", "K7QM-4R2P-9HNX"));
    assert!(!should_clear("typed over", "K7QM-4R2P-9HNX"));
    covered.push("words");

    a.set_wrote(false);
    a.go_check();
    assert_eq!(a.screen, Screen::Words);
    a.set_wrote(true);
    a.go_check();
    assert_eq!(a.screen, Screen::Check);
    assert_eq!(a.asks(), [3, 7, 11]);
    for (i, n) in a.asks().into_iter().enumerate() {
        let real = words[n as usize - 1].as_str();
        assert!(a.choice_list(i).iter().any(|word| word == real));
    }
    a.pick(0, "nope");
    a.pick(1, &words[6]);
    a.pick(2, &words[10]);
    let wrong = a.confirm().unwrap_err();
    assert_eq!(
        wrong,
        "That's not word 3. Look at your paper and try again."
    );
    a.pick(0, &words[2]);
    a.pick(1, &words[6]);
    a.pick(2, &words[10]);
    covered.push("check");

    let (doc, pages) = library(&[
        ("trip", "Trip Budget\ndock fee 18\nmilk"),
        ("groceries", "Groceries\napples 4"),
        ("harbor", "Harbor notes\npier 12"),
    ]);
    a.adopt(LocalPages {
        doc,
        pages,
        locked: vec!["harbor".into()],
    });
    a.confirm().unwrap();
    assert_eq!(a.screen, Screen::PagesHere);
    assert_eq!(a.locked_title(), "Harbor notes");
    covered.push("pages-here");
    a.leave_locked();
    covered.push("locked");
    a.bring().unwrap();
    assert_eq!(a.screen, Screen::Ready);
    covered.push("ready");
    assert_rule(&a, &words);
    assert!(a.leader_ok());

    a.done();
    assert_eq!(a.chip(Instant::now()), Chip::Synced);
    a.testing_busy(Duration::from_millis(200));
    assert_ne!(a.chip(Instant::now()), Chip::Syncing);
    a.testing_busy(Duration::from_secs(2));
    assert_eq!(a.chip(Instant::now()), Chip::Syncing);
    a.sync_now().ok();
    covered.push("chip");

    a.show_home();
    let mine = a
        .devices()
        .into_iter()
        .find(|row| row.here)
        .expect("this computer");
    a.ask_remove(&mine.id);
    assert_eq!(a.screen, Screen::Home);
    a.begin_rename(&mine.id);
    assert_eq!(a.screen, Screen::Rename);
    a.save_rename("Harbor desk").unwrap();
    assert!(
        a.devices()
            .iter()
            .any(|row| row.here && row.name == "Harbor desk")
    );
    covered.push("devices");
    a.start_invite().unwrap();
    let first_code = a.invite_code().to_string();
    let picture = qr::round_trip(a.invite_link()).unwrap();
    let (_relay, _account, scanned) = qr::parse_join(&picture).unwrap();
    assert_eq!(scanned, first_code);
    covered.push("scan");

    let mut denied = Session::open(scratch("denied"), &origin);
    denied.choose_join();
    covered.push("join");
    denied.open_scan();
    denied.type_instead();
    assert_eq!(denied.screen, Screen::Join);
    unsafe {
        std::env::set_var("OMACUS_SYNC_CAMERA", "off");
    }
    assert!(!denied.camera_open());
    unsafe {
        std::env::remove_var("OMACUS_SYNC_CAMERA");
    }
    denied.submit_join(&first_code).unwrap();
    assert_eq!(denied.screen, Screen::Waiting);
    a.poll_requests().unwrap();
    a.open_approve();
    assert_eq!(a.screen, Screen::Approve);
    assert_eq!(a.numbers(), denied.numbers());
    assert_eq!(a.four_words().split_whitespace().count(), 4);
    a.deny().unwrap();
    let refused = denied.poll().unwrap_err();
    assert_eq!(refused, "That phone or computer said no.");

    a.start_invite().unwrap();
    let code = a.invite_code().to_string();
    let (_origin, _account, parsed) = qr::parse_join(a.invite_link()).unwrap();
    assert_eq!(parsed, code);
    let mut b = Session::open(scratch("b"), &origin);
    b.set_name("Harbor phone");
    let payload = qr::round_trip(a.invite_link()).unwrap();
    b.submit_scanned(&payload).unwrap();
    a.poll_requests().unwrap();
    a.open_approve();
    assert_eq!(a.numbers(), b.numbers());
    assert_eq!(a.four_words(), b.four_words());
    a.approve().unwrap();
    b.poll().unwrap();
    assert_eq!(b.screen, Screen::Match);
    b.accept_match().unwrap();
    assert!(b.page_text("trip").unwrap().contains("dock fee 18"));
    assert!(b.page_text("harbor").is_none());
    covered.push("approve");
    covered.push("waiting");

    a.note_edit("trip", "Trip Budget\nmilk");
    a.sync_now().unwrap();
    b.sync_now().unwrap();
    let gone = b.page_text("trip").unwrap();
    assert!(!gone.contains("dock fee 18"));
    assert!(gone.contains("milk"));

    a.note_edit("trip", "Trip Budget\nmilk\nalpha berth");
    b.note_edit("trip", "Trip Budget\nmilk\nbeta berth");
    a.sync_now().unwrap();
    b.sync_now().unwrap();
    a.sync_now().unwrap();
    let merged = a.page_text("trip").unwrap().to_string();
    assert!(merged.contains("alpha berth"), "{merged}");
    assert!(merged.contains("beta berth"), "{merged}");
    assert!(!merged.contains("dock fee 18"), "{merged}");
    covered.push("merge");
    assert_rule(&a, &words);

    let (cdoc, cpages) = library(&[("localtrip", "Trip Budget\nfrom this computer")]);
    let mut clash = Session::open(scratch("clash"), &origin);
    clash.adopt(LocalPages {
        doc: cdoc,
        pages: cpages,
        locked: Vec::new(),
    });
    a.start_invite().unwrap();
    clash.set_name("Other desk");
    clash.submit_join(a.invite_code()).unwrap();
    a.poll_requests().unwrap();
    a.open_approve();
    assert_eq!(a.numbers(), clash.numbers());
    a.approve().unwrap();
    clash.poll().unwrap();
    clash.accept_match().unwrap();
    assert_eq!(clash.screen, Screen::Clash);
    clash.keep_both().unwrap();
    let renamed = clash
        .pages()
        .iter()
        .any(|(_, text)| text.starts_with("Trip Budget, this computer"));
    assert!(renamed);
    covered.push("clash");

    a.ensure_folder("trips", "Trips", None);
    a.set_place("trip", Some("trips"));
    assert_eq!(a.place_of("trip").as_deref(), Some("trips"), "before");
    a.sync_now().unwrap();
    b.sync_now().unwrap();
    assert_eq!(a.place_of("trip").as_deref(), Some("trips"), "a");
    assert_eq!(b.place_of("trip").as_deref(), Some("trips"), "b");
    covered.push("folders");

    a.unlock_page("harbor");
    a.sync_now().unwrap();
    b.sync_now().unwrap();
    assert!(b.page_text("harbor").unwrap().contains("pier 12"));
    assert_rule(&a, &words);
    assert_rule(&b, &words);

    a.delete_page("groceries");
    a.sync_now().unwrap();
    b.sync_now().unwrap();
    assert!(b.page_text("groceries").is_none());
    a.restore_page("groceries", "Groceries\napples 4");
    a.sync_now().unwrap();
    b.sync_now().unwrap();
    assert!(b.page_text("groceries").unwrap().contains("apples 4"));
    covered.push("trash");

    let saved = origin.clone();
    a.set_origin("http://127.0.0.1:1");
    a.note_edit("trip", "Trip Budget\nmilk\nalpha berth\noffline marker 7");
    let offline = a.sync_now().unwrap_err();
    assert!(offline.starts_with("Offline"));
    assert_eq!(a.chip(Instant::now()), Chip::Offline);
    let outbox = fs::read(a.dir().join("sync").join("outbox.bin")).unwrap();
    let outbox_text = String::from_utf8_lossy(&outbox);
    assert!(!outbox_text.contains("offline marker 7"));
    assert!(!outbox_text.contains("pier 12"));
    assert!(
        fs::read_to_string(a.dir().join("pages").join("trip.txt"))
            .unwrap()
            .contains("offline marker 7")
    );
    covered.push("offline");
    a.set_origin(&saved);
    a.sync_now().unwrap();

    a.set_in_set("groceries", false);
    a.sync_now().unwrap();
    assert!(!a.in_set("groceries"));
    a.ask_set_move("groceries", "Groceries", true);
    a.confirm_move().unwrap();
    assert!(a.in_set("groceries"));
    covered.push("set");

    let bid = b
        .devices()
        .into_iter()
        .find(|row| row.here)
        .map(|row| row.id)
        .unwrap();
    a.remove_device(&bid).unwrap();
    b.sync_now().unwrap();
    assert_eq!(b.chip(Instant::now()), Chip::Paused);
    assert!(b.page_text("trip").is_some());

    let copy = copy_dir(a.dir(), &scratch("off"));
    let mut offline_off = Session::open(copy.clone(), &origin);
    offline_off.set_origin("http://127.0.0.1:1");
    let still = offline_off.turn_off().unwrap_err();
    assert!(still.contains("still on the list"));
    assert!(still.contains("Try again"));
    offline_off.turn_off_here();
    assert!(!keys::path(&copy).exists());
    assert!(offline_off.chip(Instant::now()) == Chip::Hidden);
    let _ = copy;

    let phrase = words.join(" ");
    let mut recovered = Session::open(scratch("recovered"), &origin);
    recovered.set_phrase(&phrase);
    recovered.recover().unwrap();
    assert!(recovered.page_text("trip").is_some());
    assert!(recovered.phrase_buf().is_empty());
    covered.push("recover");
    assert_rule(&recovered, &words);

    let settings = a.dir().join("settings.toml");
    fs::write(&settings, "theme = \"dark\"\n").unwrap();
    let zip = a.dir().join("sync-backup.zip");
    crate::backup::export_library(a.dir(), Some(&settings), &zip).unwrap();
    let packed = fs::read(&zip).unwrap();
    let packed_text = String::from_utf8_lossy(&packed);
    assert!(!packed_text.contains("OMACUS-DEVICE"));
    assert!(packed_text.contains("Trip Budget") || packed.windows(11).any(|w| w == b"Trip Budget"));
    covered.push("export");

    let other_dir = a.dir().to_path_buf();
    let mut other = Session::open(other_dir, &origin);
    assert!(a.leader_ok());
    assert!(!other.leader_ok());
    other.note_edit("scratch", "Scratch\nlocal only");
    assert!(
        fs::read_to_string(a.dir().join("pages").join("scratch.txt"))
            .unwrap()
            .contains("local only")
    );
    let sent = other.sent_bodies().len();
    other.poll().unwrap();
    assert_eq!(other.sent_bodies().len(), sent);
    covered.push("windows");

    let old = scratch("v100");
    fs::create_dir_all(old.join("pages")).unwrap();
    fs::write(old.join("pages").join("welcome.txt"), "Welcome\nstill here").unwrap();
    fs::write(
        old.join("library.automerge"),
        omacus_doc::Library::new().save(),
    )
    .unwrap();
    let legacy = Session::open(old.clone(), &origin);
    assert!(legacy.sent_bodies().is_empty());
    assert_eq!(legacy.screen, Screen::Closed);
    assert!(!old.join("sync").join("device.bin").exists());
    assert!(
        fs::read_to_string(old.join("pages").join("welcome.txt"))
            .unwrap()
            .contains("still here")
    );
    covered.push("update");

    a.ask_off();
    a.turn_off().unwrap();
    assert_eq!(a.screen, Screen::Closed);
    assert_eq!(a.chip(Instant::now()), Chip::Hidden);
    assert!(!keys::path(a.dir()).exists());
    assert!(a.page_text("trip").is_some());
    covered.push("off");

    covered.sort();
    covered.dedup();
    let mut expected = PARITY_ROWS.to_vec();
    expected.sort();
    assert_eq!(covered, expected, "missing parity rows");
}

fn library(pages: &[(&str, &str)]) -> (Vec<u8>, Vec<(String, String)>) {
    let mut lib = omacus_doc::Library::new();
    let mut owned = Vec::new();
    for (id, text) in pages {
        lib.reconcile(id, text, None);
        lib.note_written(id, text);
        owned.push(((*id).to_string(), (*text).to_string()));
    }
    (lib.save(), owned)
}

fn start_relay() -> (String, String) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = std::env::temp_dir().join(format!("omacus-desk-relay-{port}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let mut store = omacus_relay::store::Store::open_dir(&dir);
    let secret = b"desktop-sync-test!";
    store.set_access_secret(secret);
    let code = omacus_relay::access::mint(secret, 4, b"desk01").unwrap();
    let store = std::sync::Arc::new(std::sync::Mutex::new(store));
    std::thread::spawn(move || {
        omacus_relay::http::serve(listener, dir.join("www"), dir.join("site"), store);
    });
    (format!("http://127.0.0.1:{port}"), code)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "omacus-desk-{name}-{}-{}",
        std::process::id(),
        name.len()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_dir(from: &Path, to: &Path) -> PathBuf {
    let _ = fs::remove_dir_all(to);
    copy_tree(from, to);
    to.to_path_buf()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn assert_rule(session: &Session, words: &[String; 12]) {
    let sent = session.sent_bodies().join("\n");
    for needle in [
        "ferry = 40",
        "milk $3",
        "pier 12",
        "dock fee 18",
        "offline marker 7",
        "OMACUS-DEVICE",
        "alpha berth",
        "beta berth",
    ] {
        assert!(!sent.contains(needle), "{needle} left this computer");
    }
    for body in session.sent_bodies() {
        assert!(
            word_leak(body, words).is_none(),
            "a recovery word left this computer"
        );
    }
    let book = fs::read_to_string(session.dir().join("sync").join("book.json")).unwrap_or_default();
    assert!(!book.contains("OMACUS-DEVICE"));
    assert!(!book.contains("pier 12"));
    assert!(!book.contains("ferry = 40"));
    assert!(
        word_leak(&book, words).is_none(),
        "a recovery word was written into the sync book"
    );
    if let Some(kind) = keys::kind(session.dir()) {
        let bytes = fs::read(keys::path(session.dir())).unwrap_or_default();
        assert_ne!(kind, "file", "the device key was stored in a plain file");
        assert!(keys::is_private(session.dir()));
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("OMACUS-DEVICE"));
    }
    walk_clean(session.dir(), words);
}

fn walk_clean(dir: &Path, words: &[String; 12]) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_clean(&path, words);
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("device.bin") {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("txt") {
            let text = fs::read_to_string(&path).unwrap_or_default();
            assert!(!text.contains("OMACUS-DEVICE"));
            // A page can say "this computer". That is the title, not the phrase.
            let phrase = words.join(" ");
            assert!(
                !text.contains(&phrase),
                "{} has the recovery phrase",
                path.display()
            );
            continue;
        }
        let bytes = fs::read(&path).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("OMACUS-DEVICE"), "{}", path.display());
        // Sealed bytes and automerge files are binary. A word can show up in
        // that noise, or as a field name such as "account", without being sent.
        if bytes.contains(&0) || path.extension().and_then(|ext| ext.to_str()) == Some("bin") {
            continue;
        }
        assert!(
            word_leak(&text, words).is_none(),
            "{} contains a recovery word",
            path.display()
        );
    }
}

/// The phrase, and each word where a person could read it.
///
/// Field names are ordinary protocol words (`account`, `state`). Sealed
/// hex and base64 are not the words either, so those are skipped. A base64
/// value is decoded first, which still catches the words if they were packed
/// into a field instead of sealed.
fn word_leak(raw: &str, words: &[String; 12]) -> Option<String> {
    let phrase = words.join(" ");
    if !phrase.is_empty() && raw.contains(&phrase) {
        return Some(phrase);
    }
    let readable = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => {
            let mut out = String::new();
            collect_readable(&value, &mut out);
            out
        }
        Err(_) => strip_opaque(raw),
    };
    words
        .iter()
        .find(|word| readable.contains(word.as_str()))
        .cloned()
}

fn collect_readable(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::String(text) => append_readable(text, out),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_readable(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_readable(item, out);
            }
        }
        _ => {}
    }
}

fn append_readable(text: &str, out: &mut String) {
    match opaque(text) {
        Some(Opaque::Hex) => {}
        Some(Opaque::Base64) => {
            if let Some(clear) = decode_base64(text) {
                out.push('\n');
                out.push_str(&clear);
            }
        }
        None => {
            out.push('\n');
            out.push_str(text);
        }
    }
}

enum Opaque {
    Hex,
    Base64,
}

fn opaque(text: &str) -> Option<Opaque> {
    if text.len() >= 16
        && text.len().is_multiple_of(2)
        && text.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Some(Opaque::Hex);
    }
    // Eight characters is the encoding of a short word. Longer sealed fields
    // use the same alphabet, with a digit or a capital so a plain word stays.
    if text.len() < 8 {
        return None;
    }
    let base64 = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_'));
    if base64 && text.chars().any(|c| !c.is_ascii_lowercase()) {
        Some(Opaque::Base64)
    } else {
        None
    }
}

fn decode_base64(text: &str) -> Option<String> {
    use base64::Engine;
    let engines = [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ];
    engines.into_iter().find_map(|engine| {
        engine
            .decode(text)
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    })
}

fn strip_opaque(text: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_') {
            run.push(c);
        } else {
            flush_run(&mut run, &mut out);
            out.push(c);
        }
    }
    flush_run(&mut run, &mut out);
    out
}

fn flush_run(run: &mut String, out: &mut String) {
    if !run.is_empty() {
        append_readable(run, out);
        run.clear();
    }
}

#[test]
fn a_field_name_is_not_a_recovery_word_leaving() {
    let mut words: [String; 12] = std::array::from_fn(|n| format!("madeup{n}"));
    words[0] = "account".into();
    words[1] = "state".into();
    words[2] = "secret".into();
    let body =
        r#"{"account":"0123456789abcdef0123456789abcdef","state":"pending","name":"Harbor desk"}"#;
    assert!(word_leak(body, &words).is_none());
    let leaked = r#"{"name":"account"}"#;
    assert_eq!(word_leak(leaked, &words).as_deref(), Some("account"));
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode("secret");
    let packed = format!(r#"{{"note":"{encoded}"}}"#);
    assert_eq!(word_leak(&packed, &words).as_deref(), Some("secret"));
    let phrase = words.join(" ");
    let pasted = format!(r#"{{"note":"{phrase}"}}"#);
    assert!(word_leak(&pasted, &words).is_some());
}

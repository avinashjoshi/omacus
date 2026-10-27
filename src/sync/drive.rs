//! A local driver for the joined-device proof. It speaks the same session the
//! window uses. Recovery words, invite codes, and passphrases stay in files
//! the caller already chose. Nothing here prints them.

use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use zeroize::Zeroize;

use super::session::{LocalPages, Screen, Session};

pub fn e2e_drive() -> i32 {
    let dir = std::env::var("OMACUS_E2E_DIR").unwrap_or_else(|_| {
        std::env::temp_dir()
            .join("omacus-e2e-sync")
            .display()
            .to_string()
    });
    let origin =
        std::env::var("OMACUS_SYNC_URL").unwrap_or_else(|_| "http://127.0.0.1:8798".into());
    let dir = PathBuf::from(dir);
    if let Err(error) = fs::create_dir_all(&dir) {
        let message = error.to_string();
        return fail(&message);
    }
    let mut session = Session::open(dir.clone(), &origin);
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            return 1;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(cmd) = parts.next() else {
            continue;
        };
        let result = match cmd {
            "setup" => setup(&mut session, &dir),
            "invite" => invite(&mut session),
            "join" => join(&mut session),
            "approve" => approve(&mut session),
            "accept" => accept(&mut session),
            "rename" => {
                let id = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("");
                rename(&mut session, id, Path::new(path))
            }
            "await" => {
                let id = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("");
                let ms = parts
                    .next()
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(8000);
                await_text(&mut session, id, Path::new(path), ms)
            }
            "status" => Ok(status(&session)),
            "quit" => {
                emit(r#"{"ok":true,"cmd":"quit"}"#);
                return 0;
            }
            _ => Err("unknown command".into()),
        };
        match result {
            Ok(extra) => emit(&format!(r#"{{"ok":true,"cmd":"{cmd}","detail":{extra}}}"#)),
            Err(error) => emit(&format!(
                r#"{{"ok":false,"cmd":"{cmd}","error":{}}}"#,
                serde_json::to_string(&safe_error(&error))
                    .unwrap_or_else(|_| "\"sync failed\"".into())
            )),
        }
    }
    0
}

fn setup(session: &mut Session, dir: &Path) -> Result<String, String> {
    let access = read_secret("OMACUS_E2E_ACCESS_FILE")?;
    let mut pass = read_secret("OMACUS_E2E_PASS_FILE")?;
    session.choose_passphrase();
    session.pass_buf().clone_from(&pass);
    session.pass_again().clone_from(&pass);
    pass.zeroize();
    session.submit_pass()?;
    session.set_name("Harbor desk");
    session.choose_first();
    if session.screen != Screen::Start {
        return Err(session.error.clone());
    }
    session.access_buf().clone_from(&access);
    session.continue_access()?;
    let mut words = session
        .words_grid()
        .ok_or_else(|| "Those words are not recovery words.".to_string())?
        .clone();
    session.set_wrote(true);
    session.go_check();
    for (i, n) in session.asks().into_iter().enumerate() {
        let word = words.get(n as usize - 1).cloned().unwrap_or_default();
        session.pick(i, &word);
    }
    for word in &mut words {
        word.zeroize();
    }
    write_demo_files(dir)?;
    session.adopt(LocalPages {
        doc: Vec::new(),
        pages: demo_pages(),
        locked: Vec::new(),
    });
    session.confirm()?;
    if session.screen == Screen::PagesHere {
        session.bring()?;
    }
    session.note_structural(&demo_structural());
    session.done();
    wait_until(session, 20_000, |session| {
        session.socket_live() && !session.is_dirty() && session.error.is_empty()
    })?;
    Ok(status(session))
}

fn invite(session: &mut Session) -> Result<String, String> {
    session.show_home();
    session.start_invite()?;
    let path =
        std::env::var("OMACUS_E2E_INVITE_FILE").map_err(|_| "missing invite file".to_string())?;
    write_secret(Path::new(&path), session.invite_code())?;
    session.done();
    Ok("{}".into())
}

fn emit(line: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn join(session: &mut Session) -> Result<String, String> {
    let mut code = read_secret("OMACUS_E2E_JOIN_FILE")?;
    session.set_name("Harbor desk");
    session.submit_join(&code)?;
    code.zeroize();
    Ok(status(session))
}

fn approve(session: &mut Session) -> Result<String, String> {
    wait_until(session, 30_000, |session| {
        let _ = session.poll_requests();
        session.has_pending()
    })?;
    session.open_approve();
    session.approve()?;
    Ok(status(session))
}

fn accept(session: &mut Session) -> Result<String, String> {
    wait_until(session, 30_000, |session| session.screen == Screen::Match)?;
    session.accept_match()?;
    let _ = wait_until(session, 15_000, |session| session.socket_live());
    Ok(status(session))
}

fn rename(session: &mut Session, id: &str, path: &Path) -> Result<String, String> {
    if id.is_empty() {
        return Err("missing page".into());
    }
    let text = fs::read_to_string(path).map_err(|_| "missing page text".to_string())?;
    let started = millis();
    session.note_edit(id, &text);
    thread::sleep(Duration::from_millis(super::live::PUSH_DEBOUNCE_MS));
    wait_until(session, 15_000, |session| {
        !session.is_dirty() && session.error.is_empty()
    })?;
    Ok(format!(r#"{{"ms":{}}}"#, millis().saturating_sub(started)))
}

fn await_text(session: &mut Session, id: &str, path: &Path, ms: u64) -> Result<String, String> {
    let needle = fs::read_to_string(path).map_err(|_| "missing needle".to_string())?;
    let needle = needle.trim().to_string();
    let started = millis();
    wait_until(session, ms, |session| {
        session
            .page_text(id)
            .is_some_and(|text| text.contains(needle.as_str()))
    })?;
    Ok(format!(r#"{{"ms":{}}}"#, millis().saturating_sub(started)))
}

fn wait_until(
    session: &mut Session,
    ms: u64,
    ready: impl Fn(&mut Session) -> bool,
) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        if ready(session) {
            return Ok(());
        }
        let _ = session.poll();
        if ready(session) {
            return Ok(());
        }
        if !session.error.is_empty()
            && session.screen != Screen::Waiting
            && session.screen != Screen::Approve
        {
            // A live socket can report a miss and recover. Keep going until the deadline
            // unless the join itself was refused.
            if session.error == "That phone or computer said no."
                || session.error.starts_with("That code expired")
            {
                return Err(session.error.clone());
            }
        }
        let gap = super::live::poll_gap_ms(
            session.screen == Screen::Waiting,
            session.is_dirty(),
            session.socket_live(),
        );
        let left = ms.saturating_sub(start.elapsed().as_millis() as u64);
        if left == 0 {
            break;
        }
        thread::sleep(Duration::from_millis(gap.min(left)));
    }
    if ready(session) {
        Ok(())
    } else {
        Err("timed out".into())
    }
}

fn status(session: &Session) -> String {
    let headings: Vec<String> = session
        .status_pages()
        .into_iter()
        .map(|(id, text)| {
            let heading = text
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("")
                .trim_start_matches(['#', '/', ' '])
                .trim()
                .to_string();
            format!("{id}:{heading}")
        })
        .collect();
    serde_json::json!({
        "screen": screen_name(session.screen),
        "chip": session.chip(Instant::now()).label().unwrap_or(""),
        "live": session.socket_live(),
        "pages": headings,
        "error": safe_error(&session.error),
    })
    .to_string()
}

fn screen_name(screen: Screen) -> &'static str {
    match screen {
        Screen::Waiting => "waiting",
        Screen::Match => "match",
        Screen::Approve => "approve",
        Screen::Closed => "closed",
        Screen::Home => "home",
        Screen::Ready => "ready",
        Screen::PassEnter => "pass",
        Screen::PassSet => "pass",
        _ => "other",
    }
}

fn safe_error(error: &str) -> String {
    if error.is_empty() {
        return String::new();
    }
    const SAFE: &[&str] = &[
        "That invite code didn't work. Check it and try again.",
        "Enter a passphrase.",
        "Use at least 8 characters.",
        "Type the same passphrase again.",
        "That passphrase didn't work. Try again.",
        "Those words are not recovery words.",
        "That's not word 3. Look at your paper and try again.",
        "That's not word 7. Look at your paper and try again.",
        "That's not word 11. Look at your paper and try again.",
        "That phone or computer said no.",
        "That code expired. Ask for a new one on the other phone or computer.",
        "That code didn't work. Check it on the other phone or computer.",
        "Sync is off.",
        "timed out",
        "missing secret file",
        "missing invite file",
        "missing page",
        "missing page text",
        "missing needle",
        "the live connection did not open",
    ];
    if SAFE.contains(&error) || error.starts_with("That's not word ") {
        error.to_string()
    } else {
        "sync failed".into()
    }
}

fn fail(error: &str) -> i32 {
    emit(&format!(
        r#"{{"ok":false,"error":{}}}"#,
        serde_json::to_string(&safe_error(error)).unwrap_or_else(|_| "\"sync failed\"".into())
    ));
    1
}

fn read_secret(key: &str) -> Result<String, String> {
    let path = std::env::var(key).map_err(|_| "missing secret file".to_string())?;
    let text = fs::read_to_string(path).map_err(|_| "missing secret file".to_string())?;
    let text = text.trim().to_string();
    if text.is_empty() {
        Err("missing secret file".into())
    } else {
        Ok(text)
    }
}

fn write_secret(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "missing invite file".to_string())?;
    }
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|_| "missing invite file".to_string())?;
        file.write_all(text.as_bytes())
            .map_err(|_| "missing invite file".to_string())?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, text).map_err(|_| "missing invite file".to_string())?;
    }
    Ok(())
}

fn demo_pages() -> Vec<(String, String)> {
    ["oats", "ferry", "bread", "cake"]
        .into_iter()
        .map(|id| {
            let title = match id {
                "oats" => "Oats",
                "ferry" => "Ferry",
                "bread" => "Bread",
                _ => "Cake",
            };
            (id.to_string(), format!("# {title}\nbowl 2\n"))
        })
        .collect()
}

fn demo_structural() -> String {
    serde_json::json!({
        "sidebar": [
            {"id": "pantry", "kind": "folder", "name": "Pantry", "parent": "", "index": 0, "collapsed": false},
            {"id": "oats", "kind": "page", "name": "", "parent": "pantry", "index": 0},
            {"id": "ferry", "kind": "page", "name": "", "parent": "pantry", "index": 1},
            {"id": "bread", "kind": "page", "name": "", "parent": "pantry", "index": 2},
            {"id": "cake", "kind": "page", "name": "", "parent": "pantry", "index": 3}
        ],
        "handles": [{"id": "ferry", "handle": "pier"}],
        "trash": [{"id": "kept", "text": "# Kept\nold loaf\n", "handle": ""}],
        "prefs": {
            "theme": "dusk",
            "hideAnswers": true,
            "lockMinutes": 15,
            "replaceExports": false,
            "checkForUpdates": true
        }
    })
    .to_string()
}

fn write_demo_files(dir: &Path) -> Result<(), String> {
    let pages = dir.join("pages");
    let trash = dir.join("trash");
    fs::create_dir_all(&pages).map_err(|error| error.to_string())?;
    fs::create_dir_all(&trash).map_err(|error| error.to_string())?;
    for (id, text) in demo_pages() {
        fs::write(pages.join(format!("{id}.txt")), text).map_err(|error| error.to_string())?;
    }
    fs::write(trash.join("kept.txt"), "# Kept\nold loaf\n").map_err(|error| error.to_string())?;
    let state = serde_json::json!({
        "order": ["oats", "ferry", "bread", "cake"],
        "active": "oats",
        "cursors": {},
        "sidebar": true,
        "sidebar_width": 240.0,
        "tree": [{
            "type": "folder",
            "id": "pantry",
            "name": "Pantry",
            "collapsed": false,
            "children": [
                {"type": "page", "id": "oats"},
                {"type": "page", "id": "ferry"},
                {"type": "page", "id": "bread"},
                {"type": "page", "id": "cake"}
            ]
        }],
        "handles": {"ferry": "pier"},
        "seeded": true
    });
    fs::write(
        dir.join("state.json"),
        serde_json::to_vec_pretty(&state).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

//! Sync for this window.
//!
//! Plain files stay the working copy. A sealed record is what leaves. The
//! server never receives the vault key, the device keys, the recovery words,
//! or the page text.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zeroize::Zeroize;

use omacus_engine::sync_keys::{DeviceLabel, PageMember, PageSet, SyncMeta};
use omacus_engine::sync_objects::{Incoming, Outgoing, Slot};
use omacus_sync_web::desk::{self, Desk};
use omacus_sync_web::invite;

use super::keys;
use super::qr;
use super::wire::{self, Client};

pub const CLIPBOARD_SECS: u64 = 60;
pub const PARTIAL_NOTE: &str = "Some changes made after a phone or computer was removed may be missing. They were locked to phones and computers that are no longer linked. Pages already here were combined with the ones these words could open. Nothing was deleted.";

pub const PARITY_ROWS: &[&str] = &[
    "explainer",
    "choice",
    "start",
    "words",
    "check",
    "ready",
    "join",
    "scan",
    "approve",
    "waiting",
    "devices",
    "off",
    "recover",
    "chip",
    "merge",
    "pages-here",
    "clash",
    "locked",
    "folders",
    "trash",
    "export",
    "offline",
    "set",
    "invite-only",
    "windows",
    "update",
];

pub const INTRO: &[&str] = &[
    "Pages are encrypted on this computer before they leave. The server keeps them, and it cannot read them.",
    "You get 12 recovery words. They are the only way back in if you lose every phone and computer. Write them down. Omacus does not keep a copy.",
    "You link another phone or computer with a code. After that, your pages stay in step, even when one of them is offline.",
    "Pages already in this window can come with you. They stay as files on this computer.",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Closed,
    Intro,
    Choice,
    Start,
    Words,
    Check,
    Ready,
    Home,
    Invite,
    Join,
    Scan,
    Waiting,
    Match,
    Approve,
    Recover,
    OffAsk,
    OffFailed,
    RemoveAsk,
    Removed,
    PagesHere,
    Locked,
    Clash,
    Rename,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Hidden,
    Synced,
    Syncing,
    Offline,
    Paused,
    NotSaved,
    Waiting,
}

impl Chip {
    pub fn label(self) -> Option<&'static str> {
        match self {
            Chip::Hidden => None,
            Chip::Synced => Some("Synced"),
            Chip::Syncing => Some("Syncing"),
            Chip::Offline => Some("Offline"),
            Chip::Paused => Some("Sync paused"),
            Chip::NotSaved => Some("Not saved"),
            Chip::Waiting => Some("Device waiting"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Clash {
    pub title: String,
    pub local_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct LocalPages {
    pub doc: Vec<u8>,
    pub pages: Vec<(String, String)>,
    pub locked: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub here: bool,
}

pub struct PrintJob {
    #[cfg_attr(not(test), allow(dead_code))]
    pub path: Option<PathBuf>,
}

pub struct Session {
    dir: PathBuf,
    origin: String,
    page_origin: String,
    client: Client,
    desk: Option<Desk>,
    pub screen: Screen,
    words: Option<[String; 12]>,
    wrote: bool,
    asks: [u8; 3],
    picks: [Option<String>; 3],
    choices: Vec<Vec<String>>,
    pub self_name: String,
    self_rev: u64,
    vault_name: String,
    name_rev: u64,
    access: String,
    join_code: String,
    join_id: String,
    join_proof: String,
    phrase: String,
    account: String,
    revision: u64,
    slots: Vec<Slot>,
    membership: Vec<PageMember>,
    doc: Vec<u8>,
    pages: Vec<(String, String)>,
    locked: Vec<String>,
    bases: Vec<(String, String)>,
    pub error: String,
    pending_device: Vec<u8>,
    pending_id: String,
    pending_name: String,
    invite_code: String,
    invite_link: String,
    on: bool,
    member: bool,
    removed: bool,
    dirty: bool,
    offline: bool,
    busy: bool,
    saving_since: Option<Instant>,
    leader: Option<File>,
    pub notice: String,
    clash: Option<Clash>,
    pre_ids: Vec<String>,
    rename_id: String,
    rename_text: String,
    labels: Vec<BookDevice>,
    move_ask: Option<MoveAsk>,
    announced: Option<MoveAsk>,
    lock_settled: bool,
    pub covered: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct MoveAsk {
    pub id: String,
    pub title: String,
    pub inside: bool,
}

impl Session {
    pub fn open(dir: PathBuf, origin: &str) -> Self {
        let mut session = Self::blank(dir, origin);
        if let Some(plain) = keys::load(&session.dir).ok().flatten()
            && let Some((device, slots)) = split_blob(&plain)
            && let Ok(desk) = Desk::import_local(&device)
        {
            session.account = desk.account_hex().unwrap_or_default();
            session.desk = Some(desk);
            session.slots = slots;
            session.on = true;
            session.member = true;
            session.leader = lock_leader(&session.dir);
        }
        session.load_book();
        session.load_disk();
        if session.removed {
            session.member = false;
        }
        session
    }

    fn blank(dir: PathBuf, origin: &str) -> Self {
        let origin = origin.trim_end_matches('/').to_string();
        Self {
            client: Client::new(&origin),
            dir,
            origin,
            page_origin: "https://app.omacus.com".to_string(),
            desk: None,
            screen: Screen::Closed,
            words: None,
            wrote: false,
            asks: desk::CONFIRM_AT,
            picks: [None, None, None],
            choices: Vec::new(),
            self_name: guess_name(),
            self_rev: 1,
            vault_name: "My pages".to_string(),
            name_rev: 1,
            access: String::new(),
            join_code: String::new(),
            join_id: String::new(),
            join_proof: String::new(),
            phrase: String::new(),
            account: String::new(),
            revision: 0,
            slots: Vec::new(),
            membership: Vec::new(),
            doc: Vec::new(),
            pages: Vec::new(),
            locked: Vec::new(),
            bases: Vec::new(),
            error: String::new(),
            pending_device: Vec::new(),
            pending_id: String::new(),
            pending_name: String::new(),
            invite_code: String::new(),
            invite_link: String::new(),
            on: false,
            member: false,
            removed: false,
            dirty: false,
            offline: false,
            busy: false,
            saving_since: None,
            leader: None,
            notice: String::new(),
            clash: None,
            pre_ids: Vec::new(),
            rename_id: String::new(),
            rename_text: String::new(),
            labels: Vec::new(),
            move_ask: None,
            announced: None,
            lock_settled: false,
            covered: Vec::new(),
        }
    }

    pub fn preview(&mut self, kind: &str) {
        if self.pages.is_empty() {
            self.pages = vec![
                (
                    "trip".into(),
                    "Trip Budget\nFlights $640\nHotel, 4 nights $820".into(),
                ),
                ("groceries".into(), "Groceries\nmilk 3".into()),
            ];
        }
        self.vault_name = "My pages".into();
        match kind {
            "explainer" | "intro" => self.screen = Screen::Intro,
            "pages" => self.screen = Screen::PagesHere,
            "locked" => {
                if self.locked.is_empty() {
                    self.locked = vec!["harbor".into()];
                }
                self.screen = Screen::Locked;
            }
            "clash" => {
                self.clash = Some(Clash {
                    title: "Trip Budget".into(),
                    local_id: "trip".into(),
                });
                self.screen = Screen::Clash;
            }
            "waiting" => {
                if !self.pages.iter().any(|(id, _)| id == "scratch") {
                    self.pages.push(("scratch".into(), "Scratch\n".into()));
                }
                self.on = true;
                self.member = true;
                self.pending_id = "wait".into();
                self.pending_name = "Phone".into();
                self.screen = Screen::Closed;
                self.set_member_flag("trip", true);
                self.set_member_flag("groceries", true);
                self.set_member_flag("scratch", false);
            }
            "scan" => {
                self.on = true;
                self.member = true;
                self.screen = Screen::Scan;
            }
            "choice" => self.screen = Screen::Choice,
            "start" => self.screen = Screen::Start,
            "words" => {
                self.words = Some(std::array::from_fn(|i| format!("word{i}")));
                self.screen = Screen::Words;
            }
            "check" => self.screen = Screen::Check,
            "ready" | "home" => {
                self.on = true;
                self.member = true;
                self.screen = Screen::Home;
            }
            "join" => self.screen = Screen::Join,
            "approve" => {
                self.pending_name = "Harbor phone".into();
                self.screen = Screen::Approve;
            }
            "recover" => self.screen = Screen::Recover,
            "off" => self.screen = Screen::OffAsk,
            _ => self.screen = Screen::Intro,
        }
    }

    #[cfg(test)]
    pub fn set_origin(&mut self, origin: &str) {
        self.origin = origin.trim_end_matches('/').to_string();
        self.client.set_origin(&self.origin);
    }

    pub fn adopt(&mut self, local: LocalPages) {
        if local.doc.is_empty() {
            self.doc = omacus_doc::Library::new().save();
        } else {
            self.doc = local.doc;
        }
        self.pages = local.pages;
        self.locked = local.locked;
        self.sync_membership_lines();
    }

    pub fn open_intro(&mut self) {
        self.error.clear();
        self.screen = Screen::Intro;
        self.mark("explainer");
    }

    pub fn not_now(&mut self) {
        self.screen = Screen::Closed;
        self.error.clear();
    }

    pub fn begin_setup(&mut self) {
        self.screen = Screen::Choice;
        self.mark("choice");
    }

    pub fn choose_first(&mut self) {
        self.error.clear();
        match Desk::start_new() {
            Ok(desk) => {
                self.words = desk.words().map(|words| words.map(str::to_string));
                self.desk = Some(desk);
                self.screen = Screen::Start;
                self.mark("start");
            }
            Err(error) => self.error = error.message().to_string(),
        }
    }

    pub fn choose_join(&mut self) {
        self.error.clear();
        self.screen = Screen::Join;
        self.mark("join");
    }

    pub fn choose_recover(&mut self) {
        self.error.clear();
        self.screen = Screen::Recover;
        self.mark("recover");
    }

    pub fn set_name(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || name.chars().any(|ch| ch.is_control()) {
            return;
        }
        if name != self.self_name {
            self.self_name = name.to_string();
            self.self_rev = self.self_rev.saturating_add(1);
        }
    }

    pub fn set_vault_name(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if name != self.vault_name {
            self.vault_name = name.to_string();
            self.name_rev = self.name_rev.saturating_add(1);
            self.dirty = true;
        }
    }

    #[cfg(test)]
    pub fn set_access(&mut self, code: &str) {
        self.access = code.trim().to_string();
    }

    pub fn continue_access(&mut self) -> Result<(), String> {
        self.error.clear();
        self.mark("invite-only");
        match self
            .client
            .open("POST", "/v1/access", Some(json!({ "code": self.access })))
        {
            Ok(_) => {
                self.screen = Screen::Words;
                self.mark("words");
                Ok(())
            }
            Err(error) => {
                self.error = if error.to_lowercase().contains("invite") {
                    "That invite code didn't work. Check it and try again.".into()
                } else {
                    error
                };
                Err(self.error.clone())
            }
        }
    }

    pub fn set_wrote(&mut self, wrote: bool) {
        self.wrote = wrote;
    }

    pub fn go_check(&mut self) {
        if !self.wrote {
            return;
        }
        let Some(words) = self.words.clone() else {
            return;
        };
        self.choices = self.asks.map(|n| choices_for(&words, n)).to_vec();
        self.picks = [None, None, None];
        self.screen = Screen::Check;
        self.mark("check");
    }

    pub fn pick(&mut self, which: usize, word: &str) {
        if which < 3 {
            self.picks[which] = Some(word.to_string());
        }
    }

    pub fn confirm(&mut self) -> Result<(), String> {
        self.error.clear();
        let Some(words) = self.words.clone() else {
            return Err("Those words are not recovery words.".into());
        };
        for (i, n) in self.asks.iter().enumerate() {
            let want = words.get(*n as usize - 1).map(String::as_str).unwrap_or("");
            let got = self.picks[i].as_deref().unwrap_or("");
            if got != want {
                self.error = format!("That's not word {n}. Look at your paper and try again.");
                return Err(self.error.clone());
            }
        }
        let owned: Vec<(u8, String)> = self
            .asks
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, self.picks[i].clone().unwrap_or_default()))
            .collect();
        let refs: Vec<(u8, &str)> = owned.iter().map(|(n, word)| (*n, word.as_str())).collect();
        let desk = self.desk.as_mut().ok_or("Sync is off.")?;
        // The record from this call seals a sample page. It is not uploaded.
        desk.confirm_saved(&refs)
            .map_err(|e| e.message().to_string())?;
        self.account = self
            .desk
            .as_ref()
            .and_then(Desk::account_hex)
            .unwrap_or_default();
        self.wipe_words();
        self.leader = lock_leader(&self.dir);
        if self.pages.iter().any(|(id, _)| !self.locked.contains(id)) {
            self.screen = Screen::PagesHere;
            self.mark("pages-here");
        } else {
            self.publish_first()?;
            self.screen = Screen::Ready;
            self.mark("ready");
        }
        Ok(())
    }

    pub fn bring(&mut self) -> Result<(), String> {
        for (id, _) in self.pages.clone() {
            let inside = !self.locked.contains(&id);
            self.set_member_flag(&id, inside);
        }
        self.publish_first()?;
        self.screen = Screen::Ready;
        self.mark("ready");
        Ok(())
    }

    pub fn leave_files(&mut self) -> Result<(), String> {
        for (id, _) in self.pages.clone() {
            self.set_member_flag(&id, false);
        }
        self.publish_first()?;
        self.screen = Screen::Ready;
        self.mark("ready");
        Ok(())
    }

    pub fn done(&mut self) {
        self.screen = Screen::Closed;
    }

    pub fn show_home(&mut self) {
        if self.on {
            self.screen = Screen::Home;
            self.mark("devices");
        }
    }

    pub fn copy_words(&mut self) -> Result<String, String> {
        let Some(words) = &self.words else {
            return Err("Copy didn't work. Write the words down.".into());
        };
        let text = numbered(words);
        copy_secret(&text, "Copy didn't work. Write the words down.")
    }

    pub fn copy_invite(&mut self) -> Result<String, String> {
        let code = self.invite_code.clone();
        if code.is_empty() {
            return Err("Copy didn't work. Type the code on the other device.".into());
        }
        copy_secret(
            &code,
            "Copy didn't work. Type the code on the other device.",
        )
    }

    pub fn print_words(&mut self) -> Result<PrintJob, String> {
        let Some(words) = &self.words else {
            return Err("Print didn't work. Write the words down.".into());
        };
        let text = numbered(words);
        deliver_print(&text)
    }

    pub fn words_grid(&self) -> Option<&[String; 12]> {
        self.words.as_ref()
    }

    pub fn asks(&self) -> [u8; 3] {
        self.asks
    }

    pub fn choice_list(&self, which: usize) -> &[String] {
        self.choices.get(which).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn start_invite(&mut self) -> Result<(), String> {
        self.error.clear();
        if !self.leader_ok() {
            return Err("This window is watching. The other window approves.".into());
        }
        let mut secret = random_bytes::<8>()?;
        secret = invite::canonical(&secret);
        let code = invite::encode_code(&secret);
        let hash = invite::hash_hex(&secret);
        let account = self.account.clone();
        let path = format!("/v1/mailboxes/{account}/invites");
        let desk = self.desk.as_ref().ok_or("Sync is off.")?;
        self.client
            .signed(desk, "POST", &path, Some(json!({ "hash": hash })), &[])?;
        let link = desk::join_link(&self.page_origin, &self.origin, &account, &code)
            .map_err(|e| e.message().to_string())?;
        secret.zeroize();
        self.invite_code = code;
        self.invite_link = link;
        self.screen = Screen::Invite;
        self.mark("scan");
        Ok(())
    }

    pub fn invite_code(&self) -> &str {
        &self.invite_code
    }

    #[cfg(test)]
    pub fn invite_link(&self) -> &str {
        &self.invite_link
    }

    pub fn invite_modules(&self) -> Result<Vec<bool>, String> {
        qr::modules(&self.invite_link)
    }

    pub fn open_scan(&mut self) {
        self.screen = Screen::Scan;
        self.mark("scan");
    }

    pub fn type_instead(&mut self) {
        self.screen = Screen::Join;
    }

    #[cfg(test)]
    pub fn camera_open(&self) -> bool {
        if std::env::var("OMACUS_SYNC_CAMERA").ok().as_deref() == Some("off") {
            return false;
        }
        #[cfg(target_os = "linux")]
        {
            Path::new("/dev/video0").exists()
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    pub fn submit_join(&mut self, code: &str) -> Result<(), String> {
        self.error.clear();
        let code = code.trim();
        if code.is_empty() {
            self.error = "That code didn't work. Check it on the other phone or computer.".into();
            return Err(self.error.clone());
        }
        let desk = Desk::start_join().map_err(|e| e.message().to_string())?;
        let device = desk.public_device();
        let proof = invite::decode_code(code).ok_or_else(|| {
            "That code didn't work. Check it on the other phone or computer.".to_string()
        })?;
        let mac = invite::proof(&proof, &device);
        let body = json!({
            "code": code,
            "device": wire::b64(&device),
            "name": self.self_name,
            "proof": wire::b64(&mac),
        });
        let created = match self.client.open("POST", "/v1/join", Some(body)) {
            Ok(reply) => reply,
            Err(_) => {
                self.error =
                    "That code didn't work. Check it on the other phone or computer.".into();
                return Err(self.error.clone());
            }
        };
        self.account = created
            .json
            .get("account")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.join_id = created
            .json
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.join_proof = wire::b64(&mac);
        self.join_code = desk.pairing_code();
        self.desk = Some(desk);
        self.screen = Screen::Waiting;
        self.mark("waiting");
        Ok(())
    }

    #[cfg(test)]
    pub fn submit_scanned(&mut self, payload: &str) -> Result<(), String> {
        let (_origin, _account, code) = qr::parse_join(payload).ok_or_else(|| {
            "That code didn't work. Check it on the other phone or computer.".to_string()
        })?;
        self.submit_join(&code)
    }

    pub fn sync_now(&mut self) -> Result<(), String> {
        if !self.on {
            return Ok(());
        }
        if self.dirty {
            self.push()
        } else {
            self.pull_if_newer()
        }
    }

    pub fn poll(&mut self) -> Result<(), String> {
        if self.screen == Screen::Waiting && !self.join_id.is_empty() {
            return self.poll_join();
        }
        if self.on && self.leader_ok() && self.screen != Screen::Approve {
            let _ = self.poll_requests();
        }
        if self.offline {
            if self.push().is_ok() {
                self.offline = false;
                let _ = fs::remove_file(self.dir.join("sync").join("outbox.bin"));
            }
            return Ok(());
        }
        if self.member && self.leader_ok() && self.on {
            if self.dirty {
                let _ = self.push();
            } else {
                let _ = self.pull_if_newer();
            }
        }
        Ok(())
    }

    fn poll_join(&mut self) -> Result<(), String> {
        let account = self.account.clone();
        let id = self.join_id.clone();
        let path = format!("/v1/mailboxes/{account}/requests/{id}");
        let proof = self.join_proof.clone();
        let reply = self
            .client
            .open_headers("GET", &path, None, &[("X-Omacus-Proof", proof)])?;
        let state = reply
            .json
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("");
        if state == "pending" {
            return Ok(());
        }
        if state != "approved" {
            self.error = "That phone or computer said no.".into();
            self.screen = Screen::Join;
            return Err(self.error.clone());
        }
        let chain = reply
            .json
            .get("chain")
            .and_then(Value::as_str)
            .unwrap_or("");
        let wrap = reply.json.get("wrap").and_then(Value::as_str).unwrap_or("");
        self.pending_device = wire::unb64(chain)?;
        self.pending_name = wrap.to_string();
        // The approving computer publishes this device onto the list first.
        // Until that record is readable, stay on the waiting screen.
        if self.fetch_mailbox().is_err() {
            return Ok(());
        }
        let done = format!("/v1/mailboxes/{account}/requests/{id}");
        let desk = self.desk.as_ref().ok_or("Sync is off.")?;
        self.client
            .signed(desk, "POST", &done, Some(json!({ "done": true })), &[])?;
        self.screen = Screen::Match;
        self.mark("approve");
        Ok(())
    }

    pub fn accept_match(&mut self) -> Result<(), String> {
        self.error.clear();
        if let Some(desk) = self.desk.as_mut() {
            desk.confirm_link();
        }
        let chain = self.pending_device.clone();
        let wrap = wire::unb64(&self.pending_name)?;
        let mailbox = self.fetch_mailbox()?;
        let record = wire::unb64(
            mailbox
                .json
                .get("record")
                .and_then(Value::as_str)
                .unwrap_or(""),
        )?;
        self.revision = mailbox
            .json
            .get("revision")
            .and_then(Value::as_u64)
            .unwrap_or(self.revision);
        let plan = {
            let desk = self.desk.as_mut().ok_or("Sync is off.")?;
            desk.approval_plan(&chain, &wrap, &record)
                .map_err(|e| e.message().to_string())?
        };
        let incoming = self.fetch_objects(&plan.want)?;
        let (packed, slots) = {
            let desk = self.desk.as_mut().ok_or("Sync is off.")?;
            if plan.legacy {
                (plan.packed, Vec::new())
            } else {
                desk.approval_finish(&record, &incoming)
                    .map_err(|e| e.message().to_string())?
            }
        };
        self.slots = slots;
        self.install_remote(&packed, true)?;
        self.member = true;
        self.on = true;
        self.removed = false;
        self.leader = lock_leader(&self.dir);
        self.save_secret()?;
        if self.clash.is_some() {
            self.screen = Screen::Clash;
            self.mark("clash");
            return Ok(());
        }
        self.dirty = true;
        let _ = self.push();
        self.screen = Screen::Home;
        self.mark("devices");
        Ok(())
    }

    pub fn numbers(&self) -> String {
        if self.screen == Screen::Match
            || (!self.join_code.is_empty() && self.screen == Screen::Waiting)
        {
            return self.join_code.clone();
        }
        if !self.pending_device.is_empty() && self.screen == Screen::Approve {
            return desk::code_for_device(&self.pending_device).unwrap_or_default();
        }
        self.desk
            .as_ref()
            .map(Desk::pairing_code)
            .unwrap_or_default()
    }

    pub fn four_words(&self) -> String {
        if let Some(words) = self
            .desk
            .as_ref()
            .and_then(|desk| desk.account_words().ok())
            && !words.is_empty()
        {
            return words;
        }
        let Some(id) = account_id(&self.account) else {
            return String::new();
        };
        omacus_engine::sync_keys::account_fingerprint(&id).join(" ")
    }

    pub fn poll_requests(&mut self) -> Result<(), String> {
        if !self.leader_ok() || self.account.is_empty() {
            return Ok(());
        }
        let account = self.account.clone();
        let path = format!("/v1/mailboxes/{account}/requests");
        let desk = self.desk.as_ref().ok_or("Sync is off.")?;
        let reply = self.client.signed(desk, "GET", &path, None, &[])?;
        let Some(list) = reply.json.get("requests").and_then(Value::as_array) else {
            return Ok(());
        };
        let Some(row) = list
            .iter()
            .find(|row| row.get("state").and_then(Value::as_str) == Some("pending"))
        else {
            if self.screen == Screen::Closed {
                self.pending_id.clear();
            }
            return Ok(());
        };
        self.pending_id = row
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.pending_name = row
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.pending_device = wire::unb64(row.get("device").and_then(Value::as_str).unwrap_or(""))?;
        self.mark("waiting");
        Ok(())
    }

    pub fn open_approve(&mut self) {
        if !self.pending_id.is_empty() {
            self.screen = Screen::Approve;
            self.mark("approve");
        }
    }

    pub fn approve(&mut self) -> Result<(), String> {
        self.error.clear();
        if !self.leader_ok() {
            return Err("This window is watching. The other window approves.".into());
        }
        if self.pending_id.is_empty() {
            return Ok(());
        }
        let device = self.pending_device.clone();
        let numbers = desk::code_for_device(&device).unwrap_or_default();
        if numbers.is_empty() || self.four_words().split_whitespace().count() != 4 {
            self.error = "Those numbers don't match.".into();
            return Err(self.error.clone());
        }
        if let Ok(id) = <[u8; 16]>::try_from(device.get(..16).unwrap_or(&[])) {
            let asked = self.pending_name.clone();
            if let Some(row) = self.meta_device_mut(&id)
                && !asked.is_empty()
            {
                row.name = asked;
            }
        }
        let approval = {
            let desk = self.desk.as_mut().ok_or("Sync is off.")?;
            desk.approve_new(&device)
                .map_err(|e| e.message().to_string())?
        };
        if let Some(name) = Some(self.pending_name.clone()).filter(|n| !n.is_empty()) {
            // The name rides in the sealed meta on the next save.
            let _ = name;
        }
        self.seal_and_publish_approval(&approval.chain, &approval.wrap)?;
        self.pending_id.clear();
        self.pending_device.clear();
        self.screen = Screen::Home;
        self.mark("approve");
        Ok(())
    }

    pub fn deny(&mut self) -> Result<(), String> {
        if self.pending_id.is_empty() {
            self.screen = Screen::Home;
            return Ok(());
        }
        let account = self.account.clone();
        let id = self.pending_id.clone();
        let path = format!("/v1/mailboxes/{account}/requests/{id}");
        let desk = self.desk.as_ref().ok_or("Sync is off.")?;
        let _ = self
            .client
            .signed(desk, "POST", &path, Some(json!({ "reject": true })), &[])?;
        self.pending_id.clear();
        self.pending_device.clear();
        self.invite_code.clear();
        self.screen = Screen::Home;
        Ok(())
    }

    pub fn ask_remove(&mut self, id: &str) {
        if self.devices().iter().any(|row| row.id == id && row.here) {
            return;
        }
        self.rename_id = id.to_string();
        self.screen = Screen::RemoveAsk;
    }

    pub fn remove_device(&mut self, id: &str) -> Result<(), String> {
        let bytes = wire::hex_id(id)?;
        {
            let desk = self.desk.as_mut().ok_or("Sync is off.")?;
            desk.remove_listed(&bytes)
                .map_err(|e| e.message().to_string())?;
        }
        self.dirty = true;
        self.push()?;
        self.screen = Screen::Home;
        self.mark("devices");
        Ok(())
    }

    pub fn begin_rename(&mut self, id: &str) {
        self.rename_id = id.to_string();
        self.rename_text = self.device_name(id).unwrap_or_default();
        self.screen = Screen::Rename;
    }

    pub fn save_rename(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(());
        }
        let id = wire::hex_id(&self.rename_id).ok();
        if let Some(id) = id {
            let mine = self.desk.as_ref().is_some_and(|desk| id == self_id(desk));
            if mine {
                self.set_name(name);
            }
            if let Some(row) = self.meta_device_mut(&id) {
                row.name = name.to_string();
                row.rev = row.rev.saturating_add(1);
            }
        }
        self.dirty = true;
        self.push()?;
        self.screen = Screen::Home;
        self.mark("devices");
        Ok(())
    }

    pub fn ask_off(&mut self) {
        self.screen = Screen::OffAsk;
        self.mark("off");
    }

    pub fn turn_off(&mut self) -> Result<(), String> {
        self.error.clear();
        // Read the latest list first. A phone that joined after this computer
        // last saved has a longer list, and the server refuses an older one.
        for _ in 0..2 {
            if let Err(error) = self.pull_if_newer() {
                if self.dropped_from_list(&error) {
                    self.mark_removed();
                }
                return self.leave_failed();
            }
            if self.removed {
                self.forget_local();
                return Ok(());
            }
            let id = self.desk.as_ref().map(self_id).ok_or("Sync is off.")?;
            let mut prior = self.desk.as_ref().and_then(|desk| desk.export_local().ok());
            let was_dirty = self.dirty;
            // Stay a member until the server accepts the shorter list. Clearing
            // membership first makes push skip the request and drop the key.
            let pushed = (|| {
                let desk = self.desk.as_mut().ok_or("Sync is off.")?;
                desk.remove_listed(&id)
                    .map_err(|e| e.message().to_string())?;
                self.dirty = true;
                self.push_attempt(false)
            })();
            match pushed {
                Ok(()) => {
                    if let Some(bytes) = &mut prior {
                        bytes.zeroize();
                    }
                    self.forget_local();
                    return Ok(());
                }
                Err(error) if error.contains("out of date") => {
                    self.restore_desk(prior);
                    self.dirty = was_dirty;
                }
                Err(_) => {
                    self.restore_desk(prior);
                    self.dirty = was_dirty;
                    return self.leave_failed();
                }
            }
        }
        self.leave_failed()
    }

    fn restore_desk(&mut self, prior: Option<Vec<u8>>) {
        let Some(mut bytes) = prior else {
            return;
        };
        if let Ok(desk) = Desk::import_local(&bytes) {
            self.desk = Some(desk);
        }
        bytes.zeroize();
    }

    fn leave_failed(&mut self) -> Result<(), String> {
        self.error = format!(
            "Couldn't reach sync, so {} is still on the list. Try again, or turn off only here. Another phone or computer can still remove this one.",
            self.self_name
        );
        self.screen = Screen::OffFailed;
        Err(self.error.clone())
    }

    pub fn turn_off_here(&mut self) {
        self.forget_local();
        self.mark("off");
    }

    fn forget_local(&mut self) {
        keys::remove(&self.dir);
        let _ = fs::remove_file(self.dir.join("sync").join("outbox.bin"));
        self.desk = None;
        self.slots.clear();
        self.on = false;
        self.member = false;
        self.removed = false;
        self.offline = false;
        self.busy = false;
        self.saving_since = None;
        self.leader = None;
        self.screen = Screen::Closed;
        self.wipe_words();
        self.phrase.zeroize();
        self.phrase.clear();
        self.save_book();
    }

    #[cfg(test)]
    pub fn set_phrase(&mut self, text: &str) {
        self.phrase = text.to_string();
    }

    pub fn recover(&mut self) -> Result<(), String> {
        self.error.clear();
        let check = desk::check_phrase(&self.phrase);
        if !check.ok {
            self.error = check.message;
            return Err(self.error.clone());
        }
        let mut desk =
            Desk::start_recovery(&check.normalized).map_err(|e| e.message().to_string())?;
        let account = desk.accept_phrase().map_err(|e| e.message().to_string())?;
        self.phrase.zeroize();
        self.phrase.clear();
        self.account = account;
        self.desk = Some(desk);
        let mailbox = self.fetch_mailbox()?;
        let record = wire::unb64(
            mailbox
                .json
                .get("record")
                .and_then(Value::as_str)
                .unwrap_or(""),
        )?;
        self.revision = mailbox
            .json
            .get("revision")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let peeked = {
            let desk = self.desk.as_mut().ok_or("Sync is off.")?;
            desk.recovery_peek(&record)
                .map_err(|e| e.message().to_string())?
        };
        let packed = if peeked.partial {
            self.notice = PARTIAL_NOTE.to_string();
            let local = self.packed_bytes()?;
            let desk = self.desk.as_mut().ok_or("Sync is off.")?;
            let record = desk
                .reestablish_pages(&local)
                .map_err(|e| e.message().to_string())?;
            self.slots.clear();
            self.publish_record(&record, &[], &[])?;
            local
        } else {
            let incoming = self.fetch_objects(&peeked.want)?;
            let (packed, slots) = {
                let desk = self.desk.as_mut().ok_or("Sync is off.")?;
                desk.recovery_apply(&record, &incoming)
                    .map_err(|e| e.message().to_string())?
            };
            self.slots = slots;
            packed
        };
        if !peeked.partial {
            self.install_remote(&packed, true)?;
            self.member = self.desk.as_ref().is_some_and(Desk::member);
            self.on = true;
            self.removed = false;
            self.leader = lock_leader(&self.dir);
            self.save_secret()?;
            self.dirty = true;
            let _ = self.push();
        } else {
            self.member = true;
            self.on = true;
            self.leader = lock_leader(&self.dir);
            self.save_secret()?;
        }
        self.screen = Screen::Home;
        self.mark("recover");
        Ok(())
    }

    pub fn note_edit(&mut self, id: &str, text: &str) {
        if let Some((_, page)) = self.pages.iter_mut().find(|(page, _)| page == id) {
            *page = text.to_string();
        } else {
            self.pages.push((id.to_string(), text.to_string()));
            if self.on {
                self.set_member_flag(id, !self.locked.contains(&id.to_string()));
            }
        }
        if let Ok(mut lib) = omacus_doc::Library::load(&self.doc)
            .or_else(|_| Ok::<_, String>(omacus_doc::Library::new()))
        {
            lib.reconcile(id, text, None);
            lib.note_written(id, text);
            self.doc = lib.save();
        }
        self.write_page(id, text);
        if self.on && self.meta_in_vault(id) {
            self.dirty = true;
            self.mark("merge");
        }
    }

    pub fn delete_page(&mut self, id: &str) {
        self.pages.retain(|(page, _)| page != id);
        if let Ok(mut lib) = omacus_doc::Library::load(&self.doc) {
            lib.remove_page(id);
            self.doc = lib.save();
        }
        self.membership.retain(|member| member.id != id);
        let _ = fs::remove_file(self.dir.join("pages").join(format!("{id}.txt")));
        if self.on {
            self.dirty = true;
            self.mark("trash");
        }
    }

    pub fn restore_page(&mut self, id: &str, text: &str) {
        self.note_edit(id, text);
        self.set_member_flag(id, true);
        self.dirty = true;
        self.mark("trash");
    }

    pub fn set_in_set(&mut self, id: &str, inside: bool) {
        self.set_member_flag(id, inside && !self.locked.contains(&id.to_string()));
        self.dirty = true;
        self.mark("set");
    }

    #[cfg(test)]
    pub fn unlock_page(&mut self, id: &str) {
        self.locked.retain(|locked| locked != id);
        self.set_member_flag(id, true);
        self.dirty = true;
        self.screen = if self.screen == Screen::Locked {
            Screen::Home
        } else {
            self.screen
        };
        self.mark("locked");
    }

    pub fn leave_locked(&mut self) {
        if let Some(id) = self.locked.first().cloned() {
            self.set_member_flag(&id, false);
        }
        self.lock_settled = true;
        self.screen = Screen::PagesHere;
        self.mark("locked");
    }

    pub fn set_place(&mut self, page: &str, folder: Option<&str>) {
        if let Ok(mut lib) = omacus_doc::Library::load(&self.doc)
            .or_else(|_| Ok::<_, String>(omacus_doc::Library::new()))
        {
            lib.set_place(page, folder);
            self.doc = lib.save();
        }
        self.dirty = true;
        self.mark("folders");
    }

    #[cfg(test)]
    pub fn ensure_folder(&mut self, id: &str, name: &str, parent: Option<&str>) {
        if let Ok(mut lib) = omacus_doc::Library::load(&self.doc)
            .or_else(|_| Ok::<_, String>(omacus_doc::Library::new()))
        {
            let mut folders = lib.folders();
            if !folders.iter().any(|folder| folder.id == id) {
                folders.push(omacus_doc::Folder {
                    id: id.to_string(),
                    name: name.to_string(),
                    parent: parent.map(str::to_string),
                });
                let seeds = folders
                    .iter()
                    .map(|folder| omacus_doc::FolderSeed {
                        id: folder.id.clone(),
                        name: folder.name.clone(),
                        parent: folder.parent.clone(),
                    })
                    .collect::<Vec<_>>();
                lib.set_folders(&seeds);
            }
            self.doc = lib.save();
        }
        self.dirty = true;
        self.mark("folders");
    }

    #[cfg(test)]
    pub fn place_of(&self, page: &str) -> Option<String> {
        omacus_doc::Library::load(&self.doc).ok()?.place_of(page)
    }

    pub fn keep_both(&mut self) -> Result<(), String> {
        let Some(clash) = self.clash.clone() else {
            return Ok(());
        };
        if let Some((_, text)) = self.pages.iter_mut().find(|(id, _)| id == &clash.local_id) {
            let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
            if lines.is_empty() {
                lines.push(format!("{}, this computer", clash.title));
            } else {
                lines[0] = format!("{}, this computer", clash.title);
            }
            *text = lines.join("\n");
            let id = clash.local_id.clone();
            let updated = text.clone();
            self.write_page(&id, &updated);
        }
        self.clash = None;
        self.dirty = true;
        self.push()?;
        self.screen = Screen::Home;
        self.mark("clash");
        Ok(())
    }

    pub fn leave_clash(&mut self) -> Result<(), String> {
        if let Some(clash) = self.clash.clone() {
            self.set_member_flag(&clash.local_id, false);
        }
        self.clash = None;
        self.dirty = true;
        self.push()?;
        self.screen = Screen::Home;
        self.mark("clash");
        Ok(())
    }

    pub fn chip(&self, now: Instant) -> Chip {
        if self.removed {
            return Chip::Paused;
        }
        if !self.on
            || matches!(
                self.screen,
                Screen::Intro
                    | Screen::Choice
                    | Screen::Start
                    | Screen::Words
                    | Screen::Check
                    | Screen::OffAsk
                    | Screen::OffFailed
            )
        {
            return Chip::Hidden;
        }
        if !self.pending_id.is_empty() && self.screen == Screen::Closed {
            return Chip::Waiting;
        }
        if self.offline {
            return Chip::Offline;
        }
        if !self.error.is_empty() && self.member && self.screen == Screen::Closed {
            return Chip::NotSaved;
        }
        if self.busy
            && self
                .saving_since
                .is_some_and(|start| now.duration_since(start) >= Duration::from_secs(1))
        {
            return Chip::Syncing;
        }
        if self.member {
            Chip::Synced
        } else {
            Chip::Hidden
        }
    }

    pub fn offer_setup(&self) -> bool {
        !self.on && !self.removed
    }

    pub fn grouped(&self) -> bool {
        self.on
    }

    pub fn waiting(&self) -> bool {
        !self.pending_id.is_empty() && self.on
    }

    pub fn in_set(&self, id: &str) -> bool {
        if !self.on {
            return true;
        }
        self.meta_in_vault(id) && !self.locked.contains(&id.to_string())
    }

    #[cfg(test)]
    pub fn pages(&self) -> &[(String, String)] {
        &self.pages
    }

    pub fn page_text(&self, id: &str) -> Option<&str> {
        self.pages
            .iter()
            .find(|(page, _)| page == id)
            .map(|(_, text)| text.as_str())
    }

    pub fn devices(&self) -> Vec<DeviceRow> {
        let Ok(rows) = self
            .desk
            .as_ref()
            .ok_or(())
            .and_then(|desk| desk.listed_devices().map_err(|_| ()))
        else {
            return Vec::new();
        };
        rows.into_iter()
            .map(|(id, _code, here)| {
                let name = self
                    .device_name(&id)
                    .unwrap_or_else(|| "Phone or computer".to_string());
                DeviceRow { id, name, here }
            })
            .collect()
    }

    #[cfg(test)]
    pub fn sent_bodies(&self) -> &[String] {
        &self.client.sent
    }

    pub fn leader_ok(&self) -> bool {
        self.leader.is_some()
    }

    pub fn vault_name(&self) -> &str {
        &self.vault_name
    }

    pub fn clash(&self) -> Option<&Clash> {
        self.clash.as_ref()
    }

    pub fn first_locked(&self) -> Option<&str> {
        self.locked.first().map(String::as_str)
    }

    pub fn pending_label(&self) -> &str {
        &self.pending_name
    }

    pub fn removing(&self) -> &str {
        &self.rename_id
    }

    pub fn lock_settled(&self) -> bool {
        self.lock_settled
    }

    pub fn locked_title(&self) -> String {
        self.locked
            .first()
            .and_then(|id| self.page_text(id))
            .map(crate::sheets::heading_of)
            .filter(|title| title != "Untitled" && title != "Locked page")
            .unwrap_or_else(|| "This page".to_string())
    }

    pub fn mark(&mut self, row: &'static str) {
        if PARITY_ROWS.contains(&row) && !self.covered.contains(&row) {
            self.covered.push(row);
        }
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn publish_first(&mut self) -> Result<(), String> {
        self.on = true;
        self.member = true;
        self.sync_membership_lines();
        let packed = self.packed_bytes()?;
        let slots = self.slots.clone();
        let planned = {
            let desk = self.desk.as_mut().ok_or("Sync is off.")?;
            desk.seal_library(&packed, &slots)
                .map_err(|e| e.message().to_string())?
        };
        let record = {
            let desk = self.desk.as_ref().ok_or("Sync is off.")?;
            desk.library_record().map_err(|e| e.message().to_string())?
        };
        self.slots = planned.slots;
        let objects = objects_json(&planned.objects);
        let drop = drop_json(&planned.drop_ids);
        if self.revision == 0 {
            let body = json!({
                "account": self.account,
                "record": wire::b64(&record),
                "code": self.access,
                "objects": objects,
            });
            let reply = self.client.open("POST", "/v1/mailboxes", Some(body))?;
            self.note_written_objects(&reply.json);
            self.revision = reply
                .json
                .get("revision")
                .and_then(Value::as_u64)
                .unwrap_or(1);
        } else {
            let reply = self.publish_record(&record, &objects, &drop)?;
            self.note_written_objects(&reply);
        }
        self.access.zeroize();
        self.access.clear();
        self.remember_base();
        self.dirty = false;
        self.offline = false;
        self.save_secret()?;
        self.save_book();
        self.write_pages();
        Ok(())
    }

    fn push(&mut self) -> Result<(), String> {
        self.push_attempt(true)
    }

    fn push_attempt(&mut self, retry: bool) -> Result<(), String> {
        if !self.leader_ok() {
            return Ok(());
        }
        if !self.member {
            return Ok(());
        }
        self.busy = true;
        self.saving_since = Some(Instant::now());
        let packed = self.packed_bytes()?;
        let slots = self.slots.clone();
        let planned = {
            let Some(desk) = self.desk.as_mut() else {
                self.busy = false;
                return Err("Sync is off.".into());
            };
            match desk.seal_library(&packed, &slots) {
                Ok(planned) => planned,
                Err(error) => {
                    self.busy = false;
                    return Err(error.message().into());
                }
            }
        };
        let record = {
            let desk = self.desk.as_ref().ok_or("Sync is off.")?;
            desk.library_record().map_err(|e| e.message().to_string())?
        };
        let objects = objects_json(&planned.objects);
        let drop = drop_json(&planned.drop_ids);
        match self.publish_record(&record, &objects, &drop) {
            Ok(reply) => {
                self.slots = planned.slots;
                self.note_written_objects(&reply);
                self.remember_base();
                self.dirty = false;
                self.offline = false;
                self.busy = false;
                self.error.clear();
                self.save_secret()?;
                self.save_book();
                let _ = fs::remove_file(self.dir.join("sync").join("outbox.bin"));
                Ok(())
            }
            Err(error) => {
                self.busy = false;
                if self.dropped_from_list(&error) {
                    self.mark_removed();
                    return Ok(());
                }
                if retry && error.contains("out of date") {
                    self.pull_if_newer()?;
                    self.dirty = true;
                    return self.push_attempt(false);
                }
                if error.starts_with("Offline") {
                    self.offline = true;
                    let _ = fs::create_dir_all(self.dir.join("sync"));
                    let _ = fs::write(self.dir.join("sync").join("outbox.bin"), &record);
                    self.mark("offline");
                } else {
                    self.error = error.clone();
                }
                Err(error)
            }
        }
    }

    fn publish_record(
        &mut self,
        record: &[u8],
        objects: &[Value],
        drop: &[Value],
    ) -> Result<Value, String> {
        let account = self.account.clone();
        let path = format!("/v1/mailboxes/{account}");
        let body = json!({
            "revision": self.revision,
            "record": wire::b64(record),
            "objects": objects,
            "drop": drop,
        });
        let desk = self.desk.as_ref().ok_or("Sync is off.")?;
        let reply = self.client.signed(
            desk,
            "PUT",
            &path,
            Some(body),
            &[("If-Match", format!("\"{}\"", self.revision))],
        )?;
        self.revision = reply
            .json
            .get("revision")
            .and_then(Value::as_u64)
            .unwrap_or(self.revision);
        Ok(reply.json)
    }

    fn seal_and_publish_approval(&mut self, chain: &[u8], wrap: &[u8]) -> Result<(), String> {
        let packed = self.packed_bytes()?;
        let slots = self.slots.clone();
        let planned = {
            let desk = self.desk.as_mut().ok_or("Sync is off.")?;
            desk.seal_library(&packed, &slots)
                .map_err(|e| e.message().to_string())?
        };
        let record = {
            let desk = self.desk.as_ref().ok_or("Sync is off.")?;
            desk.library_record().map_err(|e| e.message().to_string())?
        };
        let account = self.account.clone();
        let id = self.pending_id.clone();
        let path = format!("/v1/mailboxes/{account}/requests/{id}");
        let body = json!({
            "chain": wire::b64(chain),
            "wrap": wire::b64(wrap),
            "record": wire::b64(&record),
            "revision": self.revision,
            "objects": objects_json(&planned.objects),
            "drop": drop_json(&planned.drop_ids),
        });
        let desk = self.desk.as_ref().ok_or("Sync is off.")?;
        let reply = self.client.signed(
            desk,
            "POST",
            &path,
            Some(body),
            &[("If-Match", format!("\"{}\"", self.revision))],
        )?;
        self.slots = planned.slots;
        self.revision = reply
            .json
            .get("revision")
            .and_then(Value::as_u64)
            .unwrap_or(self.revision);
        self.note_written_objects(&reply.json);
        self.remember_base();
        self.save_secret()?;
        Ok(())
    }

    fn fetch_mailbox(&mut self) -> Result<wire::Reply, String> {
        let account = self.account.clone();
        let path = format!("/v1/mailboxes/{account}");
        let desk = self.desk.as_ref().ok_or("Sync is off.")?;
        self.client.signed(desk, "GET", &path, None, &[])
    }

    fn fetch_objects(&mut self, ids: &[[u8; 16]]) -> Result<Vec<Incoming>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let account = self.account.clone();
        let path = format!("/v1/mailboxes/{account}/objects");
        let id_list: Vec<_> = ids.iter().map(|id| desk::hex(id)).collect();
        let desk = self.desk.as_ref().ok_or("Sync is off.")?;
        let reply =
            self.client
                .signed(desk, "POST", &path, Some(json!({ "ids": id_list })), &[])?;
        let mut out = Vec::new();
        for row in reply
            .json
            .get("objects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            out.push(Incoming {
                id: wire::hex_id(row.get("id").and_then(Value::as_str).unwrap_or(""))?,
                revision: row.get("revision").and_then(Value::as_u64).unwrap_or(0),
                body: wire::unb64(row.get("body").and_then(Value::as_str).unwrap_or(""))?,
            });
        }
        Ok(out)
    }

    /// A device that was on the list and then dropped gets the same reply as a
    /// missing mailbox. The pages stay on this computer and the chip says
    /// Sync paused. A computer that has never joined still hears the error.
    fn dropped_from_list(&self, error: &str) -> bool {
        self.member && error == "No pages for that yet."
    }

    fn mark_removed(&mut self) {
        self.removed = true;
        self.member = false;
        self.busy = false;
        self.error.clear();
        self.screen = Screen::Removed;
        self.save_book();
    }

    fn pull_if_newer(&mut self) -> Result<(), String> {
        let mailbox = match self.fetch_mailbox() {
            Ok(mailbox) => mailbox,
            Err(error) => {
                if self.dropped_from_list(&error) {
                    self.mark_removed();
                    return Ok(());
                }
                return Err(error);
            }
        };
        let revision = mailbox
            .json
            .get("revision")
            .and_then(Value::as_u64)
            .unwrap_or(self.revision);
        if revision == self.revision {
            return Ok(());
        }
        let record = wire::unb64(
            mailbox
                .json
                .get("record")
                .and_then(Value::as_str)
                .unwrap_or(""),
        )?;
        let slots = self.slots.clone();
        let (legacy, want) = {
            let desk = self.desk.as_mut().ok_or("Sync is off.")?;
            desk.wanted_objects(&record, &slots)
                .map_err(|e| e.message().to_string())?
        };
        let _ = legacy;
        let incoming = self.fetch_objects(&want)?;
        let doc = self.doc.clone();
        let opened = {
            let desk = self.desk.as_mut().ok_or("Sync is off.")?;
            desk.open_library(&record, &incoming, &slots, &doc)
                .map_err(|e| e.message().to_string())?
        };
        self.revision = revision;
        self.member = opened.member;
        if !opened.member && !opened.replace {
            self.removed = true;
            self.member = false;
            self.screen = Screen::Removed;
            self.save_book();
            self.mark("off");
            return Ok(());
        }
        if !opened.replace {
            self.save_secret()?;
            return Ok(());
        }
        self.slots = opened.slots;
        let changed = self.dirty_vs_base();
        self.install_remote(&opened.packed, changed)?;
        if opened.republish && self.member {
            self.dirty = true;
        }
        Ok(())
    }

    fn install_remote(&mut self, packed: &[u8], combine: bool) -> Result<(), String> {
        let remote = PageSet::from_bytes(packed).map_err(|e| e.message().to_string())?;
        let merged = if combine && self.dirty_vs_base() {
            let local = self.page_set()?;
            self.dirty = true;
            local.merge(remote)
        } else {
            remote
        };
        let locked_pages: Vec<_> = self
            .locked
            .iter()
            .filter_map(|id| self.pages.iter().find(|(page, _)| page == id).cloned())
            .collect();
        self.pre_ids = self.pages.iter().map(|(id, _)| id.clone()).collect();
        self.doc = merged.doc;
        self.pages = merged.pages;
        self.vault_name = if merged.meta.vault_name.is_empty() {
            self.vault_name.clone()
        } else {
            merged.meta.vault_name.clone()
        };
        self.name_rev = merged.meta.name_rev.max(self.name_rev);
        self.membership = merged.meta.membership.clone();
        self.absorb_labels(&merged.meta.devices);
        for (id, text) in locked_pages {
            if !self.pages.iter().any(|(page, _)| page == &id) {
                self.pages.push((id.clone(), text));
            }
            self.set_member_flag(&id, false);
        }
        self.notice_clash();
        if self.clash.is_none() && !self.dirty {
            self.remember_base();
        }
        self.write_pages();
        let _ = fs::write(self.dir.join("library.automerge"), &self.doc);
        self.save_book();
        Ok(())
    }

    fn notice_clash(&mut self) {
        let mut seen: Vec<(String, String)> = Vec::new();
        for (id, text) in &self.pages {
            let title = crate::sheets::heading_of(text);
            if title == "Untitled" {
                continue;
            }
            if let Some((other, _)) = seen.iter().find(|(_, name)| name == &title) {
                let local = if self.pre_ids.contains(id) {
                    id.clone()
                } else {
                    other.clone()
                };
                self.clash = Some(Clash {
                    title,
                    local_id: local,
                });
                return;
            }
            seen.push((id.clone(), title));
        }
    }

    fn page_set(&self) -> Result<PageSet, String> {
        let mut meta = SyncMeta {
            vault_name: self.vault_name.clone(),
            name_rev: self.name_rev,
            devices: Vec::new(),
            membership: self.membership.clone(),
        };
        if let Some(desk) = &self.desk {
            let id = self_id(desk);
            meta.devices.push(DeviceLabel {
                id,
                rev: self.self_rev,
                name: self.self_name.clone(),
            });
        }
        for row in self.devices_for_meta() {
            if !meta.devices.iter().any(|have| have.id == row.id) {
                meta.devices.push(row);
            }
        }
        Ok(PageSet {
            doc: self.doc.clone(),
            pages: self.pages.clone(),
            meta,
        })
    }

    fn packed_bytes(&self) -> Result<Vec<u8>, String> {
        let mut set = self.page_set()?;
        set.retain_vault();
        set.to_bytes().map_err(|e| e.message().to_string())
    }

    fn sync_membership_lines(&mut self) {
        for (id, _) in self.pages.clone() {
            if !self.membership.iter().any(|member| member.id == id) {
                let inside = !self.locked.contains(&id);
                self.membership.push(PageMember {
                    id,
                    rev: 1,
                    in_vault: inside,
                });
            }
        }
    }

    fn set_member_flag(&mut self, id: &str, inside: bool) {
        if let Some(row) = self.membership.iter_mut().find(|member| member.id == id) {
            row.rev = row.rev.saturating_add(1);
            row.in_vault = inside;
        } else {
            self.membership.push(PageMember {
                id: id.to_string(),
                rev: 1,
                in_vault: inside,
            });
        }
    }

    fn meta_in_vault(&self, id: &str) -> bool {
        self.membership
            .iter()
            .find(|member| member.id == id)
            .map(|member| member.in_vault)
            .unwrap_or(true)
    }

    fn dirty_vs_base(&self) -> bool {
        if self.dirty {
            return true;
        }
        for (id, text) in &self.pages {
            let hash = hash_text(text);
            match self.bases.iter().find(|(page, _)| page == id) {
                Some((_, base)) if base == &hash => {}
                Some(_) => return true,
                None if self.meta_in_vault(id) => return true,
                None => {}
            }
        }
        false
    }

    fn remember_base(&mut self) {
        self.bases = self
            .pages
            .iter()
            .filter(|(id, _)| self.meta_in_vault(id))
            .map(|(id, text)| (id.clone(), hash_text(text)))
            .collect();
    }

    fn wipe_words(&mut self) {
        if let Some(words) = &mut self.words {
            for word in words.iter_mut() {
                word.zeroize();
            }
        }
        self.words = None;
        self.picks = [None, None, None];
        self.choices.clear();
    }

    fn save_secret(&self) -> Result<(), String> {
        let Some(desk) = &self.desk else {
            return Ok(());
        };
        let device = desk.export_local().map_err(|e| e.message().to_string())?;
        let slots = serde_json::to_vec(&slots_value(&self.slots)).unwrap_or_default();
        let mut blob = Vec::new();
        let len = u32::try_from(device.len()).unwrap_or(u32::MAX);
        blob.extend_from_slice(&len.to_be_bytes());
        blob.extend_from_slice(&device);
        blob.extend_from_slice(&slots);
        keys::save(&self.dir, &blob)
    }

    fn save_book(&self) {
        let book = Book {
            account: self.account.clone(),
            revision: self.revision,
            vault_name: self.vault_name.clone(),
            name_rev: self.name_rev,
            self_name: self.self_name.clone(),
            self_rev: self.self_rev,
            member: self.member,
            removed: self.removed,
            devices: self
                .devices()
                .into_iter()
                .map(|row| BookDevice {
                    id: row.id,
                    name: row.name,
                    rev: 1,
                })
                .collect(),
            membership: self
                .membership
                .iter()
                .map(|member| BookMember {
                    id: member.id.clone(),
                    rev: member.rev,
                    inside: member.in_vault,
                })
                .collect(),
            bases: self
                .bases
                .iter()
                .map(|(id, hash)| BookBase {
                    id: id.clone(),
                    hash: hash.clone(),
                })
                .collect(),
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&book) {
            let _ = fs::create_dir_all(self.dir.join("sync"));
            let _ = fs::write(self.dir.join("sync").join("book.json"), bytes);
        }
    }

    fn load_book(&mut self) {
        let Ok(bytes) = fs::read(self.dir.join("sync").join("book.json")) else {
            return;
        };
        let Ok(book) = serde_json::from_slice::<Book>(&bytes) else {
            return;
        };
        if self.account.is_empty() {
            self.account = book.account;
        }
        self.revision = book.revision;
        if !book.vault_name.is_empty() {
            self.vault_name = book.vault_name;
        }
        self.name_rev = book.name_rev;
        if !book.self_name.is_empty() {
            self.self_name = book.self_name;
        }
        self.self_rev = book.self_rev.max(1);
        self.member = book.member;
        self.removed = book.removed;
        self.membership = book
            .membership
            .into_iter()
            .map(|member| PageMember {
                id: member.id,
                rev: member.rev,
                in_vault: member.inside,
            })
            .collect();
        self.bases = book
            .bases
            .into_iter()
            .map(|base| (base.id, base.hash))
            .collect();
        self.labels = book.devices;
    }

    fn load_disk(&mut self) {
        if self.doc.is_empty()
            && let Ok(bytes) = fs::read(self.dir.join("library.automerge"))
        {
            self.doc = bytes;
        }
        let pages = self.dir.join("pages");
        let Ok(read) = fs::read_dir(&pages) else {
            return;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("txt") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if text.starts_with("OMACUS-LOCK") && !self.locked.contains(&id.to_string()) {
                self.locked.push(id.to_string());
            }
            if !self.pages.iter().any(|(page, _)| page == id) {
                self.pages.push((id.to_string(), text));
            }
        }
    }

    fn write_pages(&self) {
        let dir = self.dir.join("pages");
        let _ = fs::create_dir_all(&dir);
        for (id, text) in &self.pages {
            if self.locked.contains(id) {
                continue;
            }
            let _ = fs::write(dir.join(format!("{id}.txt")), text);
        }
        let _ = fs::write(self.dir.join("library.automerge"), &self.doc);
    }

    fn write_page(&self, id: &str, text: &str) {
        let dir = self.dir.join("pages");
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join(format!("{id}.txt")), text);
    }

    fn note_written_objects(&mut self, json: &Value) {
        let Some(rows) = json.get("objects").and_then(Value::as_array) else {
            return;
        };
        for row in rows {
            let id = row.get("id").and_then(Value::as_str).unwrap_or("");
            let rev = row.get("revision").and_then(Value::as_u64).unwrap_or(0);
            if let Some(slot) = self
                .slots
                .iter_mut()
                .find(|slot| desk::hex(&slot.object_id) == id)
            {
                slot.server_rev = rev;
            }
        }
    }

    fn device_name(&self, id: &str) -> Option<String> {
        self.labels
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.name.clone())
            .or_else(|| {
                if self
                    .desk
                    .as_ref()
                    .is_some_and(|desk| desk::hex(&self_id(desk)) == id)
                {
                    Some(self.self_name.clone())
                } else {
                    None
                }
            })
    }

    fn absorb_labels(&mut self, devices: &[DeviceLabel]) {
        for device in devices {
            let id = desk::hex(&device.id);
            if let Some(row) = self.labels.iter_mut().find(|row| row.id == id) {
                if device.rev >= row.rev {
                    row.name = device.name.clone();
                    row.rev = device.rev;
                }
            } else {
                self.labels.push(BookDevice {
                    id,
                    name: device.name.clone(),
                    rev: device.rev,
                });
            }
        }
    }

    fn devices_for_meta(&self) -> Vec<DeviceLabel> {
        self.labels
            .iter()
            .filter_map(|row| {
                let id = wire::hex_id(&row.id).ok()?;
                Some(DeviceLabel {
                    id,
                    rev: row.rev,
                    name: row.name.clone(),
                })
            })
            .collect()
    }

    fn meta_device_mut(&mut self, id: &[u8; 16]) -> Option<&mut BookDevice> {
        let hex = desk::hex(id);
        if !self.labels.iter().any(|row| row.id == hex) {
            self.labels.push(BookDevice {
                id: hex.clone(),
                name: "Phone or computer".into(),
                rev: 1,
            });
        }
        self.labels.iter_mut().find(|row| row.id == hex)
    }

    pub fn active(&self) -> bool {
        self.on
    }

    pub fn wrote(&self) -> bool {
        self.wrote
    }

    pub fn access_buf(&mut self) -> &mut String {
        &mut self.access
    }

    pub fn join_buf(&mut self) -> &mut String {
        &mut self.join_code
    }

    pub fn phrase_buf(&mut self) -> &mut String {
        &mut self.phrase
    }

    pub fn name_buf(&mut self) -> &mut String {
        &mut self.self_name
    }

    pub fn vault_buf(&mut self) -> &mut String {
        &mut self.vault_name
    }

    pub fn rename_buf(&mut self) -> &mut String {
        &mut self.rename_text
    }

    pub fn move_pending(&self) -> Option<&MoveAsk> {
        self.move_ask.as_ref()
    }

    pub fn ask_set_move(&mut self, id: &str, title: &str, inside: bool) {
        self.move_ask = Some(MoveAsk {
            id: id.to_string(),
            title: title.to_string(),
            inside,
        });
        self.mark("set");
    }

    pub fn confirm_move(&mut self) -> Result<(), String> {
        let Some(ask) = self.move_ask.take() else {
            return Ok(());
        };
        self.set_in_set(&ask.id, ask.inside);
        self.announced = Some(ask);
        self.push()
    }

    pub fn take_announced(&mut self) -> Option<MoveAsk> {
        self.announced.take()
    }

    pub fn cancel_move(&mut self) {
        self.move_ask = None;
    }

    pub fn close_screen(&mut self) {
        self.screen = Screen::Closed;
    }

    /// Chip timing, without waiting. `ago` is how long the save has already been running.
    #[cfg(test)]
    pub fn testing_busy(&mut self, ago: Duration) {
        self.on = true;
        self.member = true;
        self.screen = Screen::Closed;
        self.busy = true;
        self.saving_since = Some(Instant::now() - ago);
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Book {
    account: String,
    revision: u64,
    #[serde(default)]
    vault_name: String,
    #[serde(default)]
    name_rev: u64,
    #[serde(default)]
    self_name: String,
    #[serde(default)]
    self_rev: u64,
    #[serde(default)]
    member: bool,
    #[serde(default)]
    removed: bool,
    #[serde(default)]
    devices: Vec<BookDevice>,
    #[serde(default)]
    membership: Vec<BookMember>,
    #[serde(default)]
    bases: Vec<BookBase>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct BookDevice {
    id: String,
    name: String,
    rev: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct BookMember {
    id: String,
    rev: u64,
    inside: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct BookBase {
    id: String,
    hash: String,
}

pub fn guess_name() -> String {
    let os = match std::env::consts::OS {
        "macos" => "Mac",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    format!("Omacus on {os}")
}

pub fn should_clear(current: &str, copied: &str) -> bool {
    !copied.is_empty() && current == copied
}

pub fn intro_lines() -> &'static [&'static str] {
    INTRO
}

fn numbered(words: &[String; 12]) -> String {
    words
        .iter()
        .enumerate()
        .map(|(i, word)| format!("{}. {word}", i + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn choices_for(words: &[String; 12], n: u8) -> Vec<String> {
    let real = words.get(n as usize - 1).cloned().unwrap_or_default();
    let mut decoys = Vec::new();
    for ch in 'a'..='z' {
        for word in omacus_engine::sync_keys::suggest(&ch.to_string()) {
            if word != real
                && !words.iter().any(|have| have == word)
                && !decoys.contains(&word.to_string())
            {
                decoys.push(word.to_string());
            }
            if decoys.len() == 5 {
                break;
            }
        }
        if decoys.len() == 5 {
            break;
        }
    }
    decoys.push(real);
    decoys.sort();
    decoys
}

fn hash_text(text: &str) -> String {
    desk::hex(&omacus_engine::sync_objects::content_hash(text))
}

fn self_id(desk: &Desk) -> [u8; 16] {
    let bytes = desk.public_device();
    let mut id = [0u8; 16];
    id.copy_from_slice(&bytes[..16]);
    id
}

fn objects_json(objects: &[Outgoing]) -> Vec<Value> {
    objects
        .iter()
        .map(|object| {
            json!({
                "id": desk::hex(&object.id),
                "body": wire::b64(&object.body),
                "base": object.base,
            })
        })
        .collect()
}

fn drop_json(ids: &[[u8; 16]]) -> Vec<Value> {
    ids.iter().map(|id| Value::String(desk::hex(id))).collect()
}

fn slots_value(slots: &[Slot]) -> Value {
    Value::Array(
        slots
            .iter()
            .map(|slot| {
                json!({
                    "pageId": slot.page_id,
                    "objectId": desk::hex(&slot.object_id),
                    "hash": desk::hex(&slot.hash),
                    "key": wire::b64(&slot.content_key),
                    "vaultId": desk::hex(&slot.vault_id),
                    "revision": slot.revision,
                    "serverRev": slot.server_rev,
                    "doc": wire::b64(&slot.doc),
                })
            })
            .collect(),
    )
}

fn slots_from(value: &Value) -> Vec<Slot> {
    let Some(list) = value.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in list {
        let Ok(key) = wire::unb64(item.get("key").and_then(Value::as_str).unwrap_or("")) else {
            continue;
        };
        let Ok(key) = <[u8; 32]>::try_from(key.as_slice()) else {
            continue;
        };
        let Ok(object_id) =
            wire::hex_id(item.get("objectId").and_then(Value::as_str).unwrap_or(""))
        else {
            continue;
        };
        let hash = decode_hash(item.get("hash").and_then(Value::as_str).unwrap_or(""))
            .unwrap_or([0u8; 32]);
        let vault = decode_hash_16(item.get("vaultId").and_then(Value::as_str).unwrap_or(""))
            .unwrap_or([0u8; 16]);
        let doc =
            wire::unb64(item.get("doc").and_then(Value::as_str).unwrap_or("")).unwrap_or_default();
        out.push(Slot {
            page_id: item
                .get("pageId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            object_id,
            hash,
            content_key: key,
            vault_id: vault,
            revision: item.get("revision").and_then(Value::as_u64).unwrap_or(0),
            server_rev: item.get("serverRev").and_then(Value::as_u64).unwrap_or(0),
            doc,
        });
    }
    out
}

fn decode_hash(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (slot, chunk) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        let text = std::str::from_utf8(chunk).ok()?;
        *slot = u8::from_str_radix(text, 16).ok()?;
    }
    Some(out)
}

fn decode_hash_16(text: &str) -> Option<[u8; 16]> {
    wire::hex_id(text).ok()
}

fn split_blob(bytes: &[u8]) -> Option<(Vec<u8>, Vec<Slot>)> {
    if bytes.len() < 4 {
        return None;
    }
    let len = u32::from_be_bytes(bytes[..4].try_into().ok()?) as usize;
    if bytes.len() < 4 + len {
        return None;
    }
    let device = bytes[4..4 + len].to_vec();
    let slots = serde_json::from_slice(&bytes[4 + len..]).unwrap_or(Value::Null);
    Some((device, slots_from(&slots)))
}

fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf).map_err(|_| "Try again in a moment.".to_string())?;
    Ok(buf)
}

fn copy_secret(text: &str, miss: &str) -> Result<String, String> {
    match arboard::Clipboard::new().and_then(|mut clip| clip.set_text(text.to_string())) {
        Ok(()) => {
            arm_clear(text.to_string());
            Ok(text.to_string())
        }
        Err(_) => Err(miss.into()),
    }
}

fn arm_clear(copied: String) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(CLIPBOARD_SECS));
        let mut owned = copied;
        if let Ok(mut clip) = arboard::Clipboard::new()
            && let Ok(current) = clip.get_text()
            && should_clear(&current, &owned)
        {
            let _ = clip.clear();
        }
        owned.zeroize();
    });
}

fn deliver_print(words: &str) -> Result<PrintJob, String> {
    let job = PrintJob { path: None };
    if cfg!(test) || std::env::var("OMACUS_SYNC_PRINT").ok().as_deref() == Some("off") {
        let _ = words;
        return Ok(job);
    }
    pipe_print(words).map_err(|_| "Print didn't work. Write the words down.".to_string())?;
    Ok(job)
}

fn pipe_print(words: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Out-Printer"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(words.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let mut child = std::process::Command::new("lp")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(words.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn account_id(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (slot, chunk) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        let text = std::str::from_utf8(chunk).ok()?;
        *slot = u8::from_str_radix(text, 16).ok()?;
    }
    Some(out)
}

fn lock_leader(dir: &Path) -> Option<File> {
    let path = dir.join("sync").join("leader.lock");
    fs::create_dir_all(path.parent()?).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .read(true)
        .open(path)
        .ok()?;
    if lock_exclusive(&file) {
        Some(file)
    } else {
        None
    }
}

fn lock_exclusive(file: &File) -> bool {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
        };
        use windows_sys::Win32::System::IO::OVERLAPPED;
        let mut overlapped = OVERLAPPED::default();
        unsafe {
            LockFileEx(
                file.as_raw_handle() as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            ) != 0
        }
    }
}

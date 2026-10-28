//! Sync for this window.
//!
//! Plain files stay the working copy. A sealed record is what leaves. The
//! server never receives the vault key, the device keys, the recovery words,
//! or the page text.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zeroize::Zeroize;

use omacus_engine::sync_keys::{DeviceLabel, PageMember, PageSet, SyncMeta, Tombstone};
use omacus_engine::sync_objects::{Incoming, Outgoing, Slot};
use omacus_sync_web::desk::{self, Desk};
use omacus_sync_web::invite;

use super::keys;
use super::qr;
use super::wire::{self, Client};

pub const CLIPBOARD_SECS: u64 = 60;
pub const CLIPBOARD_NOTE: &str =
    "A clipboard history may keep these words. Printing them, or writing them down, is safest.";
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
    PassSet,
    PassEnter,
    Keyring,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Hidden,
    Live,
    Syncing,
    Offline,
    Paused,
    NotSaved,
    Waiting,
    ListChanged,
}

impl Chip {
    pub fn label(self) -> Option<&'static str> {
        match self {
            Chip::Hidden => None,
            Chip::Live => Some("Live"),
            Chip::Syncing => Some("Syncing"),
            Chip::Offline => Some("Offline"),
            Chip::Paused => Some("Sync paused"),
            Chip::NotSaved => Some("Not saved"),
            Chip::Waiting => Some("Device waiting"),
            Chip::ListChanged => Some("List changed"),
        }
    }
}

/// Shown when the linked phones and computers are not the list last acknowledged.
pub const LIST_NOTE: &str = "The list of phones and computers changed. One that was removed can still read pages it already saved. New pages stay with the ones still linked. Your recovery words can put the list back.";

/// The relay uses one sentence for a missing mailbox and for a device it will
/// not authenticate. That is not proof this computer was removed.
pub const COPY_UNREADABLE: &str = "That copy could not be read.";

/// What a person sees when the saved copy could not be read. Sync stays on.
pub const COPY_MISSED: &str =
    "This computer could not read the saved copy. Your pages are still here.";

/// Hover and the paused screen. This computer is actually off the list.
pub const PAUSED_HINT: &str =
    "This computer is not on the list. New pages stay here. Open Sync to join again.";

/// The approval dialog, when the red sentence is about the saved copy.
pub const COPY_NOTE: &str = "That sentence is about the saved copy, not this request. The numbers and the words are still the ones on that phone. Yes adds it only if this computer can save the list.";

/// The approval dialog, when the request itself has no numbers or words.
pub const REQUEST_UNREADABLE: &str = "This request could not be read, so it is not added.";

/// The first list is the baseline. The same ids, in any order, are quiet.
pub fn list_ids_changed(previous: &[String], next: &[String]) -> bool {
    if previous.is_empty() {
        return false;
    }
    let mut before = previous.to_vec();
    let mut after = next.to_vec();
    tidy_ids(&mut before);
    tidy_ids(&mut after);
    before != after
}

fn tidy_ids(ids: &mut Vec<String>) {
    ids.retain(|id| !id.is_empty());
    ids.sort();
    ids.dedup();
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

/// A sync step that talks to the key store or the relay. The window queues it
/// and runs it off the thread that draws.
#[derive(Clone, Debug)]
pub enum Work {
    LoadKey,
    BeginSetup,
    PrepareKeyring,
    ContinueAccess,
    Confirm,
    Bring,
    LeaveFiles,
    KeepBoth,
    LeaveClash,
    StartInvite,
    SubmitJoin,
    AcceptMatch,
    Approve,
    Deny,
    Recover,
    TurnOff,
    RemoveDevice(String),
    CheckAgain,
    SubmitPass,
    SaveRename(String),
    ConfirmMove,
    SaveKey,
    Poll,
    SyncNow,
}

impl Work {
    fn label(&self) -> &'static str {
        match self {
            Work::LoadKey => "Opening the sync key…",
            Work::BeginSetup | Work::SubmitPass => "Checking the key store…",
            Work::PrepareKeyring => "Setting up the key store…",
            Work::Approve => "Approving this phone…",
            Work::SaveKey | Work::AcceptMatch | Work::Confirm | Work::Bring | Work::LeaveFiles => {
                "Saving the sync key…"
            }
            Work::Poll => "",
            _ => "Saving…",
        }
    }

    fn quiet(&self) -> bool {
        matches!(self, Work::Poll)
    }
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
    tombstones: Vec<omacus_engine::sync_keys::Tombstone>,
    /// Signed deletes waiting for a yes. More than two pages.
    removal_ask: Vec<omacus_engine::sync_keys::Tombstone>,
    /// A no on this set. The same deletes do not ask again, and the pages stay.
    removal_declined: Vec<omacus_engine::sync_keys::Tombstone>,
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
    seen: Vec<String>,
    list_note: bool,
    /// Screenshot seed. Empty during a real session.
    list_line: String,
    clash: Option<Clash>,
    pre_ids: Vec<String>,
    rename_id: String,
    rename_text: String,
    labels: Vec<BookDevice>,
    move_ask: Option<MoveAsk>,
    announced: Option<MoveAsk>,
    lock_settled: bool,
    pub covered: Vec<&'static str>,
    pass_buf: String,
    pass_again: String,
    pass_lock: Option<keys::PassLock>,
    pass_locked: bool,
    without_keyring: bool,
    pending: Option<Work>,
    frozen: bool,
    progress: String,
    defer_load: bool,
    retry_save: bool,
    approval_sent: bool,
    key_note: String,
    live: super::live::Live,
    /// Sidebar, trash, and settings from the last pull. The window applies it once.
    pending_present: Option<String>,
    #[cfg(test)]
    fail_key_write: bool,
    #[cfg(test)]
    needs_key_setup: bool,
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
        if keys::has_pass_file(&session.dir) {
            session.pass_locked = true;
            session.screen = Screen::PassEnter;
        } else if keys::defer_secret_load(&session.dir) {
            // The key store is opened on the first background step, not while
            // the window is being created.
            session.defer_load = true;
        } else if let Some(plain) = keys::load(&session.dir).ok().flatten()
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
        session.restore_false_pause();
        session.tidy_half_state();
        if session.desk.is_none() && !session.defer_load && session.approval_sent {
            session.approval_sent = false;
            session.save_book();
        } else if !session.defer_load
            && session.approval_sent
            && session.desk.is_some()
            && !session.pass_locked
        {
            session.retry_save = true;
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
            tombstones: Vec::new(),
            removal_ask: Vec::new(),
            removal_declined: Vec::new(),
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
            seen: Vec::new(),
            list_note: false,
            list_line: String::new(),
            clash: None,
            pre_ids: Vec::new(),
            rename_id: String::new(),
            rename_text: String::new(),
            labels: Vec::new(),
            move_ask: None,
            announced: None,
            lock_settled: false,
            covered: Vec::new(),
            pass_buf: String::new(),
            pass_again: String::new(),
            pass_lock: None,
            pass_locked: false,
            without_keyring: false,
            pending: None,
            frozen: false,
            progress: String::new(),
            defer_load: false,
            retry_save: false,
            approval_sent: false,
            key_note: String::new(),
            live: super::live::Live::new(),
            pending_present: None,
            #[cfg(test)]
            fail_key_write: false,
            #[cfg(test)]
            needs_key_setup: false,
        }
    }

    pub fn ask(&mut self, work: Work) {
        if self.frozen || self.pending.is_some() {
            return;
        }
        self.pending = Some(work);
    }

    pub fn take_pending(&mut self) -> Option<Work> {
        self.pending.take()
    }

    pub fn frozen(&self) -> bool {
        self.frozen
    }

    pub fn progress(&self) -> &str {
        &self.progress
    }

    pub fn key_note(&self) -> &str {
        &self.key_note
    }

    pub fn needs_key_save(&self) -> bool {
        self.approval_sent
    }

    /// True once, when the window should open the key store in the background.
    pub fn claim_deferred_load(&mut self) -> bool {
        if self.defer_load && !self.frozen {
            self.defer_load = false;
            return true;
        }
        false
    }

    /// True once, when an approval was sent and the key still needs writing.
    pub fn claim_retry_save(&mut self) -> bool {
        if self.retry_save && !self.frozen {
            self.retry_save = false;
            return true;
        }
        false
    }

    /// A copy the window can paint while the real session is on a background step.
    pub fn display_shell(&self, work: &Work) -> Self {
        let mut shell = Self::blank(self.dir.clone(), &self.origin);
        shell.screen = self.screen;
        shell.on = self.on;
        shell.member = self.member;
        shell.removed = self.removed;
        shell.error = self.error.clone();
        shell.pass_locked = self.pass_locked;
        shell.offline = self.offline;
        shell.account = self.account.clone();
        shell.join_code = self.join_code.clone();
        shell.pending_device = self.pending_device.clone();
        shell.pending_id = self.pending_id.clone();
        shell.pending_name = self.pending_name.clone();
        shell.vault_name = self.vault_name.clone();
        shell.self_name = self.self_name.clone();
        shell.notice = self.notice.clone();
        shell.key_note = self.key_note.clone();
        shell.approval_sent = self.approval_sent;
        shell.live = self.live.clone();
        shell.revision = self.revision;
        shell.dirty = self.dirty;
        shell.frozen = true;
        shell.progress = work.label().to_string();
        if !work.quiet() && shell.screen == Screen::Closed {
            shell.screen = Screen::Intro;
        }
        if work.quiet() {
            shell.busy = false;
            shell.saving_since = None;
        } else {
            shell.busy = true;
            shell.saving_since = Some(Instant::now() - Duration::from_secs(2));
        }
        shell
    }

    pub fn run_work(&mut self, work: Work) {
        self.progress = work.label().to_string();
        if !work.quiet() {
            self.busy = true;
            self.saving_since = Some(Instant::now());
        }
        let result = match work {
            Work::LoadKey => {
                self.load_deferred_key();
                Ok(())
            }
            Work::BeginSetup => {
                self.begin_setup();
                Ok(())
            }
            Work::PrepareKeyring => self.prepare_keyring(),
            Work::ContinueAccess => self.continue_access(),
            Work::Confirm => self.confirm(),
            Work::Bring => self.bring(),
            Work::LeaveFiles => self.leave_files(),
            Work::KeepBoth => self.keep_both(),
            Work::LeaveClash => self.leave_clash(),
            Work::StartInvite => self.start_invite(),
            Work::SubmitJoin => {
                let code = self.join_code.clone();
                self.submit_join(&code)
            }
            Work::AcceptMatch => self.accept_match(),
            Work::Approve => self.approve(),
            Work::Deny => self.deny(),
            Work::Recover => self.recover(),
            Work::TurnOff => self.turn_off(),
            Work::RemoveDevice(id) => self.remove_device(&id),
            Work::CheckAgain => self.check_again(),
            Work::SubmitPass => self.submit_pass(),
            Work::SaveRename(name) => self.save_rename(&name),
            Work::ConfirmMove => self.confirm_move(),
            Work::SaveKey => self.retry_key_write(),
            Work::Poll => self.poll(),
            Work::SyncNow => self.sync_now(),
        };
        if let Err(error) = result
            && error != "Enter a passphrase."
            && !keys::wants_key_setup(&error)
            && self.error.is_empty()
        {
            self.error = error;
        }
        self.progress.clear();
        self.busy = false;
        self.saving_since = None;
        self.frozen = false;
    }

    pub fn job_lost(&mut self) {
        self.frozen = false;
        self.busy = false;
        self.saving_since = None;
        if self.error.is_empty() {
            self.error = "Sync stopped. Try again.".into();
        }
    }

    fn load_deferred_key(&mut self) {
        self.defer_load = false;
        match keys::load(&self.dir) {
            Ok(Some(plain)) => {
                if let Err(error) = self.install_device(plain) {
                    self.error = error;
                    return;
                }
                if self.approval_sent && !self.pass_locked {
                    let _ = self.retry_key_write();
                }
            }
            Ok(None) => self.tidy_half_state(),
            Err(error) => {
                if keys::wants_key_setup(&error) {
                    self.screen = Screen::Keyring;
                    self.error.clear();
                } else if error == "The key store took too long." {
                    self.screen = Screen::Keyring;
                    self.error = "The key store took too long. Try again.".into();
                } else {
                    self.error = error;
                    self.screen = Screen::Intro;
                }
            }
        }
    }

    fn retry_key_write(&mut self) -> Result<(), String> {
        self.save_secret()?;
        self.approval_sent = false;
        self.pending_id.clear();
        self.pending_device.clear();
        self.save_book();
        if self.on {
            self.screen = Screen::Home;
        }
        Ok(())
    }

    /// An earlier attempt wrote sync flags and never stored a key. The pages stay.
    fn tidy_half_state(&mut self) {
        if self.desk.is_some() || self.pass_locked || self.defer_load {
            return;
        }
        if !(self.on || self.member || self.removed) {
            return;
        }
        self.on = false;
        self.member = false;
        self.removed = false;
        self.save_book();
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
                self.words = Some(sample_words());
                self.wrote = true;
                self.screen = Screen::Words;
            }
            "check" => {
                self.words = Some(sample_words());
                self.wrote = true;
                self.go_check();
                let word = self
                    .words
                    .as_ref()
                    .and_then(|words| words.get(self.asks[0] as usize - 1))
                    .cloned()
                    .unwrap_or_default();
                self.pick(0, &word);
            }
            "list" | "list-sheet" => {
                self.on = true;
                self.member = true;
                self.list_note = true;
                self.screen = Screen::Closed;
                self.list_line =
                    "Your device list changed: Harbor was added, Ferry was removed.".into();
            }
            "removal" => {
                self.on = true;
                self.member = true;
                self.screen = Screen::Closed;
                self.pages = vec![
                    ("pantry".into(), "Pantry\noats\n".into()),
                    ("kept".into(), "Kept\nferry = 40\n".into()),
                    ("harbor".into(), "Harbor\ncake\n".into()),
                ];
                self.removal_ask = vec![
                    Tombstone {
                        id: "pantry".into(),
                        rev: 1,
                    },
                    Tombstone {
                        id: "kept".into(),
                        rev: 2,
                    },
                    Tombstone {
                        id: "harbor".into(),
                        rev: 3,
                    },
                ];
            }
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
            "paused" => {
                self.on = true;
                self.member = false;
                self.removed = true;
                self.screen = Screen::Removed;
            }
            "off" => self.screen = Screen::OffAsk,
            "pass" | "pass-enter" => self.screen = Screen::PassEnter,
            "pass-set" => self.screen = Screen::PassSet,
            "keyring" => self.screen = Screen::Keyring,
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
        if self.pass_locked && self.pass_lock.is_none() {
            self.screen = Screen::PassEnter;
            return;
        }
        self.screen = Screen::Intro;
        self.mark("explainer");
    }

    pub fn not_now(&mut self) {
        self.screen = Screen::Closed;
        self.error.clear();
    }

    pub fn begin_setup(&mut self) {
        self.error.clear();
        if self.pass_lock.is_none() && keys::has_pass_file(&self.dir) {
            self.pass_locked = true;
            self.screen = Screen::PassEnter;
            return;
        }
        if self.pass_lock.is_none() && self.without_keyring {
            self.screen = Screen::PassSet;
            return;
        }
        if self.pass_lock.is_none() && self.needs_store_setup() {
            self.screen = Screen::Keyring;
            return;
        }
        self.screen = Screen::Choice;
        self.mark("choice");
    }

    /// The key store is running, or this test asked for the setup step.
    /// A passphrase file is not chosen here.
    fn needs_store_setup(&self) -> bool {
        #[cfg(test)]
        if self.needs_key_setup {
            return true;
        }
        keys::key_store_needs_setup()
    }

    /// One-time key store setup. The passphrase file is not created here.
    pub fn prepare_keyring(&mut self) -> Result<(), String> {
        self.error.clear();
        #[cfg(test)]
        {
            self.needs_key_setup = false;
            if self.approval_sent {
                return self.retry_key_write();
            }
            self.screen = Screen::Choice;
            self.mark("choice");
            Ok(())
        }
        #[cfg(not(test))]
        {
            match keys::prepare_keyring() {
                Ok(()) => {
                    if self.approval_sent {
                        return self.retry_key_write();
                    }
                    self.screen = Screen::Choice;
                    self.mark("choice");
                    Ok(())
                }
                Err(error) => {
                    self.error = error;
                    self.screen = Screen::Keyring;
                    Err(self.error.clone())
                }
            }
        }
    }

    /// The person asked for a passphrase file. This is never the default.
    pub fn choose_passphrase(&mut self) {
        self.error.clear();
        self.without_keyring = true;
        self.screen = Screen::PassSet;
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

    pub fn picked(&self, which: usize) -> Option<&str> {
        self.picks.get(which).and_then(|word| word.as_deref())
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
        if self.removed {
            self.screen = Screen::Removed;
            return;
        }
        if self.on {
            self.screen = Screen::Home;
            self.mark("devices");
        }
    }

    /// Sidebar and the collapsed cloud. Sync stays reachable after a pause.
    pub fn paused(&self) -> bool {
        self.removed
    }

    pub fn entry_visible(&self) -> bool {
        self.offer_setup() || self.on || self.removed
    }

    pub fn entry_label(&self) -> &'static str {
        if self.on || self.removed {
            "Sync"
        } else {
            "Turn on sync"
        }
    }

    /// Open the sync screen from the sidebar, the cloud, or the status pill.
    pub fn open_entry(&mut self) {
        if self.removed {
            self.screen = Screen::Removed;
            return;
        }
        if !self.pending_id.is_empty() && self.screen == Screen::Closed {
            self.open_approve();
            return;
        }
        if self.offer_setup() && !self.on {
            self.open_intro();
            return;
        }
        self.show_home();
    }

    /// What the status pill does when it is tapped.
    pub fn status_opens_list(&self, now: Instant) -> bool {
        self.chip(now) == Chip::ListChanged
    }

    pub fn open_from_status(&mut self, now: Instant) {
        match self.chip(now) {
            Chip::Hidden | Chip::ListChanged => {}
            Chip::Waiting => self.open_approve(),
            // A dismissed pause still opens. A screen that is already open stays.
            Chip::Paused => self.screen = Screen::Removed,
            _ if self.screen != Screen::Closed => {}
            _ => self.open_entry(),
        }
    }

    /// The sentence under the pill. It names the state. It is not a second label.
    pub fn chip_hint(&self, now: Instant) -> String {
        match self.chip(now) {
            Chip::Hidden => String::new(),
            Chip::Live => "These pages are live on your other devices.".into(),
            Chip::Syncing => {
                if self.busy {
                    "Saving these pages.".into()
                } else {
                    "Checking your other devices.".into()
                }
            }
            Chip::Offline => "Offline. Edits are saved on this computer.".into(),
            Chip::Paused => PAUSED_HINT.into(),
            Chip::NotSaved => {
                if self.error.is_empty() {
                    "The last save did not finish. Open Sync to try again.".into()
                } else {
                    self.error.clone()
                }
            }
            Chip::Waiting => "A phone or computer is waiting to join.".into(),
            Chip::ListChanged => "The list of phones and computers changed.".into(),
        }
    }

    /// The join request has numbers and four words. A saved-copy error is separate.
    pub fn request_readable(&self) -> bool {
        if self.pending_device.is_empty() {
            return false;
        }
        let numbers = desk::code_for_device(&self.pending_device).unwrap_or_default();
        !numbers.is_empty() && self.four_words().split_whitespace().count() == 4
    }

    pub fn copy_note(&self) -> &'static str {
        if !self.request_readable() && self.screen == Screen::Approve {
            return REQUEST_UNREADABLE;
        }
        if self.error == COPY_UNREADABLE {
            COPY_NOTE
        } else {
            ""
        }
    }

    /// Ask the server once more. This does not add a device and does not delete pages.
    pub fn check_again(&mut self) -> Result<(), String> {
        self.error.clear();
        let listed = self.desk.as_ref().is_some_and(Desk::member);
        if listed {
            self.removed = false;
            self.member = true;
            self.on = true;
        }
        match self.pull_if_newer() {
            Ok(()) if !self.removed && self.member => {
                self.screen = Screen::Home;
                self.save_book();
                Ok(())
            }
            Ok(()) => {
                self.screen = if self.removed {
                    Screen::Removed
                } else {
                    Screen::Home
                };
                Ok(())
            }
            Err(error) => {
                if listed && self.desk.as_ref().is_some_and(Desk::member) {
                    self.removed = false;
                    self.member = true;
                    self.on = true;
                }
                self.screen = if self.removed {
                    Screen::Removed
                } else {
                    Screen::Home
                };
                Err(error)
            }
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
        let result = if self.dirty {
            self.push()
        } else {
            self.pull_if_newer()
        };
        if result.is_ok() {
            self.observe_list();
        }
        result
    }

    pub fn poll(&mut self) -> Result<(), String> {
        self.observe_list();
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
            #[cfg(not(test))]
            self.ensure_live();
            if self.dirty {
                let _ = self.push();
            } else {
                let _ = self.pull_if_newer();
            }
        }
        Ok(())
    }

    #[cfg(not(test))]
    fn ensure_live(&mut self) {
        if self.live.connected() || self.account.is_empty() {
            return;
        }
        let Some(desk) = self.desk.as_ref() else {
            return;
        };
        let path = format!("/v1/mailboxes/{}/live", self.account);
        let Ok(auth) = desk.sign_request("GET", &path, b"") else {
            return;
        };
        let (scheme, host) = if let Some(host) = self.origin.strip_prefix("https://") {
            ("wss://", host)
        } else if let Some(host) = self.origin.strip_prefix("http://") {
            ("ws://", host)
        } else {
            return;
        };
        let url = format!(
            "{scheme}{host}{path}?device={}&time={}&nonce={}&signature={}",
            auth.device, auth.time, auth.nonce, auth.signature
        );
        self.live.offer(url);
    }

    fn poll_join(&mut self) -> Result<(), String> {
        self.ensure_pass()?;
        let account = self.account.clone();
        let id = self.join_id.clone();
        let path = format!("/v1/mailboxes/{account}/requests/{id}");
        let proof = self.join_proof.clone();
        let reply = match self
            .client
            .open_headers("GET", &path, None, &[("X-Omacus-Proof", proof)])
        {
            Ok(reply) => reply,
            Err(message) => {
                if super::live::join_status(Some(message.as_str()), "") == "expired"
                    && message == "No pages for that yet."
                {
                    self.expire_join();
                    return Err(self.error.clone());
                }
                return Err(message);
            }
        };
        let state = reply
            .json
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("");
        match super::live::join_status(None, state) {
            "open" if state == "pending" => return Ok(()),
            "rejected" => {
                self.error = "That phone or computer said no.".into();
                self.join_id.clear();
                self.screen = Screen::Join;
                return Err(self.error.clone());
            }
            "open" => {}
            _ => {
                self.expire_join();
                return Err(self.error.clone());
            }
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

    fn expire_join(&mut self) {
        self.error = "That code expired. Ask for a new one on the other phone or computer.".into();
        self.join_id.clear();
        self.screen = Screen::Join;
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
        self.reveal_request();
        self.mark("waiting");
        Ok(())
    }

    fn reveal_request(&mut self) {
        if self.screen == Screen::Closed && !self.pending_id.is_empty() {
            self.screen = Screen::Approve;
        }
    }

    pub fn open_approve(&mut self) {
        if !self.pending_id.is_empty() {
            self.screen = Screen::Approve;
            self.mark("approve");
        }
    }

    pub fn approve(&mut self) -> Result<(), String> {
        self.error.clear();
        if self.approval_sent {
            return self.retry_key_write();
        }
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
        self.observe_list();
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
        self.observe_list();
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
            if self.pull_if_newer().is_err() {
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
        self.pass_lock = None;
        self.pass_locked = false;
        self.pass_buf.zeroize();
        self.pass_again.zeroize();
        self.pass_buf.clear();
        self.pass_again.clear();
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

    #[cfg(test)]
    pub fn testing_without_keyring(&mut self) {
        self.without_keyring = true;
    }

    #[cfg(test)]
    pub fn testing_needs_key_setup(&mut self) {
        self.needs_key_setup = true;
        self.without_keyring = false;
    }

    #[cfg(test)]
    pub fn set_pass_fields(&mut self, password: &str, again: &str) {
        self.pass_buf = password.to_string();
        self.pass_again = again.to_string();
    }

    #[cfg(test)]
    pub fn testing_rewrap_pass(&mut self, password: &str) -> Result<(), String> {
        self.without_keyring = true;
        let plain =
            keys::load(&self.dir)?.ok_or_else(|| "That copy could not be read.".to_string())?;
        let lock = keys::new_pass_lock(password)?;
        keys::write_pass(&self.dir, &plain, &lock)?;
        self.pass_lock = Some(lock);
        Ok(())
    }

    #[cfg(test)]
    pub fn desk_open(&self) -> bool {
        self.desk.is_some()
    }

    #[cfg(test)]
    pub fn account_id(&self) -> &str {
        &self.account
    }

    pub fn recover(&mut self) -> Result<(), String> {
        self.error.clear();
        self.ensure_pass()?;
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
        self.note_tombstone(id);
        let _ = fs::remove_file(self.dir.join("pages").join(format!("{id}.txt")));
        if self.on {
            self.dirty = true;
            self.mark("trash");
        }
    }

    fn note_tombstone(&mut self, id: &str) {
        if id.is_empty() {
            return;
        }
        let rev = self
            .tombstones
            .iter()
            .map(|tomb| tomb.rev)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.tombstones.retain(|tomb| tomb.id != id);
        self.tombstones.push(Tombstone {
            id: id.to_string(),
            rev,
        });
    }

    pub fn restore_page(&mut self, id: &str, text: &str) {
        self.tombstones.retain(|tomb| tomb.id != id);
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
                    | Screen::PassSet
                    | Screen::PassEnter
                    | Screen::Keyring
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
        // A failed save stays on the pill on every screen, including approval.
        // Live is a socket that is up. Syncing is the ordinary poll while that
        // socket is down. Offline is a request that could not reach the server.
        if !self.error.is_empty() && self.member {
            return Chip::NotSaved;
        }
        if self.busy
            && self
                .saving_since
                .is_some_and(|start| now.duration_since(start) >= Duration::from_secs(1))
        {
            return Chip::Syncing;
        }
        if self.list_note && self.member {
            return Chip::ListChanged;
        }
        if self.member && self.live.connected() {
            Chip::Live
        } else if self.member {
            Chip::Syncing
        } else {
            Chip::Hidden
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn socket_live(&self) -> bool {
        self.live.connected()
    }

    /// A notice arrived that this device has not pulled yet.
    pub fn remote_ready(&self) -> bool {
        let rev = self.live.revision();
        rev > self.revision && rev != self.live.acked()
    }

    pub fn ack_remote(&self) {
        self.live.ack(self.live.revision());
    }

    pub fn library_bytes(&self) -> Vec<u8> {
        self.doc.clone()
    }

    pub fn take_present(&mut self) -> Option<String> {
        self.pending_present.take()
    }

    /// Fold the sidebar, trash, handles, and settings into the sealed library.
    pub fn note_structural(&mut self, structural: &str) -> bool {
        let mut lib =
            omacus_doc::Library::load(&self.doc).unwrap_or_else(|_| omacus_doc::Library::new());
        if !lib.note_vault(structural) {
            return false;
        }
        self.doc = lib.save();
        let _ = fs::write(self.dir.join("library.automerge"), &self.doc);
        if self.on {
            self.dirty = true;
            self.mark("folders");
        }
        true
    }

    #[cfg(test)]
    pub fn set_socket_live(&self, on: bool) {
        self.live.set_connected(on);
    }

    /// Remember the first device list. Later changes stay noted until Got it.
    pub fn observe_list(&mut self) {
        let Some(desk) = self.desk.as_ref() else {
            return;
        };
        let Ok(rows) = desk.listed_devices() else {
            return;
        };
        let mut ids: Vec<String> = rows.into_iter().map(|(id, _, _)| id).collect();
        tidy_ids(&mut ids);
        if ids.is_empty() {
            return;
        }
        if self.seen.is_empty() {
            self.seen = ids;
            self.list_note = false;
            self.save_book();
            return;
        }
        self.list_note = list_ids_changed(&self.seen, &ids);
    }

    pub fn ack_list(&mut self) {
        if let Some(desk) = self.desk.as_ref()
            && let Ok(rows) = desk.listed_devices()
        {
            let mut ids: Vec<String> = rows.into_iter().map(|(id, _, _)| id).collect();
            tidy_ids(&mut ids);
            if !ids.is_empty() {
                self.seen = ids;
            }
        }
        self.list_note = false;
        self.save_book();
    }

    pub fn list_note_text(&self) -> &'static str {
        if self.list_note { LIST_NOTE } else { "" }
    }

    /// Names for the list-changed sheet. `added` and `removed` are device names.
    pub fn device_change_sentence(added: &[String], removed: &[String]) -> String {
        let mut parts = Vec::new();
        for name in added {
            if !name.is_empty() {
                parts.push(format!("{name} was added"));
            }
        }
        for name in removed {
            if !name.is_empty() {
                parts.push(format!("{name} was removed"));
            }
        }
        if parts.is_empty() {
            "Your device list changed.".to_string()
        } else {
            format!("Your device list changed: {}.", parts.join(", "))
        }
    }

    pub fn list_change_sentence_now(&self) -> String {
        if !self.list_line.is_empty() {
            return self.list_line.clone();
        }
        let current = self.devices();
        let mut added = Vec::new();
        let mut removed = Vec::new();
        for row in &current {
            if !self.seen.iter().any(|id| id == &row.id) {
                added.push(row.name.clone());
            }
        }
        for id in &self.seen {
            if !current.iter().any(|row| &row.id == id) {
                removed.push(
                    self.device_name(id)
                        .unwrap_or_else(|| "A phone or computer".to_string()),
                );
            }
        }
        Self::device_change_sentence(&added, &removed)
    }

    pub fn removal_count(&self) -> usize {
        if self.removal_ask.len() > 2 {
            self.removal_ask.len()
        } else {
            0
        }
    }

    pub fn removal_body(&self) -> String {
        removal_sentence(self.removal_count())
    }

    pub fn removal_titles(&self) -> Vec<String> {
        self.removal_ask
            .iter()
            .filter_map(|tomb| {
                self.pages
                    .iter()
                    .find(|(id, _)| id == &tomb.id)
                    .map(|(_, text)| crate::sheets::heading_of(text))
            })
            .collect()
    }

    /// Yes. The named pages move to Recently deleted, and the deletes are published.
    pub fn confirm_removal(&mut self) {
        let asked = std::mem::take(&mut self.removal_ask);
        if asked.len() <= 2 {
            self.removal_ask = asked;
            return;
        }
        let gone: Vec<(String, String)> = self
            .pages
            .iter()
            .filter(|(id, _)| asked.iter().any(|tomb| tomb.id == *id))
            .cloned()
            .collect();
        self.pages
            .retain(|(id, _)| !asked.iter().any(|tomb| tomb.id == *id));
        if let Ok(mut lib) = omacus_doc::Library::load(&self.doc) {
            for (id, _) in &gone {
                lib.remove_page(id);
            }
            self.doc = lib.save();
        }
        for tomb in &asked {
            if let Some(row) = self.tombstones.iter_mut().find(|have| have.id == tomb.id) {
                if tomb.rev >= row.rev {
                    row.rev = tomb.rev;
                }
            } else {
                self.tombstones.push(tomb.clone());
            }
        }
        self.tombstones
            .retain(|tomb| !self.pages.iter().any(|(id, _)| id == &tomb.id));
        self.park_removed(&gone);
        self.write_pages();
        self.dirty = true;
        self.save_book();
    }

    /// No. The pages stay, and this set of deletes does not ask again.
    pub fn decline_removal(&mut self) {
        let asked = std::mem::take(&mut self.removal_ask);
        if asked.is_empty() {
            return;
        }
        for tomb in &asked {
            if let Some(row) = self
                .removal_declined
                .iter_mut()
                .find(|have| have.id == tomb.id)
            {
                if tomb.rev > row.rev {
                    row.rev = tomb.rev;
                }
            } else {
                self.removal_declined.push(tomb.clone());
            }
        }
        self.tombstones
            .retain(|tomb| !asked.iter().any(|ask| ask.id == tomb.id));
        self.dirty = true;
        self.save_book();
    }

    pub fn offer_setup(&self) -> bool {
        self.pass_locked || (!self.on && !self.removed)
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

    /// Page id and text, for a local proof. Sorted so two devices can be compared.
    pub fn status_pages(&self) -> Vec<(String, String)> {
        let mut pages = self.pages.clone();
        pages.sort_by(|left, right| left.0.cmp(&right.0));
        pages
    }

    pub fn has_pending(&self) -> bool {
        !self.pending_id.is_empty()
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
        self.ensure_pass()?;
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
        self.ensure_pass()?;
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
        let generation = self.desk.as_ref().map(Desk::generation).unwrap_or(0);
        let planned = {
            let Some(desk) = self.desk.as_mut() else {
                self.busy = false;
                return Err("Sync is off.".into());
            };
            match desk.seal_library(&packed, &slots) {
                Ok(planned) => planned,
                Err(error) => {
                    desk.rewind_generation(generation);
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
                if let Some(desk) = self.desk.as_mut() {
                    desk.rewind_generation(generation);
                }
                if self.note_missed_copy(&error) {
                    return Err(self.error.clone());
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
        self.ensure_pass()?;
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
        // The approval is already on the relay. A failed key write retries the
        // write only, so the same yes is not sent twice.
        self.approval_sent = true;
        self.save_book();
        if let Err(error) = self.save_secret() {
            if self.screen != Screen::PassSet
                && self.screen != Screen::PassEnter
                && self.screen != Screen::Keyring
            {
                self.screen = Screen::Home;
                self.error =
                    "The approval was sent. The key on this computer still needs to be saved."
                        .into();
            }
            return Err(if self.error.is_empty() {
                error
            } else {
                self.error.clone()
            });
        }
        self.approval_sent = false;
        self.save_book();
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

    /// The server says "No pages for that yet" both when the mailbox is missing
    /// and when this device does not authenticate. That reply does not remove
    /// this computer. The pages stay, the pill says the copy could not be read,
    /// and Sync stays open. A real removal is a record whose list leaves this
    /// computer out.
    fn note_missed_copy(&mut self, error: &str) -> bool {
        if !self.member || error != "No pages for that yet." {
            return false;
        }
        self.busy = false;
        self.error = COPY_MISSED.into();
        true
    }

    /// A pause written by an older build, while this computer is still on its
    /// own list, was a missed read. Resume. A list that no longer includes
    /// this computer stays paused, and Sync still opens.
    fn restore_false_pause(&mut self) {
        if !self.removed {
            return;
        }
        if self.desk.as_ref().is_some_and(Desk::member) {
            self.removed = false;
            self.member = true;
            self.on = true;
            self.save_book();
            return;
        }
        self.member = false;
        if self.desk.is_some() {
            self.on = true;
        }
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
                if self.note_missed_copy(&error) {
                    return Ok(());
                }
                if !error.starts_with("Offline") {
                    self.error = error.clone();
                }
                return Err(error);
            }
        };
        if self.error == COPY_MISSED {
            self.error.clear();
        }
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
            self.mark_removed();
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
        let local_pages = self.pages.clone();
        // Always a merge. A page that is only here stays in the result and is
        // published. The remote list does not replace this library.
        let local = self.page_set()?;
        let mut merged = local.merge(remote.clone());
        if combine {
            self.dirty = true;
        }
        // Missing is not a delete. A signed tombstone is. More than two of
        // those wait for a yes. A page that stays is published on the next save.
        let mut kept = pages_after(
            &local_pages,
            &merged.pages,
            &merged.meta.tombstones,
            &self.removal_declined,
        );
        let welcome = welcome_plan(&kept.pages, &merged.meta.tombstones);
        let mut welcome_park = Vec::new();
        if welcome.removed.len() > 2 {
            kept.held = true;
            for tomb in &welcome.tombstones {
                if !kept.asked.iter().any(|ask| ask.id == tomb.id) {
                    kept.asked.push(tomb.clone());
                }
                if !merged.meta.tombstones.iter().any(|have| have.id == tomb.id) {
                    merged.meta.tombstones.push(tomb.clone());
                }
            }
        } else if !welcome.removed.is_empty() {
            kept.pages = welcome.pages;
            for tomb in &welcome.tombstones {
                if let Some(row) = merged
                    .meta
                    .tombstones
                    .iter_mut()
                    .find(|have| have.id == tomb.id)
                {
                    if tomb.rev >= row.rev {
                        row.rev = tomb.rev;
                    }
                } else {
                    merged.meta.tombstones.push(tomb.clone());
                }
            }
            if let Ok(mut lib) = omacus_doc::Library::load(&merged.doc) {
                for (id, _) in &welcome.removed {
                    lib.remove_page(id);
                }
                merged.doc = lib.save();
            }
            welcome_park = welcome.removed;
            self.dirty = true;
        }
        let extras: Vec<(String, bool, String)> = kept
            .pages
            .iter()
            .filter(|(id, _)| !merged.pages.iter().any(|(page, _)| page == id))
            .map(|(id, text)| {
                let inside = self.meta_in_vault(id) && !self.locked.contains(id);
                (id.clone(), inside, text.clone())
            })
            .collect();
        let upload = local_pages.iter().any(|(id, _)| {
            !remote.pages.iter().any(|(page, _)| page == id)
                && kept.pages.iter().any(|(page, _)| page == id)
                && self.meta_in_vault(id)
                && !self.locked.contains(id)
        });
        // A held delete must not publish a copy that drops those tombstones.
        // The pages stay here until yes or no. Other missing pages still upload.
        let republish = upload || extras.iter().any(|(_, inside, _)| *inside);
        let locked_pages: Vec<_> = self
            .locked
            .iter()
            .filter_map(|id| self.pages.iter().find(|(page, _)| page == id).cloned())
            .collect();
        self.pre_ids = self.pages.iter().map(|(id, _)| id.clone()).collect();
        self.doc = merged.doc;
        self.pages = kept.pages;
        self.vault_name = if merged.meta.vault_name.is_empty() {
            self.vault_name.clone()
        } else {
            merged.meta.vault_name.clone()
        };
        self.name_rev = merged.meta.name_rev.max(self.name_rev);
        self.membership = merged.meta.membership.clone();
        self.absorb_labels(&merged.meta.devices);
        if kept.held {
            self.removal_ask = kept.asked.clone();
        } else {
            self.removal_ask.clear();
        }
        for tomb in &merged.meta.tombstones {
            if kept.held && !self.removal_ask.iter().any(|ask| ask.id == tomb.id) {
                continue;
            }
            if let Some(row) = self.tombstones.iter_mut().find(|have| have.id == tomb.id) {
                if tomb.rev >= row.rev {
                    row.rev = tomb.rev;
                }
            } else {
                self.tombstones.push(tomb.clone());
            }
        }
        self.tombstones.retain(|tomb| {
            if self.removal_ask.iter().any(|ask| ask.id == tomb.id) {
                return true;
            }
            !self.pages.iter().any(|(id, _)| id == &tomb.id)
        });
        if !extras.is_empty() {
            let mut lib =
                omacus_doc::Library::load(&self.doc).unwrap_or_else(|_| omacus_doc::Library::new());
            for (id, inside, text) in &extras {
                lib.reconcile(id, text, None);
                lib.note_written(id, text);
                self.set_member_flag(id, *inside);
            }
            self.doc = lib.save();
        }
        if republish {
            self.dirty = true;
        }
        for (id, text) in locked_pages {
            if kept.removed.iter().any(|(gone, _)| gone == &id) {
                continue;
            }
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
        self.park_removed(&kept.removed);
        self.park_removed(&welcome_park);
        self.pending_present = omacus_doc::Library::load(&self.doc)
            .ok()
            .and_then(|lib| lib.vault_present());
        let _ = fs::write(self.dir.join("library.automerge"), &self.doc);
        self.save_book();
        Ok(())
    }

    fn park_removed(&self, gone: &[(String, String)]) {
        if gone.is_empty() {
            return;
        }
        let trash = self.dir.join("trash");
        let _ = fs::create_dir_all(&trash);
        for (id, text) in gone {
            let dest = trash.join(format!("{id}.txt"));
            if !dest.exists() {
                let _ = fs::write(&dest, text);
            }
            let _ = fs::remove_file(self.dir.join("pages").join(format!("{id}.txt")));
        }
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
            tombstones: self.tombstones.clone(),
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

    fn save_secret(&mut self) -> Result<(), String> {
        #[cfg(test)]
        if self.fail_key_write {
            self.fail_key_write = false;
            return Err("The key on this computer still needs to be saved.".into());
        }
        self.ensure_pass()?;
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
        let saved = if let Some(lock) = &self.pass_lock {
            keys::write_pass(&self.dir, &blob, lock)
        } else {
            keys::save(&self.dir, &blob)
        };
        match saved {
            Ok(()) => Ok(()),
            Err(error) if keys::wants_key_setup(&error) => {
                self.screen = Screen::Keyring;
                self.error.clear();
                Err(error)
            }
            Err(error) if error == "The key store took too long." => {
                self.screen = Screen::Keyring;
                self.error = "The key store took too long. Try again.".into();
                Err(self.error.clone())
            }
            Err(error) => Err(error),
        }
    }

    fn must_pass(&self) -> bool {
        self.without_keyring || keys::needs_passphrase(&self.dir)
    }

    fn ensure_pass(&mut self) -> Result<(), String> {
        if self.pass_lock.is_some() || !self.must_pass() {
            return Ok(());
        }
        if keys::has_pass_file(&self.dir) {
            self.pass_locked = true;
            self.screen = Screen::PassEnter;
        } else {
            self.screen = Screen::PassSet;
        }
        Err("Enter a passphrase.".into())
    }

    fn install_device(&mut self, plain: Vec<u8>) -> Result<(), String> {
        let (device, slots) =
            split_blob(&plain).ok_or_else(|| "That copy could not be read.".to_string())?;
        let desk = Desk::import_local(&device).map_err(|e| e.message().to_string())?;
        self.account = desk.account_hex().unwrap_or_default();
        self.desk = Some(desk);
        self.slots = slots;
        self.on = !self.removed;
        if self.removed {
            self.member = false;
        }
        if self.leader.is_none() {
            self.leader = lock_leader(&self.dir);
        }
        self.pass_locked = false;
        self.screen = Screen::Closed;
        Ok(())
    }

    pub fn pass_buf(&mut self) -> &mut String {
        &mut self.pass_buf
    }

    pub fn pass_again(&mut self) -> &mut String {
        &mut self.pass_again
    }

    pub fn back_from_pass(&mut self) {
        self.pass_buf.zeroize();
        self.pass_again.zeroize();
        self.pass_buf.clear();
        self.pass_again.clear();
        self.error.clear();
        if self.pass_locked {
            self.screen = Screen::Closed;
        } else {
            self.screen = Screen::Intro;
        }
    }

    pub fn submit_pass(&mut self) -> Result<(), String> {
        self.error.clear();
        if self.screen == Screen::PassSet {
            if self.pass_buf.chars().count() < 8 {
                self.error = "Use at least 8 characters.".into();
                return Err(self.error.clone());
            }
            if self.pass_buf != self.pass_again {
                self.error = "Type the same passphrase again.".into();
                return Err(self.error.clone());
            }
            let lock = match keys::new_pass_lock(&self.pass_buf) {
                Ok(lock) => lock,
                Err(error) => {
                    self.error = error;
                    return Err(self.error.clone());
                }
            };
            self.pass_buf.zeroize();
            self.pass_again.zeroize();
            self.pass_buf.clear();
            self.pass_again.clear();
            self.pass_lock = Some(lock);
            if self.approval_sent {
                self.retry_key_write()?;
                return Ok(());
            }
            self.screen = Screen::Choice;
            self.mark("choice");
            return Ok(());
        }
        if self.pass_buf.is_empty() {
            self.error = "Enter a passphrase.".into();
            return Err(self.error.clone());
        }
        let password = std::mem::take(&mut self.pass_buf);
        match keys::open_pass(&self.dir, &password) {
            Ok((plain, lock)) => {
                let mut password = password;
                password.zeroize();
                self.pass_lock = Some(lock);
                self.install_device(plain)
            }
            Err(error) => {
                let mut password = password;
                password.zeroize();
                self.error = error;
                Err(self.error.clone())
            }
        }
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
            seen: self.seen.clone(),
            tombstones: self
                .tombstones
                .iter()
                .map(|tomb| BookTomb {
                    id: tomb.id.clone(),
                    rev: tomb.rev,
                })
                .collect(),
            removal_declined: self
                .removal_declined
                .iter()
                .map(|tomb| BookTomb {
                    id: tomb.id.clone(),
                    rev: tomb.rev,
                })
                .collect(),
            approval_sent: self.approval_sent,
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
        self.seen = book.seen;
        tidy_ids(&mut self.seen);
        self.labels = book.devices;
        self.tombstones = book
            .tombstones
            .into_iter()
            .map(|tomb| Tombstone {
                id: tomb.id,
                rev: tomb.rev,
            })
            .collect();
        self.approval_sent = book.approval_sent;
        self.removal_declined = book
            .removal_declined
            .into_iter()
            .map(|tomb| Tombstone {
                id: tomb.id,
                rev: tomb.rev,
            })
            .collect();
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

    #[cfg(test)]
    pub fn fail_next_key_write(&mut self) {
        self.fail_key_write = true;
    }

    #[cfg(test)]
    pub fn sent_count(&self) -> usize {
        self.client.sent.len()
    }

    #[cfg(test)]
    pub fn approval_waiting(&self) -> bool {
        self.approval_sent
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
    #[serde(default)]
    seen: Vec<String>,
    #[serde(default)]
    tombstones: Vec<BookTomb>,
    #[serde(default)]
    removal_declined: Vec<BookTomb>,
    #[serde(default)]
    approval_sent: bool,
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct BookTomb {
    id: String,
    rev: u64,
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

fn sample_words() -> [String; 12] {
    [
        "harbor", "ferry", "oats", "bread", "cake", "pantry", "kept", "meadow", "lantern", "river",
        "cedar", "maple",
    ]
    .map(str::to_string)
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

struct KeptPages {
    pages: Vec<(String, String)>,
    removed: Vec<(String, String)>,
    held: bool,
    asked: Vec<Tombstone>,
}

fn pages_after(
    local: &[(String, String)],
    remote: &[(String, String)],
    tombstones: &[Tombstone],
    declined: &[Tombstone],
) -> KeptPages {
    let mut named = Vec::new();
    let mut asked = Vec::new();
    for tomb in tombstones {
        if declined
            .iter()
            .any(|row| row.id == tomb.id && row.rev >= tomb.rev)
        {
            continue;
        }
        if local.iter().any(|(id, _)| id == &tomb.id) && !named.iter().any(|id| id == &tomb.id) {
            named.push(tomb.id.clone());
            asked.push(tomb.clone());
        }
    }
    let held = omacus_engine::sync_objects::mass_delete(local.len(), named.len());
    let mut pages = remote.to_vec();
    let mut removed = Vec::new();
    for (id, text) in local {
        let doomed = named.iter().any(|tomb| tomb == id);
        if pages.iter().any(|(page, _)| page == id) {
            if !held && doomed {
                pages.retain(|(page, _)| page != id);
                removed.push((id.clone(), text.clone()));
            }
            continue;
        }
        if !held && doomed {
            removed.push((id.clone(), text.clone()));
            continue;
        }
        pages.push((id.clone(), text.clone()));
    }
    KeptPages {
        pages,
        removed,
        held,
        asked: if held { asked } else { Vec::new() },
    }
}

const WELCOME_TITLE: &str = "Welcome to Omacus";

struct WelcomePlan {
    pages: Vec<(String, String)>,
    removed: Vec<(String, String)>,
    tombstones: Vec<Tombstone>,
}

/// Identical Welcome to Omacus pages become one. The lower id stays.
/// A different text stays its own page.
fn welcome_plan(pages: &[(String, String)], tombstones: &[Tombstone]) -> WelcomePlan {
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, (_, text)) in pages.iter().enumerate() {
        if crate::sheets::heading_of(text) != WELCOME_TITLE {
            continue;
        }
        if let Some((_, ids)) = groups.iter_mut().find(|(body, _)| body == text) {
            ids.push(index);
        } else {
            groups.push((text.clone(), vec![index]));
        }
    }
    let mut drop_at = Vec::new();
    for (_, indexes) in &groups {
        if indexes.len() < 2 {
            continue;
        }
        let mut ids: Vec<&str> = indexes
            .iter()
            .map(|index| pages[*index].0.as_str())
            .collect();
        ids.sort();
        for id in ids.into_iter().skip(1) {
            if let Some(index) = pages.iter().position(|(page, _)| page == id) {
                drop_at.push(index);
            }
        }
    }
    drop_at.sort_unstable();
    drop_at.dedup();
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for (index, row) in pages.iter().enumerate() {
        if drop_at.contains(&index) {
            removed.push(row.clone());
        } else {
            kept.push(row.clone());
        }
    }
    let mut tombstones = tombstones.to_vec();
    for (id, _) in &removed {
        let rev = tombstones
            .iter()
            .map(|tomb| tomb.rev)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        if let Some(row) = tombstones.iter_mut().find(|tomb| tomb.id == *id) {
            if rev > row.rev {
                row.rev = rev;
            }
        } else {
            tombstones.push(Tombstone {
                id: id.clone(),
                rev,
            });
        }
    }
    let new_tombs: Vec<Tombstone> = removed
        .iter()
        .filter_map(|(id, _)| tombstones.iter().find(|tomb| tomb.id == *id).cloned())
        .collect();
    WelcomePlan {
        pages: kept,
        removed,
        tombstones: new_tombs,
    }
}

pub fn removal_sentence(count: usize) -> String {
    let word = if count == 1 { "page" } else { "pages" };
    format!(
        "Another phone or computer removed {count} {word}. They are still here. Remove them here too, or keep them?"
    )
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

struct ClipHold {
    board: Option<ClipBoard>,
    secret: Option<String>,
    generation: u64,
}

enum ClipBoard {
    /// Kept alive so Linux still serves the text after `set_text` returns.
    Live(arboard::Clipboard),
    #[cfg(test)]
    Memory(String),
}

fn clip_hold() -> &'static Mutex<ClipHold> {
    static HOLD: OnceLock<Mutex<ClipHold>> = OnceLock::new();
    HOLD.get_or_init(|| {
        Mutex::new(ClipHold {
            board: None,
            secret: None,
            generation: 0,
        })
    })
}

fn open_board() -> Result<ClipBoard, String> {
    match arboard::Clipboard::new() {
        Ok(clip) => Ok(ClipBoard::Live(clip)),
        #[cfg(test)]
        Err(_) => Ok(ClipBoard::Memory(String::new())),
        #[cfg(not(test))]
        Err(error) => Err(error.to_string()),
    }
}

fn publish_live(clip: &mut arboard::Clipboard, text: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        use arboard::SetExtLinux;
        clip.set()
            .exclude_from_history()
            .text(text.to_string())
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        clip.set_text(text.to_string())
            .map_err(|error| error.to_string())
    }
}

fn publish(board: &mut ClipBoard, text: &str) -> Result<(), String> {
    match board {
        ClipBoard::Live(clip) => publish_live(clip, text),
        #[cfg(test)]
        ClipBoard::Memory(slot) => {
            slot.clear();
            slot.push_str(text);
            Ok(())
        }
    }
}

fn read_board(board: &mut ClipBoard) -> Result<String, String> {
    match board {
        ClipBoard::Live(clip) => clip.get_text().map_err(|error| error.to_string()),
        #[cfg(test)]
        ClipBoard::Memory(slot) => Ok(slot.clone()),
    }
}

fn clear_board(board: &mut ClipBoard) -> Result<(), String> {
    match board {
        ClipBoard::Live(clip) => clip.clear().map_err(|error| error.to_string()),
        #[cfg(test)]
        ClipBoard::Memory(slot) => {
            slot.zeroize();
            Ok(())
        }
    }
}

fn copy_secret(text: &str, miss: &str) -> Result<String, String> {
    let mut hold = clip_hold().lock().map_err(|_| miss.to_string())?;
    if hold.board.is_none() {
        hold.board = Some(open_board().map_err(|_| miss.to_string())?);
    }
    let board = hold.board.as_mut().ok_or_else(|| miss.to_string())?;
    publish(board, text).map_err(|_| miss.to_string())?;
    if let Some(mut previous) = hold.secret.replace(text.to_string()) {
        previous.zeroize();
    }
    hold.generation = hold.generation.wrapping_add(1);
    let generation = hold.generation;
    drop(hold);
    arm_clear(generation);
    Ok(text.to_string())
}

fn clear_generation(generation: u64) -> bool {
    let Ok(mut hold) = clip_hold().lock() else {
        return false;
    };
    if hold.generation != generation {
        return false;
    }
    let Some(mut secret) = hold.secret.take() else {
        return false;
    };
    let matches = hold
        .board
        .as_mut()
        .and_then(|board| read_board(board).ok())
        .is_some_and(|current| should_clear(&current, &secret));
    if matches && let Some(board) = hold.board.as_mut() {
        let _ = clear_board(board);
    }
    secret.zeroize();
    matches
}

/// Clear a copied secret if the 60 second timer has not run yet, then drop the
/// clipboard. Linux only keeps the text while this process still serves it.
pub fn release_clipboard() {
    let generation = clip_hold().lock().map(|hold| hold.generation).unwrap_or(0);
    let _ = clear_generation(generation);
    if let Ok(mut hold) = clip_hold().lock() {
        hold.board = None;
        if let Some(mut secret) = hold.secret.take() {
            secret.zeroize();
        }
    }
}

#[cfg(not(test))]
fn arm_clear(generation: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(CLIPBOARD_SECS));
        let _ = clear_generation(generation);
    });
}

#[cfg(test)]
fn arm_clear(_generation: u64) {}

#[cfg(test)]
fn clipboard_text() -> String {
    let Ok(mut hold) = clip_hold().lock() else {
        return String::new();
    };
    hold.board
        .as_mut()
        .and_then(|board| read_board(board).ok())
        .unwrap_or_default()
}

#[cfg(test)]
fn overwrite_clipboard(text: &str) {
    let Ok(mut hold) = clip_hold().lock() else {
        return;
    };
    if let Some(board) = hold.board.as_mut() {
        let _ = publish(board, text);
    }
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

#[cfg(test)]
mod clip_tests {
    use super::*;

    #[test]
    fn a_copy_stays_after_the_call_and_quitting_clears_it() {
        let text = "harbor words stay";
        assert_eq!(copy_secret(text, "miss").unwrap(), text);
        assert_eq!(clipboard_text(), text);
        let generation = clip_hold().lock().unwrap().generation;
        assert!(clear_generation(generation));
        assert!(clipboard_text().is_empty());

        assert_eq!(copy_secret(text, "miss").unwrap(), text);
        overwrite_clipboard("typed over");
        let generation = clip_hold().lock().unwrap().generation;
        assert!(!clear_generation(generation));
        assert_eq!(clipboard_text(), "typed over");
        release_clipboard();
        assert!(clipboard_text().is_empty());
    }
}

#[cfg(test)]
mod list_note_tests {
    use super::*;

    #[test]
    fn the_first_list_is_quiet_and_a_later_change_is_not() {
        assert!(!list_ids_changed(&[], &["a".into()]));
        assert!(!list_ids_changed(
            &["b".into(), "a".into()],
            &["a".into(), "b".into()]
        ));
        assert!(list_ids_changed(&["a".into()], &["a".into(), "b".into()]));
        assert!(list_ids_changed(&["a".into(), "b".into()], &["a".into()]));
    }

    #[test]
    fn list_changed_replaces_synced_and_yields_to_offline() {
        let dir = std::env::temp_dir().join(format!("omacus-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        session.on = true;
        session.member = true;
        session.set_socket_live(true);
        assert_eq!(session.chip(Instant::now()), Chip::Live);
        session.list_note = true;
        assert_eq!(session.chip(Instant::now()), Chip::ListChanged);
        assert_eq!(session.list_note_text(), LIST_NOTE);
        session.offline = true;
        assert_eq!(session.chip(Instant::now()), Chip::Offline);
        session.offline = false;
        session.error = "Not saved".into();
        session.screen = Screen::Closed;
        assert_eq!(session.chip(Instant::now()), Chip::NotSaved);
        session.error.clear();
        session.pending_id = "wait".into();
        assert_eq!(session.chip(Instant::now()), Chip::Waiting);
        session.pending_id.clear();
        session.set_socket_live(true);
        session.ack_list();
        assert_eq!(session.chip(Instant::now()), Chip::Live);
        assert!(session.list_note_text().is_empty());
        session.set_socket_live(false);
        assert_eq!(session.chip(Instant::now()), Chip::Syncing);
        assert_eq!(
            crate::sync::poll_gap_ms(false, false, session.socket_live()),
            2_000
        );
        session.offline = true;
        assert_eq!(session.chip(Instant::now()), Chip::Offline);
        session.offline = false;
        session.set_socket_live(true);
        session.list_note = true;
        session.screen = Screen::Closed;
        session.poll().unwrap();
        assert!(session.list_note, "noting the list stopped sync");
        assert_eq!(session.chip(Instant::now()), Chip::ListChanged);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missed_copy_does_not_pause_or_hide_sync() {
        let dir = std::env::temp_dir().join(format!("omacus-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        session.on = true;
        session.member = true;
        session.screen = Screen::Home;
        assert!(session.note_missed_copy("No pages for that yet."));
        assert!(!session.paused());
        assert!(session.member);
        assert_eq!(session.error, COPY_MISSED);
        assert_eq!(session.chip(Instant::now()), Chip::NotSaved);
        assert_eq!(session.chip_hint(Instant::now()), COPY_MISSED);
        assert!(session.entry_visible());
        assert_eq!(session.entry_label(), "Sync");
        assert!(!session.note_missed_copy("Couldn't sync."));
        session.error = COPY_UNREADABLE.into();
        assert_eq!(session.copy_note(), COPY_NOTE);
        session.screen = Screen::Approve;
        assert_eq!(session.copy_note(), REQUEST_UNREADABLE);
        assert!(!session.request_readable());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pause_still_opens_sync_and_a_false_pause_resumes() {
        let dir = std::env::temp_dir().join(format!("omacus-paused-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        session.on = true;
        session.member = true;
        session.mark_removed();
        assert!(session.paused());
        assert!(session.on, "pausing must leave the sidebar entry in place");
        session.on = false;
        assert!(session.entry_visible());
        assert_eq!(session.entry_label(), "Sync");
        session.on = true;
        assert!(session.entry_visible());
        assert_eq!(session.entry_label(), "Sync");
        assert_eq!(session.chip(Instant::now()), Chip::Paused);
        assert!(
            session
                .chip_hint(Instant::now())
                .contains("not on the list")
        );
        assert!(
            session
                .chip_hint(Instant::now())
                .to_lowercase()
                .contains("join again")
        );
        session.screen = Screen::Closed;
        session.open_from_status(Instant::now());
        assert_eq!(session.screen, Screen::Removed);
        session.screen = Screen::Home;
        session.open_from_status(Instant::now());
        assert_eq!(session.screen, Screen::Removed);
        session.removed = false;
        session.member = true;
        session.on = true;
        session.screen = Screen::Home;
        session.error = "Not saved".into();
        session.open_from_status(Instant::now());
        assert_eq!(
            session.screen,
            Screen::Home,
            "a status tap leaves the open sync screen where it is"
        );
        session.screen = Screen::Closed;
        session.open_from_status(Instant::now());
        assert_eq!(session.screen, Screen::Home);
        session.error.clear();
        session.mark_removed();
        for chip in [
            Chip::Live,
            Chip::Syncing,
            Chip::Offline,
            Chip::Paused,
            Chip::NotSaved,
            Chip::Waiting,
            Chip::ListChanged,
        ] {
            assert!(chip.label().is_some());
        }
        let _ = std::fs::remove_dir_all(&dir);

        let kept = std::env::temp_dir().join(format!("omacus-resume-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&kept);
        std::fs::create_dir_all(kept.join("sync")).unwrap();
        let mut desk = Desk::start_new().unwrap();
        let words = desk.words().unwrap();
        let picks: Vec<(u8, &str)> = omacus_sync_web::desk::CONFIRM_AT
            .into_iter()
            .map(|n| (n, words[n as usize - 1]))
            .collect();
        desk.confirm_saved(&picks).unwrap();
        assert!(desk.member());
        let device = desk.export_local().unwrap();
        let mut blob = Vec::new();
        blob.extend_from_slice(&u32::try_from(device.len()).unwrap().to_be_bytes());
        blob.extend_from_slice(&device);
        keys::save(&kept, &blob).unwrap();
        std::fs::write(
            kept.join("sync").join("book.json"),
            r#"{"account":"","revision":1,"removed":true,"member":false}"#,
        )
        .unwrap();
        let resumed = Session::open(kept.clone(), "http://127.0.0.1:9");
        assert!(
            !resumed.paused(),
            "a computer still on its own list resumes"
        );
        assert!(resumed.entry_visible());
        assert_eq!(resumed.entry_label(), "Sync");
        assert_eq!(resumed.chip(Instant::now()), Chip::Syncing);
        let book = std::fs::read_to_string(kept.join("sync").join("book.json")).unwrap();
        assert!(!book.contains("\"removed\": true"));
        let _ = std::fs::remove_dir_all(kept);
    }

    #[test]
    fn a_pending_device_opens_the_approval_screen() {
        let dir = std::env::temp_dir().join(format!("omacus-approve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        session.on = true;
        session.screen = Screen::Closed;
        session.pending_id = "harbor".into();
        session.reveal_request();
        assert_eq!(session.screen, Screen::Approve);
        session.screen = Screen::Home;
        session.reveal_request();
        assert_eq!(session.screen, Screen::Home);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_list_sheet_names_who_joined_and_who_left() {
        let text = Session::device_change_sentence(&["Harbor".into()], &["Ferry".into()]);
        assert_eq!(
            text,
            "Your device list changed: Harbor was added, Ferry was removed."
        );
        assert_eq!(
            Session::device_change_sentence(&[], &[]),
            "Your device list changed."
        );
    }

    #[test]
    fn a_picked_word_stays_picked() {
        let dir = std::env::temp_dir().join(format!("omacus-pick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        session.pick(0, "harbor");
        session.pick(0, "ferry");
        assert_eq!(session.picked(0), Some("ferry"));
        assert_eq!(session.picked(1), None);
        session.set_wrote(true);
        assert!(session.wrote());
        session.set_wrote(false);
        assert!(!session.wrote());
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tomb_tests {
    use super::*;

    fn remote_with(pages: Vec<(String, String)>, tombstones: Vec<Tombstone>) -> Vec<u8> {
        PageSet {
            doc: Vec::new(),
            pages,
            meta: SyncMeta {
                tombstones,
                ..SyncMeta::default()
            },
        }
        .to_bytes()
        .unwrap()
    }

    #[test]
    fn a_page_from_the_phone_stays_until_a_tombstone_names_it() {
        let dir = std::env::temp_dir().join(format!("omacus-tomb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        session.pages = vec![
            ("pantry".into(), "Pantry\noats\n".into()),
            ("kept".into(), "Kept\nferry = 40\n".into()),
            ("harbor".into(), "Harbor\ncake\n".into()),
        ];
        session
            .install_remote(
                &remote_with(vec![("kept".into(), "Kept\nferry = 40\n".into())], vec![]),
                false,
            )
            .unwrap();
        assert!(
            session
                .pages
                .iter()
                .any(|(id, text)| id == "pantry" && text.contains("oats"))
        );
        assert!(session.pages.iter().any(|(id, _)| id == "harbor"));
        assert!(
            session.dirty,
            "the pages only here were not published again"
        );
        assert!(session.tombstones.is_empty());

        session.dirty = false;
        session
            .install_remote(
                &remote_with(
                    vec![("kept".into(), "Kept\nferry = 40\n".into())],
                    vec![Tombstone {
                        id: "pantry".into(),
                        rev: 1,
                    }],
                ),
                false,
            )
            .unwrap();
        assert!(!session.pages.iter().any(|(id, _)| id == "pantry"));
        assert!(session.pages.iter().any(|(id, _)| id == "kept"));
        assert!(session.pages.iter().any(|(id, _)| id == "harbor"));
        let parked =
            std::fs::read_to_string(dir.join("trash").join("pantry.txt")).unwrap_or_default();
        assert!(parked.contains("oats"), "{parked}");
        assert!(session.tombstones.iter().any(|tomb| tomb.id == "pantry"));

        session.pages = vec![
            ("pantry".into(), "Pantry\noats\n".into()),
            ("kept".into(), "Kept\nferry = 40\n".into()),
            ("harbor".into(), "Harbor\ncake\n".into()),
        ];
        session.tombstones.clear();
        session
            .install_remote(
                &remote_with(
                    vec![("harbor".into(), "Harbor\ncake\n".into())],
                    vec![
                        Tombstone {
                            id: "pantry".into(),
                            rev: 1,
                        },
                        Tombstone {
                            id: "kept".into(),
                            rev: 2,
                        },
                    ],
                ),
                false,
            )
            .unwrap();
        assert!(
            !session.pages.iter().any(|(id, _)| id == "pantry"),
            "two signed deletes left pantry"
        );
        assert!(
            !session.pages.iter().any(|(id, _)| id == "kept"),
            "two signed deletes left kept"
        );
        assert!(session.pages.iter().any(|(id, _)| id == "harbor"));
        assert_eq!(session.removal_count(), 0, "two deletes asked first");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn more_than_two_deletes_wait_until_yes() {
        let dir = std::env::temp_dir().join(format!("omacus-mass-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        session.pages = vec![
            ("pantry".into(), "Pantry\noats\n".into()),
            ("kept".into(), "Kept\nferry = 40\n".into()),
            ("harbor".into(), "Harbor\ncake\n".into()),
        ];
        let packed = remote_with(
            vec![],
            vec![
                Tombstone {
                    id: "pantry".into(),
                    rev: 1,
                },
                Tombstone {
                    id: "kept".into(),
                    rev: 2,
                },
                Tombstone {
                    id: "harbor".into(),
                    rev: 3,
                },
            ],
        );
        session.install_remote(&packed, false).unwrap();
        assert_eq!(
            session.pages.len(),
            3,
            "more than two deletes removed a page"
        );
        assert_eq!(session.removal_count(), 3);
        assert!(removal_sentence(3).contains("3 pages"));
        assert_eq!(
            session.removal_titles(),
            vec!["Pantry".to_string(), "Kept".into(), "Harbor".into()]
        );
        session.decline_removal();
        assert_eq!(session.removal_count(), 0);
        assert!(session.dirty, "keeping the pages was not published");
        session.dirty = false;
        session.install_remote(&packed, false).unwrap();
        assert_eq!(session.pages.len(), 3, "a declined delete removed a page");
        assert_eq!(session.removal_count(), 0, "a declined delete asked again");

        session.removal_declined.clear();
        session.install_remote(&packed, false).unwrap();
        assert_eq!(session.removal_count(), 3);
        session.confirm_removal();
        assert!(session.pages.is_empty(), "yes left a page in the list");
        assert_eq!(session.removal_count(), 0);
        let parked =
            std::fs::read_to_string(dir.join("trash").join("pantry.txt")).unwrap_or_default();
        assert!(parked.contains("oats"), "{parked}");
        assert!(session.tombstones.iter().any(|tomb| tomb.id == "pantry"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn two_libraries_keep_every_page_and_one_welcome() {
        let dir = std::env::temp_dir().join(format!("omacus-fold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        let welcome = "# Welcome to Omacus\nA note.\n";
        let mut local_lib = omacus_doc::Library::new();
        local_lib.reconcile("pantry", "Pantry\noats\n", None);
        local_lib.note_written("pantry", "Pantry\noats\n");
        local_lib.reconcile("welcome-b", welcome, None);
        local_lib.note_written("welcome-b", welcome);
        session.doc = local_lib.save();
        session.pages = vec![
            ("pantry".into(), "Pantry\noats\n".into()),
            ("welcome-b".into(), welcome.into()),
        ];
        let mut remote_lib = omacus_doc::Library::new();
        remote_lib.reconcile("harbor", "Harbor\ncake\n", None);
        remote_lib.note_written("harbor", "Harbor\ncake\n");
        remote_lib.reconcile("welcome-a", welcome, None);
        remote_lib.note_written("welcome-a", welcome);
        remote_lib.reconcile("welcome-c", "# Welcome to Omacus\nferry\n", None);
        remote_lib.note_written("welcome-c", "# Welcome to Omacus\nferry\n");
        let remote = PageSet {
            doc: remote_lib.save(),
            pages: vec![
                ("harbor".into(), "Harbor\ncake\n".into()),
                ("welcome-a".into(), welcome.into()),
                ("welcome-c".into(), "# Welcome to Omacus\nferry\n".into()),
            ],
            meta: SyncMeta::default(),
        };
        session
            .install_remote(&remote.to_bytes().unwrap(), false)
            .unwrap();
        assert!(
            session
                .pages
                .iter()
                .any(|(id, text)| id == "pantry" && text.contains("oats")),
            "a page only on this computer was dropped"
        );
        assert!(
            session
                .pages
                .iter()
                .any(|(id, text)| id == "harbor" && text.contains("cake")),
            "a page only on the other library was dropped"
        );
        assert!(
            session.pages.iter().any(|(id, _)| id == "welcome-a"),
            "the earlier welcome page was dropped"
        );
        assert!(
            !session.pages.iter().any(|(id, _)| id == "welcome-b"),
            "an identical welcome page stayed"
        );
        assert!(
            session
                .pages
                .iter()
                .any(|(id, text)| id == "welcome-c" && text.contains("ferry")),
            "a different welcome page was folded away"
        );
        assert!(
            session.tombstones.iter().any(|tomb| tomb.id == "welcome-b"),
            "the identical welcome page was not a signed delete"
        );
        assert!(session.dirty, "the joined pages were not published");
        let shown = omacus_doc::Library::load(&session.doc).unwrap();
        assert!(
            shown
                .snapshot("pantry")
                .unwrap_or_default()
                .contains("oats")
        );
        assert!(
            shown
                .snapshot("harbor")
                .unwrap_or_default()
                .contains("cake")
        );
        assert!(shown.snapshot("welcome-b").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod union_tests {
    use super::*;

    #[test]
    fn a_page_only_on_this_computer_stays_beside_the_other_library() {
        let dir = std::env::temp_dir().join(format!("omacus-union-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        let pantry = "Pantry\noats\n";
        let kept_local = "Kept\nferry = 40\nbread $2\n";
        let kept_remote = "Kept\nferry = 40\n";
        let harbor = "Harbor\ncake\n";
        session.pages = vec![
            ("pantry".into(), pantry.into()),
            ("kept".into(), kept_local.into()),
            ("harbor".into(), harbor.into()),
        ];
        session.bases = vec![
            ("pantry".into(), hash_text(pantry)),
            ("kept".into(), hash_text(kept_local)),
            ("harbor".into(), hash_text(harbor)),
        ];
        session.membership = vec![
            PageMember {
                id: "pantry".into(),
                rev: 1,
                in_vault: true,
            },
            PageMember {
                id: "kept".into(),
                rev: 1,
                in_vault: true,
            },
            PageMember {
                id: "harbor".into(),
                rev: 1,
                in_vault: false,
            },
        ];
        let mut remote_lib = omacus_doc::Library::new();
        remote_lib.reconcile("kept", kept_remote, None);
        remote_lib.note_written("kept", kept_remote);
        let remote = PageSet {
            doc: remote_lib.save(),
            pages: vec![("kept".into(), kept_remote.into())],
            meta: SyncMeta::default(),
        };
        session
            .install_remote(&remote.to_bytes().unwrap(), false)
            .unwrap();
        let pantry_text = session
            .pages
            .iter()
            .find(|(id, _)| id == "pantry")
            .map(|(_, text)| text.as_str())
            .unwrap_or("");
        assert!(pantry_text.contains("oats"), "{pantry_text}");
        let kept_text = session
            .pages
            .iter()
            .find(|(id, _)| id == "kept")
            .map(|(_, text)| text.as_str())
            .unwrap_or("");
        assert!(kept_text.contains("ferry = 40"), "{kept_text}");
        assert!(!kept_text.contains("bread"), "{kept_text}");
        assert!(
            session
                .pages
                .iter()
                .any(|(id, text)| id == "harbor" && text.contains("cake"))
        );
        assert!(session.dirty, "the page only here was not published again");
        assert!(
            session
                .membership
                .iter()
                .any(|member| member.id == "pantry" && member.in_vault)
        );
        assert!(
            session
                .membership
                .iter()
                .any(|member| member.id == "harbor" && !member.in_vault)
        );
        let shown = omacus_doc::Library::load(&session.doc)
            .unwrap()
            .snapshot("pantry")
            .unwrap_or_default();
        assert!(shown.contains("oats"), "{shown}");
        let packed = PageSet::from_bytes(&session.packed_bytes().unwrap()).unwrap();
        assert!(packed.pages.iter().any(|(id, _)| id == "pantry"));
        assert!(packed.pages.iter().any(|(id, _)| id == "kept"));
        assert!(!packed.pages.iter().any(|(id, _)| id == "harbor"));
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod park_tests {
    use super::*;

    #[test]
    fn a_page_still_here_is_not_parked_when_the_other_side_omits_it() {
        let dir = std::env::temp_dir().join(format!("omacus-park-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(dir.join("pages").join("pantry.txt"), "Pantry\noats\n").unwrap();
        fs::write(dir.join("pages").join("kept.txt"), "Kept\n1\n").unwrap();
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        session.pages = vec![
            ("pantry".into(), "Pantry\noats\n".into()),
            ("kept".into(), "Kept\n1\n".into()),
        ];
        session.bases = vec![
            ("pantry".into(), hash_text("Pantry\noats\n")),
            ("kept".into(), hash_text("Kept\n1\n")),
        ];
        let remote = PageSet {
            doc: Vec::new(),
            pages: vec![("kept".into(), "Kept\n1\n".into())],
            meta: SyncMeta::default(),
        };
        session
            .install_remote(&remote.to_bytes().unwrap(), false)
            .unwrap();
        assert_eq!(session.page_text("pantry"), Some("Pantry\noats\n"));
        assert_eq!(session.page_text("kept"), Some("Kept\n1\n"));
        assert!(!dir.join("trash").join("pantry.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_puts_a_recently_deleted_page_back() {
        let dir = std::env::temp_dir().join(format!("omacus-restore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut session = Session::blank(dir.clone(), "http://127.0.0.1:9");
        session.park_removed(&[("pantry".into(), "Pantry\noats\n".into())]);
        let trashed = fs::read_to_string(dir.join("trash").join("pantry.txt")).unwrap();
        assert!(trashed.contains("oats"));
        session.restore_page("pantry", "Pantry\noats\n");
        assert_eq!(session.page_text("pantry"), Some("Pantry\noats\n"));
        let _ = fs::remove_dir_all(&dir);
    }
}

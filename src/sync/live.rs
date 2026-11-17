#![cfg_attr(test, allow(dead_code))]
//! The mailbox tells a device when a write lands.
//!
//! The socket carries a revision number. Page text stays in the sealed library
//! and is pulled after the notice. A dropped socket falls back to polling.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tungstenite::{Message, connect};

/// How long a desktop edit waits before it is sent.
pub const PUSH_DEBOUNCE_MS: u64 = 400;
/// A visible device with no socket asks this often.
pub const FALLBACK_POLL_MS: u64 = 2_000;
/// A live socket is the notice. This poll only catches a missed one.
pub const LIVE_FALLBACK_MS: u64 = 30_000;
pub const WAITING_POLL_MS: u64 = 800;

/// A device that is already syncing keeps the socket up, including while the
/// sync sheet is open. A join that is still waiting keeps asking too.
pub fn should_poll_sync(active: bool, _screen_closed: bool, screen_waiting: bool) -> bool {
    active || screen_waiting
}

pub fn poll_gap_ms(waiting: bool, dirty: bool, live: bool) -> u64 {
    if waiting {
        return WAITING_POLL_MS;
    }
    if dirty {
        return live_delivery_ms();
    }
    if live {
        return LIVE_FALLBACK_MS;
    }
    FALLBACK_POLL_MS
}

/// Desktop to the other device, before the pull. The notice itself is in the write.
pub fn live_delivery_ms() -> u64 {
    PUSH_DEBOUNCE_MS
}

/// What a waiting join should do with a reply.
pub fn join_status(message: Option<&str>, state: &str) -> &'static str {
    if message == Some("No pages for that yet.") {
        return "expired";
    }
    match state {
        "pending" | "approved" => "open",
        "rejected" => "rejected",
        _ => "expired",
    }
}

/// Called from the socket thread when a notice lands or the socket opens or
/// drops. The window is asleep between frames, so something has to wake it.
type Wake = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

fn wake(slot: &Wake) {
    if let Ok(slot) = slot.lock()
        && let Some(call) = slot.as_ref()
    {
        call();
    }
}

/// One socket for this set of pages. The thread starts on the first address.
#[derive(Clone)]
pub struct Live {
    wake: Wake,
    flag: Arc<AtomicBool>,
    rev: Arc<AtomicU64>,
    seen: Arc<AtomicU64>,
    tx: Sender<String>,
    started: Arc<AtomicBool>,
    take_rx: Arc<Mutex<Option<Receiver<String>>>>,
}

impl Live {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            wake: Arc::new(Mutex::new(None)),
            flag: Arc::new(AtomicBool::new(false)),
            rev: Arc::new(AtomicU64::new(0)),
            seen: Arc::new(AtomicU64::new(0)),
            tx,
            started: Arc::new(AtomicBool::new(false)),
            take_rx: Arc::new(Mutex::new(Some(rx))),
        }
    }

    /// What to call when the other device's write is announced. Without it
    /// the window only looks again on its slow fallback poll.
    pub fn on_notice(&self, call: impl Fn() + Send + 'static) {
        if let Ok(mut slot) = self.wake.lock() {
            *slot = Some(Box::new(call));
        }
    }

    pub fn connected(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn set_connected(&self, on: bool) {
        self.flag.store(on, Ordering::Relaxed);
    }

    pub fn revision(&self) -> u64 {
        self.rev.load(Ordering::Relaxed)
    }

    pub fn ack(&self, rev: u64) {
        self.seen.store(rev, Ordering::Relaxed);
    }

    pub fn acked(&self) -> u64 {
        self.seen.load(Ordering::Relaxed)
    }

    /// A fresh signed address. A new nonce is required after a drop.
    pub fn offer(&self, url: String) {
        self.ensure();
        let _ = self.tx.send(url);
    }

    fn ensure(&self) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let flag = Arc::clone(&self.flag);
        let rev = Arc::clone(&self.rev);
        let wake = Arc::clone(&self.wake);
        let rx = self.take_rx.lock().ok().and_then(|mut slot| slot.take());
        let Some(rx) = rx else {
            return;
        };
        thread::Builder::new()
            .name("omacus-live".into())
            .spawn(move || run(rx, flag, rev, wake))
            .ok();
    }
}

impl Default for Live {
    fn default() -> Self {
        Self::new()
    }
}

fn run(rx: Receiver<String>, flag: Arc<AtomicBool>, rev: Arc<AtomicU64>, wake_slot: Wake) {
    let mut wait = Duration::from_secs(1);
    let mut last = String::new();
    loop {
        let arrived = match rx.recv_timeout(wait) {
            Ok(url) => url,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last.is_empty() {
                    continue;
                }
                last.clone()
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let mut url = arrived;
        while let Ok(newer) = rx.try_recv() {
            url = newer;
        }
        last = url.clone();
        flag.store(false, Ordering::Relaxed);
        if connect_once(&url, &flag, &rev, &wake_slot).is_ok() {
            wait = Duration::from_secs(1);
        } else {
            wait = (wait * 2).min(Duration::from_secs(30));
        }
        flag.store(false, Ordering::Relaxed);
        // The status leaves Live, and the window goes back to asking.
        wake(&wake_slot);
    }
}

fn connect_once(url: &str, flag: &AtomicBool, rev: &AtomicU64, wake_slot: &Wake) -> Result<(), ()> {
    let (mut socket, _) = connect(url).map_err(|_| ())?;
    flag.store(true, Ordering::Relaxed);
    wake(wake_slot);
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                if note_rev(&text, rev) {
                    wake(wake_slot);
                }
            }
            Ok(Message::Binary(bytes)) => {
                if let Ok(text) = String::from_utf8(bytes.to_vec())
                    && note_rev(&text, rev)
                {
                    wake(wake_slot);
                }
            }
            Ok(Message::Ping(payload)) => {
                let _ = socket.send(Message::Pong(payload));
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(_) => return Err(()),
        }
    }
}

/// True when the message was a revision notice.
fn note_rev(text: &str, rev: &AtomicU64) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    // A notice is a generation number. The sealed library is a separate pull.
    if value.get("record").is_some() || value.get("objects").is_some() {
        return false;
    }
    if let Some(n) = value.get("rev").and_then(|item| item.as_u64()) {
        rev.store(n, Ordering::Relaxed);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window is asleep between frames. A notice that only stored a
    /// number sat there until the fallback poll, half a minute later.
    #[test]
    fn a_notice_wakes_the_window() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let host = thread::spawn(move || {
            let (stream, _) = server.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            socket.send(Message::text(r#"{"rev":7}"#)).unwrap();
            // Hold the socket until the device has heard the notice.
            let _ = socket.read();
        });
        let live = Live::new();
        let (tx, woke) = mpsc::channel();
        live.on_notice(move || {
            let _ = tx.send(());
        });
        live.offer(format!("ws://127.0.0.1:{port}/live"));
        let limit = Duration::from_secs(20);
        let start = std::time::Instant::now();
        while live.revision() != 7 {
            woke.recv_timeout(limit.saturating_sub(start.elapsed()))
                .expect("the notice did not wake the window");
        }
        assert!(live.connected());
        drop(live);
        drop(host);
    }

    #[test]
    fn an_edit_waits_under_half_a_second_and_a_live_socket_does_not_poll() {
        assert_eq!(PUSH_DEBOUNCE_MS, 400);
        assert_eq!(live_delivery_ms(), 400);
        assert_eq!(poll_gap_ms(true, true, true), 800);
        assert_eq!(poll_gap_ms(false, true, true), 400);
        assert_eq!(poll_gap_ms(false, false, true), 30_000);
        assert_eq!(poll_gap_ms(false, false, false), 2_000);
    }

    #[test]
    fn a_waiting_join_keeps_asking_until_it_is_approved() {
        assert!(should_poll_sync(false, false, true));
        assert!(should_poll_sync(true, true, false));
        assert!(
            should_poll_sync(true, false, false),
            "an open sync sheet still connects"
        );
        assert!(!should_poll_sync(false, false, false));
    }

    #[test]
    fn a_missing_join_expires_and_a_refusal_stays_a_refusal() {
        assert_eq!(
            join_status(Some("No pages for that yet."), "pending"),
            "expired"
        );
        assert_eq!(join_status(None, "pending"), "open");
        assert_eq!(join_status(None, "approved"), "open");
        assert_eq!(join_status(None, "rejected"), "rejected");
        assert_eq!(join_status(None, ""), "expired");
        assert_eq!(join_status(None, "released"), "expired");
    }
}

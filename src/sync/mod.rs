//! Sync in this window.
//!
//! The pages stay plain text in the data folder. What leaves is a sealed
//! record. The server, and whoever hosts it, never receives the vault key,
//! the device keys, the recovery words, or the page text.

mod drive;
mod keys;
mod live;
mod qr;
mod session;
pub mod settings_preview;
mod ui;
mod wire;

pub use drive::e2e_drive;
pub use keys::e2e_store;
pub use live::{poll_gap_ms, should_poll_sync};
pub use session::{Chip, LocalPages, Screen, Session, Work, intro_lines, release_clipboard};
pub use ui::show;

#[cfg(test)]
mod parity;

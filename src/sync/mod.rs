//! Sync in this window.
//!
//! The pages stay plain text in the data folder. What leaves is a sealed
//! record. The server, and whoever hosts it, never receives the vault key,
//! the device keys, the recovery words, or the page text.

mod keys;
mod qr;
mod session;
mod ui;
mod wire;

pub use session::{Chip, LocalPages, Screen, Session, intro_lines, release_clipboard};
pub use ui::show;

#[cfg(test)]
mod parity;

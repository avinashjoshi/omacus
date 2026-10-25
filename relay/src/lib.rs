//! A mailbox for the private sync test.
//!
//! The bytes that land here are sealed records, public device bytes, and the
//! sealed copy made for an approved device. This crate has no way to open a page.

pub mod access;
pub mod adapter;
pub mod auth;
pub mod http;
pub mod invite;
pub mod store;

pub const LABEL: &str = "1.0.0-beta.21";

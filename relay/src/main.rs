//! One process for the private sync test: the mailbox and the test pages.

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use omacus_relay::LABEL;
use omacus_relay::http;
use omacus_relay::store::Store;

fn main() {
    let port = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(8787u16);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let www = root.join("www");
    let site = root.join("../site");
    if !www.join("pkg/omacus_sync_web.js").is_file() {
        eprintln!("The test page is not built. From the repository, run: ./relay/run");
        std::process::exit(1);
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|_| {
        eprintln!("Port {port} is already in use.");
        std::process::exit(1);
    });
    println!("Omacus sync test {LABEL}");
    println!("Open http://127.0.0.1:{port}");
    println!("This server stores sealed pages. It cannot open them.");
    let mut store = Store::new();
    if let Ok(secret) = std::env::var("ACCESS_SECRET") {
        store.set_access_secret(secret.as_bytes());
    }
    http::serve(listener, www, site, Arc::new(Mutex::new(store)));
}

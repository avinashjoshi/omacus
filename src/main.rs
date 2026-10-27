// On Windows, don't open a console window behind the GUI. CLI use reattaches below.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod backup;
mod calc;
mod cli;
mod design;
mod help;
mod highlight;
mod library;
mod links;
mod lock;
mod mcp;
mod pdf;
mod preview;
mod rates;
mod settings;
mod sheets;
mod sync;
mod theme;
mod update;

fn main() {
    if std::env::var("OMACUS_E2E_SYNC").ok().as_deref() == Some("1") {
        std::process::exit(sync::e2e_drive());
    }
    if std::env::var("OMACUS_E2E_KEYRING").ok().as_deref() == Some("1") {
        let dir = std::env::var("OMACUS_E2E_DIR").unwrap_or_else(|_| {
            std::env::temp_dir()
                .join("omacus-e2e-key")
                .display()
                .to_string()
        });
        let code = sync::e2e_store(std::path::Path::new(&dir));
        std::process::exit(code);
    }
    update::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return run_app();
    }
    // A share link on its own (or one the OS hands over) opens the window.
    // `omacus open <link>` is handled with the other commands, after a console
    // is attached, so a bad link can be reported.
    if args.len() == 1
        && let Some(text) = links::from_arg(&args[0])
    {
        links::push(text);
        return run_app();
    }
    attach_console();
    match cli::run(&args) {
        cli::Outcome::Launch => run_app(),
        cli::Outcome::Exit(code) => std::process::exit(code),
    }
}

fn run_app() {
    links::register();
    if let Err(e) = app::run() {
        eprintln!("omacus: {e}");
        show_error(&format!("Omacus couldn't open its window.\n\n{e}"));
        std::process::exit(1);
    }
}

/// Started from Start or Explorer there's no terminal to print to, so say it in a box.
#[cfg(windows)]
fn show_error(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("Omacus"));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_error(_text: &str) {}

/// A GUI-subsystem exe has no console; borrow the terminal it was started from.
#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: plain Win32 call with no pointers; failure just means no console.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}

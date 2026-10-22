// On Windows, don't open a console window behind the GUI. CLI use reattaches below.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod calc;
mod help;
mod highlight;
mod links;
mod rates;
mod settings;
mod sheets;
mod theme;
mod update;

use std::io::Read;

const HELP: &str = "\
omacus — a notepad calculator

USAGE:
    omacus                 open the calculator window
    omacus <expression>    evaluate one expression and print the result
    cat page | omacus -    evaluate a page from stdin, one result per line
    omacus open <link>     open a share link (omacus.com/app#…) as a page
    omacus update          install the latest version
    omacus --version       print the version
";

fn main() {
    update::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // A share link, from `omacus open <link>` or the OS opening an omacus:// link.
    let link = match args.as_slice() {
        [cmd, link] if cmd == "open" => Some(link),
        [link] => Some(link),
        _ => None,
    };
    if let Some(text) = link.and_then(|l| links::from_arg(l)) {
        links::push(text);
        return run_app();
    }
    if args.first().is_some_and(|a| a == "open") {
        attach_console();
        eprintln!("omacus: that isn't an Omacus share link");
        std::process::exit(1);
    }
    if !args.is_empty() {
        attach_console();
    }
    match args.first().map(String::as_str) {
        Some("-h" | "--help") => print!("{HELP}"),
        Some("-V" | "--version") => println!("omacus {}", env!("CARGO_PKG_VERSION")),
        Some("update" | "--update") if args.len() == 1 => std::process::exit(update::cli()),
        Some("help" | "version") if args.len() == 1 => {
            if args[0] == "help" {
                print!("{HELP}");
            } else {
                println!("omacus {}", env!("CARGO_PKG_VERSION"));
            }
        }
        Some("-") => run_stdin(),
        Some(_) => run_expr(&args.join(" ")),
        None => run_app(),
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

fn cli_engine() -> omacus_engine::Engine {
    omacus_engine::Engine::new(rates::Rates::load_blocking().table())
        .with_clock(omacus_engine::monotonic_ms)
}

fn run_expr(expr: &str) {
    match cli_engine().eval_line(expr) {
        Some(r) => println!("{r}"),
        None => std::process::exit(1),
    }
}

fn run_stdin() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    for r in cli_engine().eval_document(input.trim_end_matches('\n')) {
        println!("{}", r.display);
    }
}

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

#[cfg(test)]
mod tests {
    #[test]
    fn cli_help_says_page() {
        assert!(
            !super::HELP.to_lowercase().contains("sheet"),
            "{}",
            super::HELP
        );
    }
}

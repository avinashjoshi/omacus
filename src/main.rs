// On Windows, don't open a console window behind the GUI. CLI use reattaches below.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod complete;
mod engine;
mod help;
mod highlight;
mod rates;
mod sheets;
mod theme;
mod time;

use std::io::Read;

const HELP: &str = "\
omacus — a notepad calculator

USAGE:
    omacus                 open the calculator window
    omacus <expression>    evaluate one expression and print the result
    cat sheet | omacus -   evaluate a sheet from stdin, one result per line
    omacus --version       print the version
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        attach_console();
    }
    match args.first().map(String::as_str) {
        Some("-h" | "--help") => print!("{HELP}"),
        Some("-V" | "--version") => println!("omacus {}", env!("CARGO_PKG_VERSION")),
        Some("help" | "version") if args.len() == 1 => {
            if args[0] == "help" {
                print!("{HELP}");
            } else {
                println!("omacus {}", env!("CARGO_PKG_VERSION"));
            }
        }
        Some("-") => run_stdin(),
        Some(_) => run_expr(&args.join(" ")),
        None => {
            if let Err(e) = app::run() {
                eprintln!("omacus: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn cli_engine() -> engine::Engine {
    engine::Engine::new(rates::Rates::load_blocking())
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

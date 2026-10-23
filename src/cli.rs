//! `omacus` with no arguments opens the window. Subcommands share that window's pages.

use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use clap::{ArgGroup, Args, CommandFactory, Parser, Subcommand, ValueEnum};

use crate::library::{self, Library, PageInfo};
use crate::mcp;

const EXAMPLES: &str = "\
EXAMPLES:
  omacus                                 Open the window
  omacus '3.5 km in miles'               One expression (same as eval)
  omacus eval '3.5 km in miles'
  omacus eval --json '2 + 2'
  printf 'a = 5\\na * 3\\n' | omacus eval   A whole page, one answer per line
  printf 'a = 5\\na * 3\\n' | omacus -
  omacus pages list
  omacus pages show 'Trip Budget'
  omacus pages new 'Weekend Plans'
  omacus pages new Groceries --from-file list.txt
  omacus pages append Groceries 'milk $3'
  omacus pages rm 'Weekend Plans'
  omacus pages export 'Trip Budget' --md
  omacus pages export 'Trip Budget' --pdf
  omacus mcp                             Model Context Protocol server, on stdio
  omacus completions bash

A page name can be its title, its id, or a unique prefix. If more than one page
fits, omacus lists them and changes nothing. A locked page is not printed or
changed without --password.

The open window re-reads pages every few seconds, and again when it regains
focus, so an edit from the command line or the MCP server shows up there.
";

#[derive(Parser)]
#[command(
    name = "omacus",
    version,
    about = "A notepad calculator: units, currencies, time zones and plain words",
    after_long_help = EXAMPLES,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Evaluate an expression, or a whole page
    Eval(EvalArgs),
    /// List, show, create, change and export pages
    #[command(subcommand_required = true, arg_required_else_help = true)]
    Pages {
        #[command(subcommand)]
        cmd: PagesCmd,
    },
    /// Run a Model Context Protocol server on stdin and stdout
    Mcp,
    /// Open a share link in the window
    Open {
        /// An omacus.com/app#… or omacus:// link
        link: String,
    },
    /// Install the latest version
    Update,
    /// Print a shell completion script
    Completions {
        /// bash, zsh, fish or powershell
        shell: ShellName,
    },
}

#[derive(Args)]
struct EvalArgs {
    /// Print JSON, one entry per line
    #[arg(long)]
    json: bool,
    /// The expression or page. Leave this out, or pass "-", to read stdin.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    expr: Vec<String>,
}

#[derive(Subcommand)]
enum PagesCmd {
    /// List pages
    List {
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Show a page with its answers
    Show {
        /// Title, id, or a unique prefix
        name: String,
        /// Print JSON (the text and each answer)
        #[arg(long)]
        json: bool,
        /// Password, when the page is locked
        #[arg(long)]
        password: Option<String>,
    },
    /// Create a page
    New {
        /// The page title
        name: String,
        /// Read the body from this file ("-" reads stdin)
        #[arg(long, value_name = "PATH")]
        from_file: Option<PathBuf>,
    },
    /// Add lines to a page
    Append {
        /// Title, id, or a unique prefix
        name: String,
        /// Password, when the page is locked
        #[arg(long)]
        password: Option<String>,
        /// The line to add. Separate words are joined.
        #[arg(trailing_var_arg = true, required = true, allow_hyphen_values = true)]
        text: Vec<String>,
    },
    /// Move a page to Recently deleted
    Rm {
        /// Title, id, or a unique prefix
        name: String,
        /// Password, when the page is locked
        #[arg(long)]
        password: Option<String>,
    },
    /// Export a page as Markdown or PDF
    #[command(group(ArgGroup::new("format").required(true).args(["md", "pdf"])))]
    Export {
        /// Title, id, or a unique prefix
        name: String,
        /// Write a Markdown table to stdout
        #[arg(long)]
        md: bool,
        /// Write a PDF
        #[arg(long)]
        pdf: bool,
        /// PDF file, or a folder (the page title names the file)
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Overwrite an existing PDF
        #[arg(long)]
        replace: bool,
        /// Password, when the page is locked
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ShellName {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

/// What `main` should do after the command line has been read.
pub enum Outcome {
    /// Open the calculator window.
    Launch,
    Exit(i32),
}

pub fn run(args: &[String]) -> Outcome {
    if args == ["-"] {
        return Outcome::Exit(eval_text(&read_stdin(), false));
    }
    if !is_command(&args[0]) {
        return Outcome::Exit(eval_text(&args.join(" "), false));
    }
    let args = normalize(args);
    let cli = match Cli::try_parse_from(std::iter::once("omacus".to_string()).chain(args)) {
        Ok(cli) => cli,
        Err(err) => {
            let code = err.exit_code();
            let _ = err.print();
            return Outcome::Exit(code);
        }
    };
    match cli.command {
        Command::Eval(args) => Outcome::Exit(eval_cmd(args)),
        Command::Pages { cmd } => Outcome::Exit(pages_cmd(cmd)),
        Command::Mcp => Outcome::Exit(mcp::serve()),
        Command::Open { link } => match crate::links::from_arg(&link) {
            Some(text) => {
                crate::links::push(text);
                Outcome::Launch
            }
            None => {
                eprintln!("omacus: that isn't an Omacus share link");
                Outcome::Exit(1)
            }
        },
        Command::Update => Outcome::Exit(crate::update::cli()),
        Command::Completions { shell } => {
            print_completions(shell);
            Outcome::Exit(0)
        }
    }
}

fn is_command(arg: &str) -> bool {
    matches!(
        arg,
        "eval"
            | "pages"
            | "mcp"
            | "open"
            | "update"
            | "completions"
            | "help"
            | "version"
            | "-h"
            | "--help"
            | "-V"
            | "--version"
            | "--update"
    )
}

fn normalize(args: &[String]) -> Vec<String> {
    let mut args = args.to_vec();
    if args.first().map(String::as_str) == Some("help") {
        args.remove(0);
        args.push("--help".into());
    }
    if args.first().map(String::as_str) == Some("version") {
        args[0] = "--version".into();
    }
    if args.first().map(String::as_str) == Some("--update") {
        args[0] = "update".into();
    }
    args
}

fn eval_cmd(args: EvalArgs) -> i32 {
    let text = if args.expr.is_empty() || args.expr == ["-"] {
        if args.expr.is_empty() && std::io::stdin().is_terminal() {
            eprintln!("omacus: pass an expression, or pipe a page to `omacus eval`");
            return 1;
        }
        read_stdin()
    } else {
        args.expr.join(" ")
    };
    eval_text(&text, args.json)
}

fn eval_text(text: &str, json: bool) -> i32 {
    let lines = library::evaluate(text);
    let code = library::eval_status(text, &lines);
    if json {
        println!("{}", serde_json::to_string_pretty(&lines).unwrap());
        return code;
    }
    if code == 1 {
        return 1;
    }
    for line in &lines {
        println!("{}", line.answer);
    }
    0
}

fn read_stdin() -> String {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    input
}

fn pages_cmd(cmd: PagesCmd) -> i32 {
    match cmd {
        PagesCmd::List { json } => {
            let pages = Library::open().list();
            print_pages(&pages, json)
        }
        PagesCmd::Show {
            name,
            json,
            password,
        } => match Library::open().show(&name, password.as_deref()) {
            Ok(view) => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&view).unwrap());
                } else {
                    println!("{}", view.rendered);
                }
                0
            }
            Err(err) => fail(err),
        },
        PagesCmd::New { name, from_file } => {
            let body = match from_file.as_deref().map(read_file) {
                Some(Ok(text)) => Some(text),
                Some(Err(err)) => return fail(err),
                None => None,
            };
            match Library::open().create(&name, body.as_deref()) {
                Ok(page) => {
                    println!("{}  {}", page.id, page.title);
                    0
                }
                Err(err) => fail(err),
            }
        }
        PagesCmd::Append {
            name,
            password,
            text,
        } => match Library::open().append(&name, &text.join(" "), password.as_deref()) {
            Ok(page) => {
                println!("{}  {}", page.id, page.title);
                0
            }
            Err(err) => fail(err),
        },
        PagesCmd::Rm { name, password } => {
            match Library::open().remove(&name, password.as_deref()) {
                Ok(title) => {
                    println!("Moved \"{title}\" to Recently deleted");
                    0
                }
                Err(err) => fail(err),
            }
        }
        PagesCmd::Export {
            name,
            md,
            pdf,
            out,
            replace,
            password,
        } => {
            let lib = Library::open();
            if md {
                return match lib.export_md(&name, password.as_deref()) {
                    Ok(text) => {
                        println!("{text}");
                        0
                    }
                    Err(err) => fail(err),
                };
            }
            if pdf {
                return match lib.export_pdf(&name, out.as_deref(), replace, password.as_deref()) {
                    Ok(path) => {
                        println!("{}", path.display());
                        0
                    }
                    Err(err) => fail(err),
                };
            }
            fail("pass --md or --pdf".into())
        }
    }
}

fn print_pages(pages: &[PageInfo], json: bool) -> i32 {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "pages": pages })).unwrap()
        );
        return 0;
    }
    let width = pages
        .iter()
        .map(|p| p.id.chars().count())
        .max()
        .unwrap_or(0);
    for page in pages {
        let locked = if page.locked { "  locked" } else { "" };
        println!("{:<width$}  {}{locked}", page.id, page.title, width = width);
    }
    0
}

fn read_file(path: &std::path::Path) -> Result<String, String> {
    if path.as_os_str() == "-" {
        return Ok(read_stdin());
    }
    std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))
}

fn fail(message: String) -> i32 {
    eprintln!("omacus: {message}");
    1
}

fn print_completions(shell: ShellName) {
    let clap = match shell {
        ShellName::Bash => clap_complete::Shell::Bash,
        ShellName::Zsh => clap_complete::Shell::Zsh,
        ShellName::Fish => clap_complete::Shell::Fish,
        ShellName::Powershell => clap_complete::Shell::PowerShell,
    };
    let mut cmd = Cli::command();
    let mut out = std::io::stdout().lock();
    clap_complete::generate(clap, &mut cmd, "omacus", &mut out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_says_page_and_not_sheet() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("page"), "{help}");
        assert!(!help.to_lowercase().contains("sheet"), "{help}");
        assert!(help.contains("omacus eval"));
        assert!(help.contains("omacus mcp"));
    }

    #[test]
    fn bash_completions_name_the_commands() {
        let mut cmd = Cli::command();
        let mut buf = Vec::new();
        clap_complete::generate(clap_complete::Shell::Bash, &mut cmd, "omacus", &mut buf);
        let text = String::from_utf8(buf).unwrap();
        for word in ["eval", "pages", "mcp", "completions"] {
            assert!(text.contains(word), "missing {word}");
        }
    }
}

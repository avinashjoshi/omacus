//! Command-line evaluation and page commands. Uses a temp library so the
//! developer's own pages are never touched.

use std::path::Path;
use std::process::{Command, Output};

fn dir(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("omacus-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn omacus(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omacus"))
        .args(args)
        .env("OMACUS_OFFLINE", "1")
        .env("OMACUS_DATA_DIR", root)
        .output()
        .unwrap()
}

fn omacus_in(root: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_omacus"))
        .args(args)
        .env("OMACUS_OFFLINE", "1")
        .env("OMACUS_DATA_DIR", root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn eval_prints_the_answer_and_json() {
    let root = dir("eval");
    let out = omacus(&root, &["eval", "3.5 km in miles"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(text(&out), "2.17 miles");

    let out = omacus(&root, &["3.5", "km", "in", "miles"]);
    assert_eq!(text(&out), "2.17 miles");

    let out = omacus(&root, &["eval", "--json", "2 + 2"]);
    assert!(out.status.success());
    let lines: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(lines[0]["answer"], "4");
    assert_eq!(lines[0]["kind"], "value");
    assert_eq!(lines[0]["text"], "2 + 2");

    let out = omacus(&root, &["eval", "hello"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "{}", text(&out));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn eval_reads_a_page_from_stdin() {
    let root = dir("stdin");
    let page = "a = 5\na * 3\n";
    let out = omacus_in(&root, &["eval"], page);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(text(&out), "5\n15");

    let out = omacus_in(&root, &["-"], page);
    assert_eq!(text(&out), "5\n15");

    let out = omacus_in(&root, &["eval", "--json"], page);
    let lines: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["answer"], "15");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pages_round_trip_and_fuzzy_names() {
    let root = dir("pages");
    let created = omacus(&root, &["pages", "new", "Trip Budget"]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    assert!(text(&created).contains("Trip Budget"));

    let listed = omacus(&root, &["pages", "list"]);
    assert!(text(&listed).contains("Trip Budget"), "{}", text(&listed));

    let appended = omacus(&root, &["pages", "append", "trip", "train $40"]);
    assert!(
        appended.status.success(),
        "{}",
        String::from_utf8_lossy(&appended.stderr)
    );

    let shown = omacus(&root, &["pages", "show", "Trip"]);
    assert!(text(&shown).contains("train"), "{}", text(&shown));
    assert!(text(&shown).contains("40"), "{}", text(&shown));

    let groceries = omacus(&root, &["pages", "new", "Groceries"]);
    assert!(
        groceries.status.success(),
        "{}",
        String::from_utf8_lossy(&groceries.stderr)
    );
    let grocery = omacus(&root, &["pages", "new", "Grocery run"]);
    assert!(grocery.status.success());
    let ambiguous = omacus(&root, &["pages", "show", "gro"]);
    assert!(!ambiguous.status.success());
    let err = String::from_utf8_lossy(&ambiguous.stderr);
    assert!(err.contains("more than one page"), "{err}");
    assert!(
        err.contains("Groceries") && err.contains("Grocery run"),
        "{err}"
    );

    let md = omacus(&root, &["pages", "export", "trip", "--md"]);
    assert!(
        md.status.success(),
        "{}",
        String::from_utf8_lossy(&md.stderr)
    );
    assert!(text(&md).contains("train"), "{}", text(&md));

    let pdf = root.join("Trip Budget.pdf");
    let exported = omacus(
        &root,
        &[
            "pages",
            "export",
            "Trip Budget",
            "--pdf",
            "--out",
            pdf.to_str().unwrap(),
        ],
    );
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let bytes = std::fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    let again = omacus(
        &root,
        &[
            "pages",
            "export",
            "trip",
            "--pdf",
            "--out",
            pdf.to_str().unwrap(),
        ],
    );
    assert!(!again.status.success());

    let removed = omacus(&root, &["pages", "rm", "trip"]);
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(text(&removed).contains("Recently deleted"));
    let missing = omacus(&root, &["pages", "show", "Trip Budget"]);
    assert!(!missing.status.success());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_locked_page_is_not_printed() {
    let root = dir("locked");
    let body = root.join("plans.txt");
    std::fs::write(&body, "cabin $400\n").unwrap();
    let created = omacus(
        &root,
        &[
            "pages",
            "new",
            "Weekend Plans",
            "--from-file",
            body.to_str().unwrap(),
        ],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let id = text(&created)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let path = root.join("pages").join(format!("{id}.txt"));
    let plain = std::fs::read_to_string(&path).unwrap();
    assert!(plain.contains("cabin $400"), "{plain}");
    let (key, wrap) = omacus_engine::lock::create("open-sesame").unwrap();
    let sealed = omacus_engine::lock::seal(&key, &plain, &wrap, id.as_bytes()).unwrap();
    std::fs::write(&path, &sealed).unwrap();
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.starts_with("OMACUS-LOCK/1\n"), "{on_disk}");
    assert!(
        !on_disk.contains("cabin") && !on_disk.contains("Weekend"),
        "{on_disk}"
    );
    // A sidecar or a password string in state.json must not open the page.
    std::fs::write(
        root.join("pages").join(format!("{id}.lock")),
        "\"side-secret\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("state.json"),
        format!(r#"{{"locks":{{"{id}":"side-secret"}}}}"#),
    )
    .unwrap();

    let hidden = omacus(&root, &["pages", "show", &id]);
    assert!(!hidden.status.success());
    let err = String::from_utf8_lossy(&hidden.stderr);
    let shown = String::from_utf8_lossy(&hidden.stdout);
    assert!(err.contains("locked"), "{err}");
    assert!(
        !err.contains("cabin") && !shown.contains("cabin") && !shown.contains("OMACUS-LOCK"),
        "{err} {shown}"
    );

    let listed = omacus(&root, &["pages", "list"]);
    let listed = text(&listed);
    assert!(listed.contains("locked"), "{listed}");
    assert!(
        !listed.contains("cabin") && !listed.contains("Weekend"),
        "{listed}"
    );

    let decoy = omacus(
        &root,
        &["pages", "show", "--password", "side-secret", "Weekend"],
    );
    assert!(!decoy.status.success());
    let decoy_out = format!(
        "{} {}",
        String::from_utf8_lossy(&decoy.stderr),
        String::from_utf8_lossy(&decoy.stdout)
    );
    assert!(
        !decoy_out.contains("cabin") && !decoy_out.contains("OMACUS-LOCK"),
        "{decoy_out}"
    );

    let opened = omacus(
        &root,
        &["pages", "show", "--password", "open-sesame", "Weekend"],
    );
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    assert!(text(&opened).contains("cabin $400"), "{}", text(&opened));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn help_and_version_and_completions() {
    let root = dir("help");
    let help = omacus(&root, &["--help"]);
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("omacus eval"));
    assert!(help.contains("omacus mcp"));
    assert!(!help.to_lowercase().contains("sheet"), "{help}");

    let version = omacus(&root, &["--version"]);
    assert!(text(&version).starts_with("omacus "));

    for shell in ["bash", "zsh", "fish"] {
        let script = omacus(&root, &["completions", shell]);
        assert!(script.status.success(), "{shell}");
        let script = String::from_utf8_lossy(&script.stdout);
        assert!(
            script.contains("eval") && script.contains("pages"),
            "{shell}"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

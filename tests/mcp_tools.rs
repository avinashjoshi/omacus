//! The MCP server, spoken to the way a client does: newline-delimited JSON-RPC.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

fn dir(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("omacus-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

struct Client {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: std::sync::mpsc::Receiver<String>,
}

impl Client {
    fn start(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_omacus"))
            .arg("mcp")
            .env("OMACUS_OFFLINE", "1")
            .env("OMACUS_DATA_DIR", root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
        }
    }

    fn send(&mut self, message: &serde_json::Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, id: u64, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                panic!("timed out waiting for {method}");
            }
            let line = self
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("server closed waiting for {method}"));
            let value: serde_json::Value = serde_json::from_str(&line).unwrap_or_else(|err| {
                panic!("not json ({err}): {line}");
            });
            if value.get("id").and_then(|id| id.as_u64()) == Some(id)
                && value.get("method").is_none()
            {
                return value;
            }
            // A server request (it has a method and an id). Answer so it can continue.
            if value.get("method").is_some() && value.get("id").is_some() {
                let reply_id = value["id"].clone();
                self.send(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": reply_id,
                    "result": {},
                }));
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tool_text(response: &serde_json::Value) -> String {
    let content = &response["result"]["content"];
    content
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["text"].as_str())
        .unwrap_or("")
        .to_string()
}

#[test]
fn tools_evaluate_and_read_pages() {
    let root = dir("tools");
    let mut client = Client::start(&root);
    let init = client.request(
        1,
        "initialize",
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "experimental": null,
                "extensions": null,
                "roots": null,
                "sampling": null,
                "elicitation": null
            },
            "clientInfo": {
                "name": "omacus-test",
                "title": null,
                "version": "0",
                "description": null,
                "icons": null,
                "websiteUrl": null
            }
        }),
    );
    assert!(init.get("error").is_none(), "{init}");
    assert_eq!(init["result"]["serverInfo"]["name"], "omacus");
    client.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    let tools = client.request(2, "tools/list", serde_json::json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for name in [
        "evaluate",
        "list_pages",
        "read_page",
        "create_page",
        "append_page",
        "update_page",
        "delete_page",
    ] {
        assert!(names.contains(&name), "missing {name} in {names:?}");
    }

    let eval = client.request(
        3,
        "tools/call",
        serde_json::json!({
            "name": "evaluate",
            "arguments": { "text": "3.5 km in miles" }
        }),
    );
    let body = tool_text(&eval);
    assert!(body.contains("2.17 miles"), "{eval}");
    assert_eq!(eval["result"]["isError"], false);

    let created = client.request(
        4,
        "tools/call",
        serde_json::json!({
            "name": "create_page",
            "arguments": { "name": "Groceries", "text": "milk $3" }
        }),
    );
    assert_eq!(created["result"]["isError"], false, "{created}");
    let created_body: serde_json::Value = serde_json::from_str(&tool_text(&created)).unwrap();
    let id = created_body["id"].as_str().unwrap();

    let read = client.request(
        5,
        "tools/call",
        serde_json::json!({
            "name": "read_page",
            "arguments": { "name": "groc" }
        }),
    );
    let page = tool_text(&read);
    assert!(page.contains("milk"), "{page}");
    assert!(page.contains('3'), "{page}");

    let resources = client.request(6, "resources/list", serde_json::json!({}));
    let uris = resources["result"]["resources"].to_string();
    assert!(uris.contains(&format!("omacus://page/{id}")), "{resources}");

    let resource = client.request(
        7,
        "resources/read",
        serde_json::json!({ "uri": format!("omacus://page/{id}") }),
    );
    let got = resource.to_string();
    assert!(got.contains("milk"), "{resource}");

    std::fs::write(
        root.join("pages").join(format!("{id}.lock")),
        "\"open-sesame\"\n",
    )
    .unwrap();
    let hidden = client.request(
        8,
        "tools/call",
        serde_json::json!({
            "name": "read_page",
            "arguments": { "name": "Groceries" }
        }),
    );
    let hidden_text = tool_text(&hidden);
    assert_eq!(hidden["result"]["isError"], true, "{hidden}");
    assert!(!hidden_text.contains("milk"), "{hidden_text}");
    assert!(hidden_text.contains("locked"), "{hidden_text}");

    let opened = client.request(
        9,
        "tools/call",
        serde_json::json!({
            "name": "read_page",
            "arguments": { "name": "Groceries", "password": "open-sesame" }
        }),
    );
    assert!(tool_text(&opened).contains("milk $3"), "{opened}");
    let _ = std::fs::remove_dir_all(root);
}

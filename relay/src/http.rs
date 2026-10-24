//! A small HTTP mailbox. Bodies that are not sealed records are refused
//! and not kept.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};

use crate::LABEL;
use crate::store::{ObjectWrite, Store, StoreError};

pub fn serve(listener: TcpListener, www: PathBuf, site: PathBuf, store: Arc<Mutex<Store>>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let www = www.clone();
        let site = site.clone();
        let store = Arc::clone(&store);
        std::thread::spawn(move || {
            let _ = handle(stream, &www, &site, &store);
        });
    }
}

fn handle(
    mut stream: TcpStream,
    www: &Path,
    site: &Path,
    store: &Mutex<Store>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let head_end = loop {
        if buf.len() > 64 * 1024 {
            return Ok(());
        }
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(end) = find_header(&buf) {
            break end;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end.saturating_sub(4)]).to_string();
    let mut lines = head.split("\r\n");
    let Some(request) = lines.next() else {
        return Ok(());
    };
    let mut parts = request.split(' ');
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse().unwrap_or(usize::MAX);
        }
    }
    if length > 8 * 1024 * 1024 {
        return write_json(
            &mut stream,
            400,
            &json!({"message": "That copy could not be read."}),
        );
    }
    while buf.len() < head_end + length {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    if buf.len() < head_end + length {
        return Ok(());
    }
    let body = buf[head_end..head_end + length].to_vec();
    let path = target.split('?').next().unwrap_or("/");
    if method == "OPTIONS" {
        return write_bytes(&mut stream, 204, "No Content", "text/plain", b"", true);
    }
    if path.starts_with("/v1/") {
        let (status, payload) = api(method, path, &body, store);
        return write_json(&mut stream, status, &payload);
    }
    serve_file(&mut stream, www, site, path)
}

fn api(method: &str, path: &str, body: &[u8], store: &Mutex<Store>) -> (u16, Value) {
    let Ok(mut store) = store.lock() else {
        return (500, json!({"message": "Try again in a moment."}));
    };
    match route(method, path, body, &mut store) {
        Ok((status, value)) => (status, value),
        Err(error) => (error.status(), json!({"message": error.message()})),
    }
}

fn route(
    method: &str,
    path: &str,
    body: &[u8],
    store: &mut Store,
) -> Result<(u16, Value), StoreError> {
    if method == "GET" && path == "/v1/health" {
        return Ok((200, json!({"label": LABEL})));
    }
    if method == "POST" && path == "/v1/join" {
        return accept_join(store, body);
    }
    if method == "POST" && path == "/v1/access" {
        let value = json_body(body)?;
        let code = value.get("code").and_then(Value::as_str).unwrap_or("");
        store.peek_access(code)?;
        return Ok((200, json!({"ok": true})));
    }
    if method == "POST" && path == "/v1/mailboxes" {
        return accept_create(store, body);
    }
    let Some(rest) = path.strip_prefix("/v1/mailboxes/") else {
        return Err(StoreError::Unknown);
    };
    let mut parts = rest.split('/');
    let account = parts.next().unwrap_or("");
    match (method, parts.next(), parts.next()) {
        ("GET", None, None) => {
            let (revision, record) = store.get(account)?;
            let objects = object_heads(store, account)?;
            Ok((
                200,
                json!({"revision": revision, "record": b64(&record), "objects": objects}),
            ))
        }
        ("PUT", None, None) => {
            let value = json_body(body)?;
            let revision = value
                .get("revision")
                .and_then(Value::as_u64)
                .ok_or(StoreError::Unreadable)?;
            let record = decode_field(&value, "record")?;
            let writes = object_writes(&value)?;
            let dropping = id_list(&value, "drop")?;
            if writes.is_empty() && dropping.is_empty() {
                let revision = store.put(account, revision, record)?;
                return Ok((200, json!({"revision": revision, "objects": []})));
            }
            let (revision, written) =
                store.put_objects(account, revision, record, writes, &dropping)?;
            Ok((
                200,
                json!({"revision": revision, "objects": written_rows(&written)}),
            ))
        }
        ("POST", Some("objects"), None) => {
            let value = json_body(body)?;
            let ids = id_list(&value, "ids")?;
            let rows = store.read_objects(account, &ids)?;
            let objects: Vec<_> = rows
                .into_iter()
                .map(|(id, revision, body)| json!({"id": id, "revision": revision, "body": b64(&body)}))
                .collect();
            Ok((200, json!({"objects": objects})))
        }
        ("GET", Some("requests"), None) => {
            let pending: Vec<_> = store
                .pending(account)?
                .into_iter()
                .map(|request| {
                    json!({
                        "id": request.id,
                        "device": b64(&request.device),
                        "name": request.name,
                        "proof": b64(&request.proof),
                    })
                })
                .collect();
            Ok((200, json!({"requests": pending})))
        }
        ("POST", Some("invites"), None) => {
            let value = json_body(body)?;
            let hash = value
                .get("hash")
                .and_then(Value::as_str)
                .ok_or(StoreError::Unreadable)?;
            store.open_invite(account, hash)?;
            Ok((200, json!({"ok": true})))
        }
        ("GET", Some("requests"), Some(id)) => {
            let request = store.request(account, id)?;
            Ok((
                200,
                json!({
                    "state": request.state,
                    "name": request.name,
                    "device": b64(&request.device),
                    "chain": b64(&request.chain),
                    "wrap": b64(&request.wrap),
                }),
            ))
        }
        ("POST", Some("requests"), Some(id)) => {
            let value = json_body(body)?;
            if value.get("reject").and_then(Value::as_bool) == Some(true) {
                store.reject(account, id)?;
            } else {
                let chain = decode_field(&value, "chain")?;
                let wrap = decode_field(&value, "wrap")?;
                store.approve(account, id, chain, wrap)?;
            }
            Ok((200, json!({"ok": true})))
        }
        _ => Err(StoreError::Unknown),
    }
}

fn accept_join(store: &mut Store, body: &[u8]) -> Result<(u16, Value), StoreError> {
    let value = json_body(body)?;
    let code = value.get("code").and_then(Value::as_str).unwrap_or("");
    if crate::invite::decode_code(code).is_none() {
        return Err(StoreError::BadInvite);
    }
    let device = decode_field(&value, "device")?;
    let name = value.get("name").and_then(Value::as_str).unwrap_or("");
    let proof = decode_field(&value, "proof")?;
    let (account, id) = store.join(code, device, name, &proof)?;
    Ok((201, json!({"id": id, "account": account})))
}

pub fn accept_create(store: &mut Store, body: &[u8]) -> Result<(u16, Value), StoreError> {
    let value = json_body(body)?;
    let account = value
        .get("account")
        .and_then(Value::as_str)
        .ok_or(StoreError::Unreadable)?;
    let record = decode_field(&value, "record")?;
    let code = value.get("code").and_then(Value::as_str).unwrap_or("");
    let writes = object_writes(&value)?;
    store.reserve(account, &record)?;
    store.take_access(code)?;
    if writes.is_empty() {
        let revision = store.create(account, record)?;
        return Ok((201, json!({"revision": revision, "objects": []})));
    }
    let (revision, written) = store.create_with(account, record, writes)?;
    Ok((
        201,
        json!({"revision": revision, "objects": written_rows(&written)}),
    ))
}

fn object_heads(store: &mut Store, account: &str) -> Result<Vec<Value>, StoreError> {
    Ok(store
        .object_index(account)?
        .into_iter()
        .map(|row| json!({"id": row.id, "revision": row.revision, "bytes": row.bytes}))
        .collect())
}

fn written_rows(rows: &[(String, u64)]) -> Vec<Value> {
    rows.iter()
        .map(|(id, revision)| json!({"id": id, "revision": revision}))
        .collect()
}

fn object_writes(value: &Value) -> Result<Vec<ObjectWrite>, StoreError> {
    let Some(list) = value.get("objects") else {
        return Ok(Vec::new());
    };
    let list = list.as_array().ok_or(StoreError::Unreadable)?;
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or(StoreError::Unreadable)?;
        let base = item
            .get("base")
            .and_then(Value::as_u64)
            .ok_or(StoreError::Unreadable)?;
        let body = decode_field(item, "body")?;
        out.push(ObjectWrite {
            id: id.to_string(),
            base,
            body,
        });
    }
    Ok(out)
}

fn id_list(value: &Value, name: &str) -> Result<Vec<String>, StoreError> {
    let Some(list) = value.get(name) else {
        return Ok(Vec::new());
    };
    let list = list.as_array().ok_or(StoreError::Unreadable)?;
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let id = item.as_str().ok_or(StoreError::Unreadable)?;
        out.push(id.to_string());
    }
    Ok(out)
}

fn json_body(body: &[u8]) -> Result<Value, StoreError> {
    serde_json::from_slice(body).map_err(|_| StoreError::Unreadable)
}

fn decode_field(value: &Value, name: &str) -> Result<Vec<u8>, StoreError> {
    let text = value
        .get(name)
        .and_then(Value::as_str)
        .ok_or(StoreError::Unreadable)?;
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|_| StoreError::Unreadable)
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn serve_file(stream: &mut TcpStream, www: &Path, site: &Path, path: &str) -> std::io::Result<()> {
    let rel = path.trim_start_matches('/');
    if rel.contains("..") || rel.contains('\\') {
        return write_json(
            stream,
            404,
            &json!({"message": "That copy could not be read."}),
        );
    }
    let file = if rel.is_empty() {
        www.join("index.html")
    } else if rel == "tokens.css" {
        site.join("css/tokens.css")
    } else if let Some(name) = rel.strip_prefix("fonts/") {
        site.join("fonts").join(name)
    } else if let Some(name) = rel.strip_prefix("img/") {
        site.join("img").join(name)
    } else {
        www.join(rel)
    };
    let Ok(bytes) = std::fs::read(&file) else {
        return write_json(
            stream,
            404,
            &json!({"message": "That copy could not be read."}),
        );
    };
    let kind = match file.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    };
    write_bytes(stream, 200, "OK", kind, &bytes, false)
}

fn write_json(stream: &mut TcpStream, status: u16, value: &Value) -> std::io::Result<()> {
    let reason = reason(status);
    let body = value.to_string();
    write_bytes(
        stream,
        status,
        reason,
        "application/json",
        body.as_bytes(),
        true,
    )
}

fn write_bytes(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
    cors: bool,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        body.len()
    );
    if cors {
        head.push_str(
            "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, PUT, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n",
        );
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        429 => "Too Many Requests",
        _ => "Error",
    }
}

fn find_header(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

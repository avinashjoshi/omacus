//! A small HTTP mailbox. Bodies that are not sealed records are refused
//! and not kept.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};

use crate::LABEL;
use crate::store::{ObjectWrite, Store, StoreError};

// Sync hosts never set a cookie, and they never take one as a login.
const CORS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, PUT, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, If-None-Match, If-Match, X-Omacus-Device, X-Omacus-Time, X-Omacus-Nonce, X-Omacus-Signature, X-Omacus-Proof\r\nAccess-Control-Expose-Headers: ETag\r\n";

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
    let mut headers = BTreeMap::new();
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if name == "content-length" {
            length = value.parse().unwrap_or(usize::MAX);
        }
        headers.insert(name, value);
    }
    let ip = stream
        .peer_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "0.0.0.0".to_string());
    if length > 8 * 1024 * 1024 {
        return write_json(
            &mut stream,
            400,
            &json!({"message": "That copy could not be read."}),
            None,
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
        return write_bytes(
            &mut stream,
            204,
            "No Content",
            "text/plain",
            b"",
            true,
            None,
        );
    }
    if path.starts_with("/v1/") {
        let (status, payload) = api(method, path, &body, &headers, &ip, store);
        if status == 304 {
            let revision = payload.get("revision").and_then(Value::as_u64).unwrap_or(0);
            return write_not_modified(&mut stream, &format!("\"{revision}\""));
        }
        let etag = mailbox_etag(&payload);
        return write_json(&mut stream, status, &payload, etag.as_deref());
    }
    serve_file(&mut stream, www, site, path)
}

fn api(
    method: &str,
    path: &str,
    body: &[u8],
    headers: &BTreeMap<String, String>,
    ip: &str,
    store: &Mutex<Store>,
) -> (u16, Value) {
    let Ok(mut store) = store.lock() else {
        return (500, json!({"message": "Try again in a moment."}));
    };
    match route(method, path, body, headers, ip, &mut store) {
        Ok((status, value)) => (status, value),
        Err(error) => (error.status(), json!({"message": error.message()})),
    }
}

fn route(
    method: &str,
    path: &str,
    body: &[u8],
    headers: &BTreeMap<String, String>,
    ip: &str,
    store: &mut Store,
) -> Result<(u16, Value), StoreError> {
    if method == "GET" && path == "/v1/health" {
        return Ok((200, json!({"label": LABEL})));
    }
    if method == "POST" && path == "/v1/join" {
        store.charge_ip(ip)?;
        store.charge_join(ip)?;
        return accept_join(store, body);
    }
    if method == "POST" && path == "/v1/access" {
        store.charge_ip(ip)?;
        store.charge_guess(ip)?;
        let value = json_body(body)?;
        let code = value.get("code").and_then(Value::as_str).unwrap_or("");
        store.peek_access(code)?;
        return Ok((200, json!({"ok": true})));
    }
    if method == "POST" && path == "/v1/mailboxes" {
        store.charge_ip(ip)?;
        return accept_create(store, body);
    }
    let Some(rest) = path.strip_prefix("/v1/mailboxes/") else {
        return Err(StoreError::Unknown);
    };
    let mut parts = rest.split('/');
    let account = parts.next().unwrap_or("");
    let sub = parts.next();
    let id = parts.next();
    if parts.next().is_some() {
        return Err(StoreError::Unknown);
    }
    store.charge_ip(ip)?;
    store.charge_account(account)?;
    if method == "GET"
        && sub == Some("requests")
        && let (Some(id), Some(proof)) = (id, crate::auth::read_proof(headers))
    {
        store.note_waiting(account, id, &proof)?;
    }
    if method == "GET" || (method == "POST" && sub == Some("objects") && id.is_none()) {
        store.charge_read(ip, account)?;
    }
    if let ("GET", Some("requests"), Some(id)) = (method, sub, id) {
        let proof = crate::auth::read_proof(headers).ok_or(StoreError::Unknown)?;
        let request = store.request_with_proof(account, id, &proof)?;
        return Ok((
            200,
            json!({
                "state": request.state,
                "name": request.name,
                "device": b64(&request.device),
                "chain": b64(&request.chain),
                "wrap": b64(&request.wrap),
            }),
        ));
    }
    let recovery = store.authorize(account, method, path, body, headers)?;
    match (method, sub, id) {
        ("GET", None, None) => {
            let (revision, record) = store.get(account)?;
            if same_revision(
                headers
                    .get("if-none-match")
                    .map(String::as_str)
                    .unwrap_or(""),
                revision,
            ) {
                return Ok((304, json!({"revision": revision})));
            }
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
            store.allow_put(account, &record, recovery)?;
            let (current, _) = store.get(account)?;
            if current != revision
                || !same_revision(
                    headers.get("if-match").map(String::as_str).unwrap_or(""),
                    current,
                )
            {
                return Err(StoreError::Revision);
            }
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
                .open_requests(account)?
                .into_iter()
                .map(|(request, state, created)| {
                    json!({
                        "id": request.id,
                        "device": b64(&request.device),
                        "name": request.name,
                        "proof": b64(&request.proof),
                        "state": state,
                        "created": created,
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
        ("POST", Some("requests"), Some(id)) => {
            let value = json_body(body)?;
            if value.get("reject").and_then(Value::as_bool) == Some(true) {
                store.reject(account, id)?;
                return Ok((200, json!({"ok": true})));
            }
            if value.get("release").and_then(Value::as_bool) == Some(true) {
                store.release(account, id)?;
                return Ok((200, json!({"ok": true})));
            }
            if value.get("done").and_then(Value::as_bool) == Some(true) {
                let device = crate::auth::read_auth(headers)
                    .map(|auth| auth.device)
                    .ok_or(StoreError::Unknown)?;
                store.finish(account, id, &device)?;
                return Ok((200, json!({"ok": true})));
            }
            let chain = decode_field(&value, "chain")?;
            let wrap = decode_field(&value, "wrap")?;
            let record = decode_field(&value, "record")?;
            let revision = value
                .get("revision")
                .and_then(Value::as_u64)
                .ok_or(StoreError::Unreadable)?;
            let (current, _) = store.get(account)?;
            if current != revision
                || !same_revision(
                    headers.get("if-match").map(String::as_str).unwrap_or(""),
                    current,
                )
            {
                return Err(StoreError::Revision);
            }
            let writes = object_writes(&value)?;
            let dropping = id_list(&value, "drop")?;
            let (revision, written) = store.publish_approval(
                account, id, chain, wrap, record, revision, recovery, writes, &dropping,
            )?;
            Ok((
                200,
                json!({"ok": true, "revision": revision, "objects": written_rows(&written)}),
            ))
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
            None,
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
            None,
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
    write_bytes(stream, 200, "OK", kind, &bytes, false, None)
}

fn same_revision(header: &str, revision: u64) -> bool {
    header.trim() == format!("\"{revision}\"")
}

fn mailbox_etag(payload: &Value) -> Option<String> {
    let revision = payload.get("revision")?.as_u64()?;
    payload.get("record")?;
    Some(format!("\"{revision}\""))
}

fn write_not_modified(stream: &mut TcpStream, etag: &str) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\nETag: {etag}\r\n{CORS}\r\n"
    );
    stream.write_all(head.as_bytes())
}

fn write_json(
    stream: &mut TcpStream,
    status: u16,
    value: &Value,
    etag: Option<&str>,
) -> std::io::Result<()> {
    let reason = reason(status);
    let body = value.to_string();
    write_bytes(
        stream,
        status,
        reason,
        "application/json",
        body.as_bytes(),
        true,
        etag,
    )
}

fn write_bytes(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
    cors: bool,
    etag: Option<&str>,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        body.len()
    );
    if let Some(etag) = etag {
        head.push_str(&format!("ETag: {etag}\r\n"));
    }
    if cors {
        head.push_str(CORS);
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::same_revision;
    use crate::store::StoreError;

    #[test]
    fn a_matching_revision_tag_is_the_only_one_that_skips_the_record() {
        assert!(same_revision("\"4\"", 4));
        assert!(same_revision(" \"4\" ", 4));
        assert!(!same_revision("\"3\"", 4));
        assert!(!same_revision("4", 4));
        assert!(!same_revision("*", 4));
        assert!(!same_revision("W/\"4\"", 4));
    }

    #[test]
    fn a_guess_a_join_and_a_read_stop_before_they_can_name_a_missing_account() {
        let mut store = crate::store::Store::new();
        store.set_now(4_000);
        let headers = BTreeMap::new();
        let guess = "203.0.113.40";
        for _ in 0..crate::auth::GUESS_LIMIT {
            let err = super::route(
                "POST",
                "/v1/access",
                br#"{"code":"nope"}"#,
                &headers,
                guess,
                &mut store,
            )
            .unwrap_err();
            assert_eq!(err, StoreError::BadAccess);
        }
        let capped = super::route(
            "POST",
            "/v1/access",
            br#"{"code":"nope"}"#,
            &headers,
            guess,
            &mut store,
        )
        .unwrap_err();
        assert_eq!(capped, StoreError::Limited);
        assert_eq!(capped.message(), "Try again in a moment.");
        store.set_now(4_000 + crate::auth::RATE_WINDOW_MS);
        let again = super::route(
            "POST",
            "/v1/access",
            br#"{"code":"nope"}"#,
            &headers,
            guess,
            &mut store,
        )
        .unwrap_err();
        assert_eq!(again, StoreError::BadAccess);

        store.set_now(4_000);
        let join = "203.0.113.41";
        for _ in 0..crate::auth::JOIN_LIMIT {
            let err = super::route(
                "POST",
                "/v1/join",
                br#"{"code":"nope"}"#,
                &headers,
                join,
                &mut store,
            )
            .unwrap_err();
            assert_eq!(err, StoreError::BadInvite);
        }
        assert_eq!(
            super::route(
                "POST",
                "/v1/join",
                br#"{"code":"nope"}"#,
                &headers,
                join,
                &mut store,
            )
            .unwrap_err(),
            StoreError::Limited
        );

        let missing = "cd".repeat(32);
        let read = "203.0.113.42";
        for _ in 0..crate::auth::READ_ACCOUNT_LIMIT {
            let err = super::route(
                "GET",
                &format!("/v1/mailboxes/{missing}"),
                b"",
                &headers,
                read,
                &mut store,
            )
            .unwrap_err();
            assert_eq!(err, StoreError::Unknown);
        }
        let hidden = super::route(
            "GET",
            &format!("/v1/mailboxes/{missing}"),
            b"",
            &headers,
            read,
            &mut store,
        )
        .unwrap_err();
        assert_eq!(hidden, StoreError::Limited);
        assert!(!hidden.message().contains(&missing));
        let other = "ce".repeat(32);
        let still = super::route(
            "GET",
            &format!("/v1/mailboxes/{other}"),
            b"",
            &headers,
            "203.0.113.43",
            &mut store,
        )
        .unwrap_err();
        assert_eq!(still, StoreError::Unknown);
        assert_eq!(still.message(), StoreError::Unknown.message());
    }
}

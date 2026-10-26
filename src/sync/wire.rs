//! The mailbox. Bodies are sealed records and sealed objects.
//! This module records every body it sends so a test can show the server
//! never received a key, the recovery words, or page text.

use std::time::Duration;

use serde_json::Value;

use omacus_sync_web::desk::{self, Desk};

pub struct Reply {
    pub json: Value,
}

pub struct Client {
    origin: String,
    pub sent: Vec<String>,
}

impl Client {
    pub fn new(origin: &str) -> Self {
        Self {
            origin: origin.trim_end_matches('/').to_string(),
            sent: Vec::new(),
        }
    }

    #[cfg(test)]
    pub fn set_origin(&mut self, origin: &str) {
        self.origin = origin.trim_end_matches('/').to_string();
    }

    pub fn open(&mut self, method: &str, path: &str, body: Option<Value>) -> Result<Reply, String> {
        self.exchange(None, method, path, body, &[])
    }

    /// An unsigned call that still sends a header, such as the join proof.
    pub fn open_headers(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        extra: &[(&str, String)],
    ) -> Result<Reply, String> {
        self.exchange(None, method, path, body, extra)
    }

    pub fn signed(
        &mut self,
        desk: &Desk,
        method: &str,
        path: &str,
        body: Option<Value>,
        extra: &[(&str, String)],
    ) -> Result<Reply, String> {
        self.exchange(Some(desk), method, path, body, extra)
    }

    fn exchange(
        &mut self,
        desk: Option<&Desk>,
        method: &str,
        path: &str,
        body: Option<Value>,
        extra: &[(&str, String)],
    ) -> Result<Reply, String> {
        let text = body
            .as_ref()
            .map(|value| value.to_string())
            .unwrap_or_default();
        if !text.is_empty() {
            self.sent.push(text.clone());
        }
        let url = format!("{}{path}", self.origin);
        let auth = match desk {
            Some(desk) => Some(
                desk.sign_request(method, path, text.as_bytes())
                    .map_err(|e| e.message().to_string())?,
            ),
            None => None,
        };
        let result = if text.is_empty() {
            let mut req = agent()
                .get(&url)
                .header("User-Agent", concat!("omacus/", env!("CARGO_PKG_VERSION")));
            if let Some(auth) = &auth {
                req = req
                    .header("X-Omacus-Device", &auth.device)
                    .header("X-Omacus-Time", &auth.time)
                    .header("X-Omacus-Nonce", &auth.nonce)
                    .header("X-Omacus-Signature", &auth.signature);
            }
            for (name, value) in extra {
                req = req.header(*name, value);
            }
            req.call()
        } else if method == "PUT" {
            let mut req = agent()
                .put(&url)
                .header("User-Agent", concat!("omacus/", env!("CARGO_PKG_VERSION")))
                .header("Content-Type", "application/json");
            if let Some(auth) = &auth {
                req = req
                    .header("X-Omacus-Device", &auth.device)
                    .header("X-Omacus-Time", &auth.time)
                    .header("X-Omacus-Nonce", &auth.nonce)
                    .header("X-Omacus-Signature", &auth.signature);
            }
            for (name, value) in extra {
                req = req.header(*name, value);
            }
            req.send(text.as_bytes())
        } else {
            let mut req = agent()
                .post(&url)
                .header("User-Agent", concat!("omacus/", env!("CARGO_PKG_VERSION")))
                .header("Content-Type", "application/json");
            if let Some(auth) = &auth {
                req = req
                    .header("X-Omacus-Device", &auth.device)
                    .header("X-Omacus-Time", &auth.time)
                    .header("X-Omacus-Nonce", &auth.nonce)
                    .header("X-Omacus-Signature", &auth.signature);
            }
            for (name, value) in extra {
                req = req.header(*name, value);
            }
            req.send(text.as_bytes())
        };
        let mut response = result.map_err(|_| "Offline. Your pages are still here.".to_string())?;
        let status = response.status().as_u16();
        if status == 304 {
            return Ok(Reply { json: Value::Null });
        }
        let raw = response
            .body_mut()
            .read_to_string()
            .map_err(|_| "Couldn't sync.".to_string())?;
        let json: Value = if raw.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&raw).unwrap_or(Value::Null)
        };
        if !(200..300).contains(&status) {
            let message = json
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Couldn't sync.");
            return Err(message.to_string());
        }
        Ok(Reply { json })
    }
}

pub fn b64(bytes: &[u8]) -> String {
    desk::b64_encode(bytes)
}

pub fn unb64(text: &str) -> Result<Vec<u8>, String> {
    desk::b64_decode(text).map_err(|e| e.message().to_string())
}

pub fn hex_id(text: &str) -> Result<[u8; 16], String> {
    desk::decode_hex_id(text).map_err(|e| e.message().to_string())
}

fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(2)))
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build()
            .into()
    })
}

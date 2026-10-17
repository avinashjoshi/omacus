//! Share links: a sheet carried in the part of a URL after `#`, which browsers never
//! send to a server. `v1.` then the text, deflated (raw, no header) and
//! base64url-encoded without padding. omacus.com/app reads and writes the same thing
//! with the browser's CompressionStream("deflate-raw").

/// Where share links open.
pub const APP_URL: &str = "https://omacus.com/app";
const VERSION: &str = "v1.";
/// A link may expand to at most this much text, so it can't unpack into gigabytes.
pub const MAX_BYTES: usize = 1 << 20;

/// The full link for a sheet: `https://omacus.com/app#v1.…`.
pub fn link(text: &str) -> String {
    format!("{APP_URL}#{}", encode(text))
}

/// `v1.…` for a sheet.
pub fn encode(text: &str) -> String {
    let packed = miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 9);
    format!("{VERSION}{}", base64url(&packed))
}

/// The sheet in a share link, from anything that carries one: the web link (with or
/// without the trailing slash), an `omacus://open#…` link, or the bare `v1.…`.
pub fn decode(link: &str) -> Option<String> {
    let link = link.trim();
    let data = match link.rsplit_once('#') {
        Some((before, after)) => {
            let ok = before.trim_end_matches('/').ends_with("omacus.com/app")
                || before.starts_with("omacus://");
            if !ok {
                return None;
            }
            after
        }
        None => link.strip_prefix("omacus://open/").unwrap_or(link),
    };
    let data = data.strip_prefix(VERSION)?;
    let packed = unbase64url(data)?;
    let bytes = miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, MAX_BYTES).ok()?;
    String::from_utf8(bytes).ok()
}

/// Whether some text is a share link (and nothing else), for pasting one into the app.
pub fn is_link(text: &str) -> bool {
    let t = text.trim();
    !t.contains(char::is_whitespace) && decode(t).is_some()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn unbase64url(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32)
    };
    let s = s.trim_end_matches('=').as_bytes();
    if s.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for chunk in s.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= value(c)? << (18 - 6 * i);
        }
        for i in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let sheet = "# Kyoto\nFlights: 4 * $1,180\n¥68,000 in USD\n🍜 noodles ₹200";
        let link = link(sheet);
        assert!(link.starts_with("https://omacus.com/app#v1."));
        assert_eq!(decode(&link).as_deref(), Some(sheet));
        assert_eq!(
            decode(&link.replace("/app#", "/app/#")).as_deref(),
            Some(sheet)
        );
        assert_eq!(
            decode(&link.replace("https://omacus.com/app", "omacus://open")).as_deref(),
            Some(sheet)
        );
        assert_eq!(decode(&encode(sheet)).as_deref(), Some(sheet));
        assert_eq!(decode(&encode("")).as_deref(), Some(""));
    }

    #[test]
    fn reads_links_made_by_the_browser() {
        // From omacus.com/app (CompressionStream "deflate-raw"): "# Live test\n$84 split 3 ways\n€50 in USD\n#336699".
        let web = "https://omacus.com/app#v1.U1bwySxLVShJLS7hUrEwUSguyMksUTBWKE-sLOZ61LTG1EAhM08hNNiFS9nY2MzM0hIA";
        assert_eq!(
            decode(web).as_deref(),
            Some("# Live test\n$84 split 3 ways\n€50 in USD\n#336699")
        );
    }

    #[test]
    fn rejects_other_links_and_bombs() {
        assert_eq!(
            decode("https://example.com/app#v1.U1bwySxLVShJLS7hUrEwUSguyMksUTBWKE"),
            None
        );
        assert_eq!(decode("v2.abc"), None);
        assert_eq!(decode("v1.!!!"), None);
        assert!(!is_link("look at this https://omacus.com/app#v1.abc"));
        // 10 MB of zeros compresses to a few KB; it must not expand past the cap.
        let bomb = encode(&"0".repeat(10 << 20));
        assert!(bomb.len() < 30_000);
        assert_eq!(decode(&bomb), None);
    }
}

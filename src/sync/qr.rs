//! The join picture. The payload is the same link the web app encodes.
//! A camera that will not open still leaves the typed code as the way in.

#[cfg(test)]
use base64::Engine;
use qrcode::QrCode;

/// Dark modules, including a quiet margin, so the window can paint the picture.
pub fn modules(payload: &str) -> Result<Vec<bool>, String> {
    let code = QrCode::new(payload.as_bytes()).map_err(|_| "Try again in a moment.".to_string())?;
    let width = code.width();
    let quiet = 2;
    let side = width + quiet * 2;
    let mut out = vec![false; side * side];
    for y in 0..width {
        for x in 0..width {
            if code[(x, y)] == qrcode::Color::Dark {
                let xx = x + quiet;
                let yy = y + quiet;
                out[yy * side + xx] = true;
            }
        }
    }
    Ok(out)
}

/// Generate a picture and read it back. This is the scan, without a camera.
#[cfg(test)]
pub fn round_trip(payload: &str) -> Result<String, String> {
    let code = QrCode::new(payload.as_bytes()).map_err(|_| "Try again in a moment.".to_string())?;
    let n = code.width();
    let quiet = 4;
    let scale = 4;
    let side = (n + quiet * 2) * scale;
    let mut pixels = vec![255u8; side * side];
    for y in 0..n {
        for x in 0..n {
            if code[(x, y)] != qrcode::Color::Dark {
                continue;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    let xx = (x + quiet) * scale + dx;
                    let yy = (y + quiet) * scale + dy;
                    pixels[yy * side + xx] = 0;
                }
            }
        }
    }
    let mut image =
        rqrr::PreparedImage::prepare_from_greyscale(side, side, |x, y| pixels[y * side + x]);
    let grids = image.detect_grids();
    let grid = grids.first().ok_or("That picture could not be read.")?;
    let (_meta, content) = grid
        .decode()
        .map_err(|_| "That picture could not be read.".to_string())?;
    Ok(content)
}

/// A join link, or the raw `omacus-join` payload inside it.
#[cfg(test)]
pub fn parse_join(raw: &str) -> Option<(String, String, String)> {
    let text = if let Some(fragment) = raw.split_once("#join=") {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(fragment.1.trim())
            .ok()?;
        String::from_utf8(bytes).ok()?
    } else {
        raw.to_string()
    };
    let mut lines = text.lines();
    if lines.next()? != "omacus-join" {
        return None;
    }
    let origin = lines.next()?.trim().to_string();
    let account = lines.next()?.trim().to_string();
    let code = lines.next()?.trim().to_string();
    if origin.is_empty() || account.len() != 64 || code.is_empty() {
        return None;
    }
    Some((origin, account, code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_join_picture_round_trips() {
        let account = "ab".repeat(32);
        let link = omacus_sync_web::desk::join_link(
            "http://127.0.0.1:9",
            "http://127.0.0.1:9",
            &account,
            "K7QM-4R2P-9HNX",
        )
        .expect("link");
        let read = round_trip(&link).expect("decode");
        assert_eq!(read, link);
        let (origin, found, code) = parse_join(&read).expect("parse");
        assert_eq!(origin, "http://127.0.0.1:9");
        assert_eq!(found, account);
        assert_eq!(code, "K7QM-4R2P-9HNX");
    }
}

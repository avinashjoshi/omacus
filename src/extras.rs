//! Small calculators that sit beside fend: CSS units (`16px in rem`) and colours
//! (`#ff8800 in rgb`, `rgb(255, 136, 0) in hsl`). Each line is complete on its own.

use std::sync::LazyLock;

use regex::Regex;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

/// An answer, and the colour to show beside it when it is one.
pub struct Extra {
    pub display: String,
    pub color: Option<[u8; 3]>,
}

pub fn eval(line: &str) -> Option<Extra> {
    css_units(line).or_else(|| color(line))
}

// ---------------------------------------------------------------------------------------
// CSS units: px, rem, em, pt. 1rem is 16px unless the line says otherwise
// ("24px in rem at 18px").

re!(
    CSS,
    r"(?i)^(-?\d+(?:\.\d+)?)\s*(px|rem|em|pt)\s+(?:in|to|as)\s+(px|rem|em|pt)(?:\s+(?:at|with)\s+(\d+(?:\.\d+)?)\s*px(?:\s+base)?)?$"
);

fn css_units(line: &str) -> Option<Extra> {
    let c = CSS.captures(line.trim())?;
    let value: f64 = c[1].parse().ok()?;
    let base: f64 = c.get(4).map_or(Some(16.0), |m| m.as_str().parse().ok())?;
    if base <= 0.0 {
        return None;
    }
    let px_per = |unit: &str| match unit.to_ascii_lowercase().as_str() {
        "px" => 1.0,
        "pt" => 4.0 / 3.0,
        _ => base,
    };
    let to = c[3].to_ascii_lowercase();
    let n = value * px_per(&c[2]) / px_per(&to);
    Some(Extra {
        display: format!("{} {to}", trim_number(n, 4)),
        color: None,
    })
}

// ---------------------------------------------------------------------------------------
// Colours: #rgb, #rrggbb, rgb(r, g, b) and hsl(h, s%, l%), converted with "in hex",
// "in rgb" or "in hsl". A bare colour shows its other common form.

re!(HEX, r"^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$");
re!(
    RGB,
    r"(?i)^rgba?\(\s*(\d{1,3})\s*[, ]\s*(\d{1,3})\s*[, ]\s*(\d{1,3})\s*(?:[,/]\s*[\d.]+%?\s*)?\)$"
);
re!(
    HSL,
    r"(?i)^hsla?\(\s*(\d+(?:\.\d+)?)(?:deg)?\s*[, ]\s*(\d+(?:\.\d+)?)%\s*[, ]\s*(\d+(?:\.\d+)?)%\s*(?:[,/]\s*[\d.]+%?\s*)?\)$"
);
re!(CONVERT, r"(?i)^(.+?)\s+(?:in|to|as)\s+(hex|rgb|hsl)$");

/// Whether a line starting with `#` is a colour rather than a header: "#ff8800",
/// "#f80 in rgb". Hex-looking words stay headers ("#bad", "#cafe").
pub fn is_color(line: &str) -> bool {
    let line = line.trim();
    let (code, converted) = match CONVERT.captures(line) {
        Some(c) => (c.get(1).map_or("", |m| m.as_str()), true),
        None => (line, false),
    };
    let Some(c) = HEX.captures(code) else {
        return false;
    };
    converted || c[1].len() == 6 || c[1].chars().any(|ch| ch.is_ascii_digit())
}

fn parse_color(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    if let Some(c) = HEX.captures(s) {
        let hex = &c[1];
        let full: String = if hex.len() == 3 {
            hex.chars().flat_map(|ch| [ch, ch]).collect()
        } else {
            hex.to_string()
        };
        let byte = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok();
        return Some([byte(0)?, byte(2)?, byte(4)?]);
    }
    if let Some(c) = RGB.captures(s) {
        let v = |i: usize| {
            c[i].parse::<u16>()
                .ok()
                .filter(|v| *v <= 255)
                .map(|v| v as u8)
        };
        return Some([v(1)?, v(2)?, v(3)?]);
    }
    if let Some(c) = HSL.captures(s) {
        let h: f64 = c[1].parse().ok()?;
        let s: f64 = c[2].parse().ok()?;
        let l: f64 = c[3].parse().ok()?;
        if s > 100.0 || l > 100.0 {
            return None;
        }
        return Some(hsl_to_rgb(h, s / 100.0, l / 100.0));
    }
    None
}

fn color(line: &str) -> Option<Extra> {
    let line = line.trim();
    let (rgb, target) = match CONVERT.captures(line) {
        Some(c) => (parse_color(&c[1])?, c[2].to_ascii_lowercase()),
        None => {
            let rgb = parse_color(line)?;
            if line.starts_with('#') && !is_color(line) {
                return None;
            }
            // Bare: show the form it isn't written in.
            (
                rgb,
                if line.starts_with('#') { "rgb" } else { "hex" }.to_string(),
            )
        }
    };
    let [r, g, b] = rgb;
    let display = match target.as_str() {
        "hex" => format!("#{r:02x}{g:02x}{b:02x}"),
        "rgb" => format!("rgb({r}, {g}, {b})"),
        _ => {
            let (h, s, l) = rgb_to_hsl(rgb);
            format!(
                "hsl({}, {}%, {}%)",
                h.round(),
                (s * 100.0).round(),
                (l * 100.0).round()
            )
        }
    };
    Some(Extra {
        display,
        color: Some(rgb),
    })
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [to(r), to(g), to(b)]
}

fn rgb_to_hsl([r, g, b]: [u8; 3]) -> (f64, f64, f64) {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s, l)
}

/// Up to `places` decimals, without trailing zeros: 0.75, 1.3333, 16.
fn trim_number(n: f64, places: usize) -> String {
    let s = format!("{n:.places$}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(line: &str) -> Option<String> {
        eval(line).map(|e| e.display)
    }

    #[test]
    fn css_units() {
        assert_eq!(show("16px in rem").as_deref(), Some("1 rem"));
        assert_eq!(show("12 px to rem").as_deref(), Some("0.75 rem"));
        assert_eq!(show("1.5rem in px").as_deref(), Some("24 px"));
        assert_eq!(show("12pt in px").as_deref(), Some("16 px"));
        assert_eq!(show("24px in rem at 18px").as_deref(), Some("1.3333 rem"));
        assert_eq!(show("16px"), None);
    }

    #[test]
    fn colors() {
        assert_eq!(show("#ff8800 in rgb").as_deref(), Some("rgb(255, 136, 0)"));
        assert_eq!(show("#f80 in rgb").as_deref(), Some("rgb(255, 136, 0)"));
        assert_eq!(show("#ff8800").as_deref(), Some("rgb(255, 136, 0)"));
        assert_eq!(show("rgb(255, 136, 0)").as_deref(), Some("#ff8800"));
        assert_eq!(
            show("rgb(255 136 0) in hsl").as_deref(),
            Some("hsl(32, 100%, 50%)")
        );
        assert_eq!(
            show("hsl(32, 100%, 50%) in hex").as_deref(),
            Some("#ff8800")
        );
        assert_eq!(show("rgb(300, 0, 0)"), None);
        assert_eq!(eval("#ff8800").unwrap().color, Some([255, 136, 0]));
    }

    #[test]
    fn hex_words_stay_headers() {
        assert!(is_color("#ff8800"));
        assert!(is_color("#f80"));
        assert!(is_color("#fff in rgb"));
        assert!(!is_color("#bad"));
        assert!(!is_color("#cafe"));
        assert!(!is_color("# Budget"));
        assert_eq!(show("#bad"), None);
    }
}

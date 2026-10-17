//! Compress the bundled fonts, so the binary carries them deflated (about 40% smaller);
//! the app inflates them once at startup.

use std::path::Path;

const FONTS: [&str; 3] = [
    "JetBrainsMono-Regular.ttf",
    "NotoSans-Currency.ttf",
    "NotoEmoji-Regular.ttf",
];

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    for name in FONTS {
        let src = Path::new("assets/fonts").join(name);
        println!("cargo::rerun-if-changed={}", src.display());
        let bytes = std::fs::read(&src).unwrap();
        let packed = miniz_oxide::deflate::compress_to_vec(&bytes, 10);
        std::fs::write(Path::new(&out).join(format!("{name}.deflate")), packed).unwrap();
    }
}

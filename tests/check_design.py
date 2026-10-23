#!/usr/bin/env python3
"""Fail when a UI color is hardcoded outside the design tokens.

Desktop colors live in src/theme.rs. Website and web-shell colors live in
site/css/tokens.css. The four built-in app themes must match, including the
borders mixed from text and background.

A line that contains `design-allow`, or a path listed in tests/design-allow.txt,
is a justified exception.
"""

from __future__ import annotations

import math
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
THEME = ROOT / "src" / "theme.rs"
TOKENS = ROOT / "site" / "css" / "tokens.css"
ALLOW = ROOT / "tests" / "design-allow.txt"

RUST_COLOR = re.compile(
    r"Color32::from_rgb\(\s*(0x[0-9a-fA-F]+|\d+)"
)
RUST_HEX = re.compile(r"""hex!\(\s*["']#|hex\(\s*["']#""")
CSS_HEX = re.compile(r"#[0-9a-fA-F]{3,8}\b")

FIELD_TO_CSS = {
    "background": "--app-bg",
    "sidebar": "--app-side",
    "foreground": "--app-fg",
    "dim": "--app-dim",
    "comment": "--app-comment",
    "accent": "--app-accent",
    "variable": "--app-var",
    "defined": "--app-def",
    "keyword": "--app-kw",
    "number": "--app-num",
    "selection": "--app-sel",
    "hover": "--app-hover",
}


def rust_round(value: float) -> int:
    """f32::round is half away from zero. Python's round is half to even."""
    if value >= 0:
        return int(math.floor(value + 0.5))
    return int(math.ceil(value - 0.5))


def parse_hex(text: str) -> tuple[int, int, int]:
    text = text.removeprefix("#")
    return tuple(int(text[i : i + 2], 16) for i in (0, 2, 4))


def hex_of(rgb: tuple[int, int, int]) -> str:
    return "".join(f"{channel:02x}" for channel in rgb)


def mix(foreground: tuple[int, int, int], background: tuple[int, int, int], t: float):
    return tuple(
        rust_round(foreground[i] * t + background[i] * (1.0 - t)) for i in range(3)
    )


def channel(value: int) -> float:
    c = value / 255.0
    if c <= 0.04045:
        return c / 12.92
    return ((c + 0.055) / 1.055) ** 2.4


def luminance(rgb: tuple[int, int, int]) -> float:
    return 0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])


def contrast(a: tuple[int, int, int], b: tuple[int, int, int]) -> float:
    hi, lo = sorted((luminance(a), luminance(b)), reverse=True)
    return (hi + 0.05) / (lo + 0.05)


def const_str(src: str, name: str) -> str:
    match = re.search(rf'const {name}: &str = "(#[0-9a-fA-F]{{6}})"', src)
    if not match:
        raise SystemExit(f"src/theme.rs is missing const {name}")
    return match.group(1).lower()


def const_f32(src: str, name: str) -> float:
    match = re.search(rf"const {name}: f32 = ([0-9.]+)", src)
    if not match:
        raise SystemExit(f"src/theme.rs is missing const {name}")
    return float(match.group(1))


def built_in(src: str, name: str) -> dict[str, str]:
    match = re.search(rf"fn {name}\(\) -> Self \{{(.*?)\n    \}}", src, re.S)
    if not match:
        raise SystemExit(f"could not read Theme::{name}")
    body = match.group(1)
    colors = {
        field: value.lower()
        for field, value in re.findall(r'(\w+): hex\("#([0-9a-fA-F]{6})"\)', body)
    }
    dark = re.search(r"dark:\s*(true|false)", body)
    if not dark or "background" not in colors:
        raise SystemExit(f"Theme::{name} is missing dark or background")
    colors["dark"] = dark.group(1)
    return colors


def css_theme(css: str, name: str) -> dict[str, str]:
    match = re.search(rf'\[data-app-theme="{name}"\] \{{(.*?)\n\}}', css, re.S)
    if not match:
        raise SystemExit(f"site/css/tokens.css is missing [data-app-theme={name}]")
    return {
        var: value.lower()
        for var, value in re.findall(r"(--app-[\w-]+):\s*#([0-9a-fA-F]{3,8})", match.group(1))
    }


def role(theme: dict[str, str], dark_hex: str, paper_hex: str, light_hex: str) -> str:
    if theme["dark"] == "true":
        return dark_hex.removeprefix("#")
    if theme["background"] == "f6f1e7":
        return paper_hex.removeprefix("#")
    return light_hex.removeprefix("#")


def on_accent(src: str, theme: dict[str, str]) -> str:
    paper = const_str(src, "INK_PAPER").removeprefix("#")
    light = const_str(src, "INK_LIGHT").removeprefix("#")
    if theme["background"] == "f6f1e7":
        dark_ink = parse_hex(paper)
    elif theme["dark"] == "true":
        dark_ink = parse_hex(theme["background"])
    else:
        dark_ink = parse_hex(light)
    accent = parse_hex(theme["accent"])
    white = (255, 255, 255)
    if contrast(dark_ink, accent) >= contrast(white, accent):
        return hex_of(dark_ink)
    return "ffffff"


def check_themes(errors: list[str]) -> None:
    src = THEME.read_text()
    css = TOKENS.read_text()
    mixes = {
        "dark": const_f32(src, "BORDER_MIX_DARK"),
        "light": const_f32(src, "BORDER_MIX_LIGHT"),
        "strong_dark": const_f32(src, "BORDER_STRONG_DARK"),
        "strong_light": const_f32(src, "BORDER_STRONG_LIGHT"),
    }
    danger = (
        const_str(src, "DANGER_DARK"),
        const_str(src, "DANGER_PAPER"),
        const_str(src, "DANGER_LIGHT"),
    )
    warning = (
        const_str(src, "WARNING_DARK"),
        const_str(src, "WARNING_PAPER"),
        const_str(src, "WARNING_LIGHT"),
    )
    for name in ("dark", "light", "paper", "dusk"):
        theme = built_in(src, name)
        painted = css_theme(css, name)
        for field, var in FIELD_TO_CSS.items():
            if painted.get(var) != theme[field]:
                errors.append(
                    f"{name} {var} is #{painted.get(var)} but Theme::{name}.{field} is #{theme[field]}"
                )
        fg = parse_hex(theme["foreground"])
        bg = parse_hex(theme["background"])
        hairline = mixes["dark"] if theme["dark"] == "true" else mixes["light"]
        strong = mixes["strong_dark"] if theme["dark"] == "true" else mixes["strong_light"]
        expect = {
            "--app-line": hex_of(mix(fg, bg, hairline)),
            "--app-line-strong": hex_of(mix(fg, bg, strong)),
            "--app-danger": role(theme, *danger),
            "--app-warning": role(theme, *warning),
            "--app-on-accent": on_accent(src, theme),
        }
        for var, value in expect.items():
            if painted.get(var) != value:
                errors.append(
                    f"{name} {var} is #{painted.get(var)} but src/theme.rs produces #{value}"
                )


def allowlist() -> list[str]:
    if not ALLOW.exists():
        return []
    paths = []
    for line in ALLOW.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            paths.append(line)
    return paths


def allowed(path: Path, line: str, listed: list[str]) -> bool:
    if "design-allow" in line:
        return True
    rel = path.relative_to(ROOT).as_posix()
    return any(rel == item or rel.startswith(item) for item in listed)


def check_rust(errors: list[str], listed: list[str]) -> None:
    for path in ROOT.rglob("*.rs"):
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith("target/") or rel == "src/theme.rs":
            continue
        for number, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
            if allowed(path, line, listed):
                continue
            if RUST_COLOR.search(line) or RUST_HEX.search(line):
                errors.append(f"{rel}:{number}: hardcoded color outside src/theme.rs")


def check_css(errors: list[str], listed: list[str]) -> None:
    for path in (ROOT / "site").rglob("*.css"):
        rel = path.relative_to(ROOT).as_posix()
        if rel == "site/css/tokens.css":
            continue
        for number, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
            if allowed(path, line, listed):
                continue
            if CSS_HEX.search(line):
                errors.append(f"{rel}:{number}: hex color outside site/css/tokens.css")


def main() -> int:
    errors: list[str] = []
    check_themes(errors)
    listed = allowlist()
    check_rust(errors, listed)
    check_css(errors, listed)
    if errors:
        print("design check failed:")
        for error in errors:
            print(f"  {error}")
        return 1
    print("design check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())

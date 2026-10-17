#!/usr/bin/env python3
"""Accuracy checks for Omacus, against exact maths rather than fend.

    python3 engine/tests/accuracy.py target/debug/omacus [lines per seed]

- Random arithmetic and percentage lines, and values near fend 1.5.8's rounding
  carry bug, each compared with the exact value (in fractions) as Omacus rounds it.
- Unit and temperature conversions compared with their exact definitions (NIST).

Prints each wrong line and exits non-zero if there are any. CI runs it."""

import random
import re
import subprocess
import sys
from fractions import Fraction as F

BIN = sys.argv[1]
N = int(sys.argv[2]) if len(sys.argv) > 2 else 2000
rng = random.Random(1)


def answers(lines):
    """Omacus's answer for each line, one sheet per 200 lines."""
    out = []
    for i in range(0, len(lines), 200):
        batch = lines[i : i + 200]
        res = subprocess.run([BIN, "-"], input="\n".join(batch), capture_output=True, text=True).stdout
        got = res.split("\n")[: len(batch)]
        out += got + [""] * (len(batch) - len(got))
    return out


def number():
    r = rng.random()
    if r < 0.5:
        v = rng.randint(0, 999)
        return str(v), F(v)
    if r < 0.8:
        a, b = rng.randint(0, 999), rng.randint(0, 99)
        return f"{a}.{b:02d}", F(f"{a}.{b:02d}")
    v = rng.randint(1000, 9_999_999)
    return f"{v:,}", F(v)


def exact(text):
    """`text` with ordinary precedence, in exact fractions (None if it divides by 0)."""
    py = re.sub(r"\d[\d,]*(?:\.\d+)?", lambda m: f'F("{m.group(0).replace(",", "")}")', text)
    try:
        return eval(py, {"F": F})  # only digits, operators and brackets reach here
    except ZeroDivisionError:
        return None


def expression(depth=0):
    if depth > 2 or rng.random() < 0.35:
        return number()[0]
    text = f"{expression(depth + 1)} {rng.choice('+-*/')} {expression(depth + 1)}"
    return f"({text})" if rng.random() < 0.3 else text


def percentage():
    a_t, a = number()
    p, q, k = F(rng.randint(1, 99)), F(rng.randint(1, 99)), F(rng.randint(2, 5))
    return rng.choice([
        (f"{a_t} + {p}%", a * (1 + p / 100)),
        (f"{a_t} - {p}%", a * (1 - p / 100)),
        (f"{p}% of {a_t}", a * p / 100),
        (f"{p}% off {a_t}", a * (1 - p / 100)),
        (f"{p}% on {a_t}", a * (1 + p / 100)),
        (f"{a_t} - {p}% - {q}%", a * (1 - p / 100) * (1 - q / 100)),
        (f"{a_t} + {p}% * {k}", a * (1 + p * k / 100)),
        (f"{a_t} * {p}%", a * p / 100),
        (f"{q}% off {p}% off {a_t}", a * (1 - p / 100) * (1 - q / 100)),
    ])


def line():
    r = rng.random()
    if r < 0.1:
        # Near fend's rounding carry: a value plus p/q * 10^-k.
        a_t, a = number()
        p, q, k = rng.randint(1, 9), rng.randint(2, 9), rng.randint(8, 14)
        return f"{a_t} + {p}/{q}/1{'0' * k}", a + F(p, q) / F(10) ** k
    if r < 0.6:
        text = expression()
        return text, exact(text)
    if r < 0.85:
        return percentage()
    a_t, a = number()
    b_t, b = number()
    op = rng.choice("+-*")
    return f"${a_t} {op} {b_t}", {"+": a + b, "-": a - b, "*": a * b}[op]


def shown(text):
    """The number in an answer ("$1,234.50", "2.88e-10", "25%"), or None."""
    s = text.strip().replace(",", "")
    neg = s.startswith("-")
    m = re.fullmatch(r"\$?(\d+(?:\.\d+)?)(?:e(-?\d+))?%?", s.lstrip("-"))
    if not m:
        return None
    v = F(m.group(1)) * (F(10) ** int(m.group(2)) if m.group(2) else 1)
    return -v if neg else v


def close(got, want):
    if want != 0 and abs(want) < F(1, 10**6):
        return abs(got - want) <= abs(want) * F(5, 1000)  # three significant digits
    return abs(got - want) <= max(F(1, 200), abs(want) * F(1, 10**9))


def check(name, cases):
    cases = [(t, v) for t, v in cases if v is not None]
    bad = 0
    for (text, want), got in zip(cases, answers([t for t, _ in cases])):
        value = shown(got)
        if value is None or not close(value, want):
            bad += 1
            if bad <= 30:
                print(f"  {text:48} got {got!r:24} want {float(want):.6g}")
    print(f"{name}: {len(cases)} lines, {bad} wrong")
    return bad


# 1 of each unit, exactly, by definition.
UNITS = [
    ("miles", "km", F("1.609344")), ("inches", "cm", F("2.54")), ("feet", "m", F("0.3048")),
    ("yards", "m", F("0.9144")), ("lb", "kg", F("0.45359237")), ("oz", "g", F("28.349523125")),
    ("gallons", "liters", F("3.785411784")), ("cups", "ml", F("236.5882365")),
    ("tablespoons", "ml", F("14.78676478125")), ("teaspoons", "ml", F("4.92892159375")),
    ("floz", "ml", F("29.5735295625")), ("nautical miles", "km", F("1.852")),
    ("acres", "m^2", F("4046.8564224")), ("hectares", "m^2", F(10000)),
    ("GiB", "bytes", F(2**30)), ("MB", "bytes", F(10**6)), ("kWh", "J", F(3600000)),
    ("mph", "km/h", F("1.609344")), ("days", "seconds", F(86400)), ("weeks", "hours", F(168)),
    ("km", "miles", 1 / F("1.609344")), ("kg", "lb", 1 / F("0.45359237")),
    ("liters", "gallons", 1 / F("3.785411784")), ("imperial gallons", "liters", F("4.54609")),
    ("stone", "kg", F("6.35029318")),
]


def conversions():
    cases = []
    for frm, to, factor in UNITS:
        for _ in range(20):
            x = F(rng.randint(1, 99999), rng.choice([1, 10, 100]))
            cases.append((f"{float(x):g} {frm} in {to}", x * factor))
    for _ in range(200):
        c = F(rng.randint(-400, 400))
        cases.append((f"{c} F in C", (c - 32) * 5 / 9))
        cases.append((f"{c} C in F", c * 9 / 5 + 32))
    return cases


if __name__ == "__main__":
    wrong = 0
    for seed in (1, 2, 3):
        rng.seed(seed)
        wrong += check(f"maths, seed {seed}", [line() for _ in range(N)])
    rng.seed(7)
    # Conversion answers carry a unit ("1.61 km"); compare the number part.
    shown_number = shown
    shown = lambda text: shown_number(text.split(" ")[0]) if text else None  # noqa: E731
    wrong += check("conversions", conversions())
    sys.exit(1 if wrong else 0)

"""The launch video's music bed and sound effects, synthesized from scratch.

    .venv/bin/python audio.py      reads build/scenes.json and build/events.json
                                   (written by render.mjs), writes build/music.wav
                                   and build/sfx.wav

Everything is generated here, so there's nothing to license. The music follows the
scenes at 116 BPM: a soft pad under the hook, kick and bass when the window lands,
claps and hats from the currency scene, a beat of silence and a drop into the montage,
a breath before the end card, and a final swell.
"""

import json
import wave
from pathlib import Path

import numpy as np

SR = 48000
BUILD = Path(__file__).parent / "build"
rng = np.random.default_rng(11)

scenes = {s["id"]: s for s in json.loads((BUILD / "scenes.json").read_text())}
events = json.loads((BUILD / "events.json").read_text())
total = max(s["start"] + s["dur"] for s in scenes.values())
N = int(total * SR) + SR


def at(scene, offset=0.0):
    return scenes[scene]["start"] + offset


def note(n):
    """MIDI note number to Hz."""
    return 440.0 * 2 ** ((n - 69) / 12)


def lowpass_fast(x, cutoff):
    """A gentle low-pass, done in the frequency domain."""
    spec = np.fft.rfft(x)
    freqs = np.fft.rfftfreq(len(x), 1 / SR)
    spec *= 1 / (1 + (freqs / cutoff) ** 4)
    return np.fft.irfft(spec, len(x))


def env(length, attack, release):
    e = np.ones(length)
    a = min(int(attack * SR), length)
    r = min(int(release * SR), length - a)
    if a:
        e[:a] = np.linspace(0, 1, a) ** 2
    if r:
        e[length - r :] *= np.linspace(1, 0, r) ** 2
    return e


def place(track, sig, t, pan=0.0, gain=1.0):
    """Add a mono signal to a stereo track at time t, panned -1..1."""
    i = int(t * SR)
    if i >= track.shape[1] or i + len(sig) <= 0:
        return
    sig = sig[: track.shape[1] - i]
    left = np.cos((pan + 1) * np.pi / 4)
    right = np.sin((pan + 1) * np.pi / 4)
    track[0, i : i + len(sig)] += sig * gain * left
    track[1, i : i + len(sig)] += sig * gain * right


# ----------------------------------------------------------------------------------
# Music

BPM = 116  # composition.js uses the same: scenes are whole beats, so cuts land on them
BEAT = 60 / BPM
BAR = 4 * BEAT
# Am9, Fmaj7, Cmaj7/E, G6: two bars each.
CHORDS = [
    [45, 57, 60, 64, 67, 71],
    [41, 53, 57, 60, 64, 69],
    [40, 52, 55, 59, 64, 67],
    [43, 55, 59, 62, 64, 67],
]
music = np.zeros((2, N))
start_pulse = at("intro")  # the window arrives: kick and bass
start_arp = at("intro", 2 * BEAT)
start_groove = at("currency")  # claps and hats
drop = at("montage")  # a beat of silence, then the drop
drop_end = at("keys")
breath = at("platforms", -BEAT)  # a beat of space before the outro
end = at("end")
ring = total - 6 * BEAT  # the last downbeat-ish: the groove stops and a chord rings out


def muted(t):
    """The beat before the drop and the breath before the outro are silent."""
    return drop - BEAT <= t < drop or breath <= t < end


t = 0.0
bar = 0
while t < total + BAR:
    chord = CHORDS[(bar // 2) % len(CHORDS)]
    length = int(2 * BAR * SR)
    n = np.arange(length) / SR
    # Pad: detuned saw-ish tones, softly filtered, slow swell.
    pad = np.zeros(length)
    for k, m in enumerate(chord[1:]):
        for det in (-0.06, 0.06):
            f = note(m + det)
            pad += sum(np.sin(2 * np.pi * f * h * n + k) / h**1.6 for h in (1, 2, 3))
    pad *= env(length, 0.9, 1.2) * 0.02
    place(music, pad, t, pan=-0.25)
    place(music, pad, t + 0.011, pan=0.25)
    # Bass on the root: eighth notes once the window is up, quarter notes in the outro.
    root = note(chord[0] - 12)
    for b in range(16):
        bt = t + b * BEAT / 2
        if (start_pulse - 0.01 <= bt < breath and not muted(bt)) or (end <= bt < ring and b % 2 == 0):
            ln = int(BEAT / 2 * SR)
            x = np.arange(ln) / SR
            g = 0.15 if b % 2 == 0 else 0.09
            bass = (np.sin(2 * np.pi * root * x) + 0.25 * np.sin(4 * np.pi * root * x)) * np.exp(-x * 5) * g
            bass *= np.minimum(1, x * 200)
            place(music, bass, bt)
    # Arpeggio in sixteenths; brighter (an octave up) through the drop.
    tones = [chord[2] + 12, chord[3] + 12, chord[4] + 12, chord[5] + 12, chord[4] + 12, chord[3] + 12]
    for st_i in range(32):
        st = t + st_i * BEAT / 4
        if (start_arp <= st < breath and not muted(st)) or (end + 0.6 <= st < end + 3.4):
            up = 12 if drop <= st < drop_end else 0
            f = note(tones[st_i % len(tones)] + up)
            ln = int(0.28 * SR)
            x = np.arange(ln) / SR
            pluck = (np.sin(2 * np.pi * f * x) + 0.3 * np.sin(4 * np.pi * f * x)) * np.exp(-x * 16) * 0.034
            place(music, pluck, st, pan=0.45 if st_i % 2 else -0.45)
    t += 2 * BAR
    bar += 2


def kick_sound(gain=0.34):
    ln = int(0.3 * SR)
    x = np.arange(ln) / SR
    f = 50 + 110 * np.exp(-x * 32)
    return np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-x * 8) * gain


def clap_sound(gain=0.06):
    ln = int(0.2 * SR)
    x = np.arange(ln) / SR
    nz = rng.standard_normal(ln)
    band = lowpass_fast(nz, 3500) - lowpass_fast(nz, 900)
    e = np.exp(-x * 30) + 0.6 * np.exp(-np.maximum(x - 0.012, 0) * 45) * (x > 0.012)
    return band * e * gain


def hat_sound(gain=0.022):
    ln = int(0.05 * SR)
    h = rng.standard_normal(ln) * np.exp(-np.arange(ln) / SR * 90) * gain
    return h - lowpass_fast(h, 7000)


# Kick on every beat, claps on 2 and 4, hats on the offbeats (sixteenths in the drop).
k = 0
while k * BEAT < total:
    bt = k * BEAT
    live = start_pulse - 0.01 <= bt < breath and not muted(bt)
    outro = end + 0.6 <= bt < ring
    if live or outro:
        place(music, kick_sound(), bt)
    if live and bt >= start_groove:
        if k % 4 in (1, 3):
            place(music, clap_sound(0.08 if drop <= bt < drop_end else 0.06), bt, pan=0.1)
        place(music, hat_sound(), bt + BEAT / 2, pan=0.3)
        if drop <= bt < drop_end:
            for q in (1, 3):
                place(music, hat_sound(0.014), bt + q * BEAT / 4, pan=-0.3)
    k += 1

# The drop: a sub boom and a crash.
ln = int(1.4 * SR)
x = np.arange(ln) / SR
boom = np.sin(2 * np.pi * np.cumsum(38 + 60 * np.exp(-x * 6)) / SR) * np.exp(-x * 2.2) * 0.32
place(music, boom, drop)
nz = rng.standard_normal(int(1.8 * SR))
crash = (nz - lowpass_fast(nz, 5000)) * np.exp(-np.arange(len(nz)) / SR * 2.4) * 0.05
place(music, crash, drop, pan=0.2)
# A short riser into the drop, and one into the end card.
for target, length in ((drop - BEAT, 1.2), (end, 1.6)):
    ln = int(length * SR)
    x = np.arange(ln) / SR
    noise = rng.standard_normal(ln)
    riser = (noise - lowpass_fast(noise, 1200)) * (x / x[-1]) ** 2.2 * 0.05
    place(music, riser, target - length)

# The ending: a held A minor 9 chord with a soft low A, ringing out under the fade.
ln = int((total - ring + 1) * SR)
x = np.arange(ln) / SR
chord = np.zeros(ln)
for m, g in ((33, 0.5), (45, 0.7), (57, 0.5), (60, 0.4), (64, 0.4), (67, 0.3), (71, 0.25)):
    for det in (-0.04, 0.04):
        f = note(m + det)
        chord += g * (np.sin(2 * np.pi * f * x) + 0.25 * np.sin(4 * np.pi * f * x)) * np.exp(-x * 0.7)
chord *= np.minimum(1, x * 40) * 0.018
place(music, chord, ring, pan=-0.1)
place(music, chord, ring + 0.013, pan=0.1)
place(music, kick_sound(0.3), ring)

# Fade the whole bed in, and out with the picture.
fade = np.ones(N)
fi = int(1.6 * SR)
fade[:fi] = np.linspace(0, 1, fi)
fo0, fo1 = int((total - 3.0) * SR), int(total * SR)
fade[fo0:fo1] = np.linspace(1, 0, fo1 - fo0) ** 1.5
fade[fo1:] = 0
music *= fade

# ----------------------------------------------------------------------------------
# Sound effects, from the composition's events

sfx = np.zeros((2, N))


def key_click(big=False):
    ln = int(0.03 * SR)
    x = np.arange(ln) / SR
    n = rng.standard_normal(ln)
    click = (n - lowpass_fast(n, 1800)) * np.exp(-x * 260)
    click += np.sin(2 * np.pi * rng.uniform(1900, 2600) * x) * np.exp(-x * 400) * 0.4
    return click * (0.07 if big else 0.045) * rng.uniform(0.75, 1.15)


def pop(big=False):
    ln = int(0.18 * SR)
    x = np.arange(ln) / SR
    f = 880 * (1 + 0.5 * (1 - np.exp(-x * 40)))
    s = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-x * 22)
    return s * (0.12 if big else 0.06)


def bead():
    ln = int(0.12 * SR)
    x = np.arange(ln) / SR
    f = rng.uniform(1500, 2100)
    s = (np.sin(2 * np.pi * f * x) + 0.5 * np.sin(2 * np.pi * f * 2.7 * x)) * np.exp(-x * 55)
    n = rng.standard_normal(ln) * np.exp(-x * 600) * 0.3
    return (s + n) * 0.09


def thock():
    ln = int(0.12 * SR)
    x = np.arange(ln) / SR
    s = np.sin(2 * np.pi * 170 * x) * np.exp(-x * 45)
    n = rng.standard_normal(ln)
    s += lowpass_fast(n, 1500) * np.exp(-x * 120) * 0.6
    return s * 0.16


def whoosh(length=0.55, gain=0.09):
    ln = int(length * SR)
    x = np.arange(ln) / SR
    n = rng.standard_normal(ln)
    band = lowpass_fast(n, 2600) - lowpass_fast(n, 300)
    shape = np.sin(np.pi * x / x[-1]) ** 2
    return band * shape * gain


def chime():
    ln = int(2.6 * SR)
    x = np.arange(ln) / SR
    s = np.zeros(ln)
    for m, g in ((81, 1.0), (88, 0.6), (93, 0.35)):
        f = note(m)
        s += g * (np.sin(2 * np.pi * f * x) + 0.2 * np.sin(2 * np.pi * f * 2.01 * x)) * np.exp(-x * 2.2)
    return s * 0.05 * np.minimum(1, x * 300)


for e in events:
    kind, t0 = e["kind"], e["t"]
    big = e.get("big", False)
    if kind == "key":
        place(sfx, key_click(big), t0, pan=rng.uniform(-0.2, 0.2))
    elif kind == "pop":
        place(sfx, pop(big), t0, pan=0.25)
    elif kind == "bead":
        place(sfx, bead(), t0, pan=rng.uniform(-0.3, 0.3))
    elif kind == "thock":
        place(sfx, thock(), t0)
    elif kind == "whoosh":
        place(sfx, whoosh(), t0 - 0.2)
    elif kind == "swish":
        place(sfx, whoosh(0.3, 0.05), t0 - 0.12, pan=0.4)
    elif kind in ("tick", "roll"):
        place(sfx, key_click() * 0.8, t0)
    elif kind == "chime":
        place(sfx, chime(), t0)


def write(path, track):
    peak = np.max(np.abs(track)) or 1
    if peak > 0.98:
        track = track * (0.98 / peak)
    data = (track.T * 32767).astype("<i2").tobytes()
    with wave.open(str(path), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(data)
    print(f"{path.name}: {track.shape[1] / SR:.1f}s, peak {peak:.2f}")


write(BUILD / "music.wav", music)
write(BUILD / "sfx.wav", sfx)

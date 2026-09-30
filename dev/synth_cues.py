"""Synthesize the daemon's session cues into ``client/myna-desktop/sounds/``.

    uv run --project server python dev/synth_cues.py

Start and stop are a marimba-like perfect fifth, E5 to B5 rising and the
reverse falling, the stop 3 dB quieter; error is a low, slightly detuned A3
double note, so it differs from both in register, roughness and rhythm. Each
note is a few exponentially decaying partials under a raised-cosine onset,
and the clip ends in a raised-cosine release to exact silence: a clip that
stops above zero clicks. Encoded as mono Ogg Vorbis with ffmpeg.
"""

from __future__ import annotations

import subprocess
import tempfile
import wave
from pathlib import Path

import numpy as np
import numpy.typing as npt

RATE = 48_000
OUT = Path(__file__).resolve().parent.parent / "client" / "myna-desktop" / "sounds"

# (frequency ratio, amplitude, decay scale) per partial.
MARIMBA = [(1.0, 1.0, 1.0), (3.93, 0.35, 0.35), (9.2, 0.12, 0.15)]
BELL = [(1.0, 1.0, 1.0), (2.0, 0.4, 0.6), (3.0, 0.15, 0.4), (4.2, 0.08, 0.25)]

Signal = npt.NDArray[np.float64]


def hz(note: str) -> float:
    semis = {"C": -9, "D": -7, "E": -5, "F": -4, "G": -2, "A": 0, "B": 2}[note[0]]
    return 440.0 * 2 ** ((semis + 12 * (int(note[-1]) - 4)) / 12)


def tone(f0: float, partials: list[tuple[float, float, float]], detune: float) -> Signal:
    t = np.arange(int(0.39 * RATE)) / RATE
    out = np.zeros_like(t)
    for ratio, amp, scale in partials:
        env = amp * np.exp(-t / (0.09 * scale))
        out += env * np.sin(2 * np.pi * f0 * ratio * t)
        if detune:
            out += 0.6 * env * np.sin(2 * np.pi * f0 * ratio * (1 + detune) * t)
    onset = np.minimum(t / 0.008, 1.0)
    return out * onset * 0.5 * (1 - np.cos(np.pi * onset))


def notes(
    names: list[str],
    gap: float,
    partials: list[tuple[float, float, float]],
    detune: float = 0.0,
) -> Signal:
    step = int(gap * RATE)
    parts = [tone(hz(name), partials, detune) for name in names]
    out = np.zeros(step * (len(parts) - 1) + len(parts[0]))
    for i, part in enumerate(parts):
        out[i * step : i * step + len(part)] += part
    return out


def write(name: str, x: Signal, gain_db: float = 0.0) -> None:
    fade = int(0.04 * RATE)
    x = x.copy()
    x[-fade:] *= 0.5 * (1 + np.cos(np.pi * np.arange(fade) / fade))
    x *= 10 ** ((-6 + gain_db) / 20) / np.max(np.abs(x))
    with tempfile.TemporaryDirectory() as tmp:
        pcm = Path(tmp) / f"{name}.wav"
        with wave.open(str(pcm), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(RATE)
            w.writeframes((x * 32767).astype("<i2").tobytes())
        subprocess.run(
            ["ffmpeg", "-v", "error", "-y", "-i", pcm, "-c:a", "libvorbis"]
            + ["-q:a", "6", "-map_metadata", "-1", OUT / f"{name}.oga"],
            check=True,
        )


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    write("start", notes(["E5", "B5"], 0.085, MARIMBA))
    write("stop", notes(["B5", "E5"], 0.085, MARIMBA), gain_db=-3)
    write("error", notes(["A3", "A3"], 0.13, BELL, detune=0.018))


if __name__ == "__main__":
    main()

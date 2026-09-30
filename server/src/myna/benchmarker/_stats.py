"""Point statistics for the aggregate table: RTFx, sample floors, repeat noise.

A clip's repeats are folded into one ``ClipSample`` (summed edit and audio
counts, every latency kept) because the clip, not the row, is the unit a
reader can generalise over; ``_bootstrap`` resamples these. Stdlib only, so
``summarize --no-ci`` runs where numpy is missing.
"""

from __future__ import annotations

import statistics
from collections.abc import Sequence
from dataclasses import dataclass

# Below these counts a tail percentile is one or two samples wearing a label:
# the p95 of 20 values is the second largest. Refuse it rather than print it.
# Latency floors count timed clips, not rows: repeats of 20 clips still put
# the p95 on the slowest one or two of them.
SAMPLE_FLOORS = ((0.99, 300), (0.95, 60))


@dataclass(frozen=True)
class ClipSample:
    """One clip's measured repeats in one cell, folded.

    ``audio_seconds``/``processing_seconds`` cover only the rows that measured
    throughput (a realtime feed's decode time is set by the pace), so they are
    not the same audio as the accuracy counts.
    """

    wer_edits: int = 0
    ref_words: int = 0
    cer_edits: int = 0
    ref_chars: int = 0
    audio_seconds: float = 0.0
    processing_seconds: float = 0.0
    latencies: tuple[float, ...] = ()
    # The secondary (Whisper-normalised) counts; False once any repeat lacks them.
    wer_whisper_edits: int = 0
    ref_words_whisper: int = 0
    cer_whisper_edits: int = 0
    ref_chars_whisper: int = 0
    whisper_scored: bool = True


def sample_floor(q: float) -> int:
    """Fewest samples a ``q`` percentile may be read from."""
    for tail, floor in SAMPLE_FLOORS:
        if q >= tail:
            return floor
    return 1


def nearest_rank(values: Sequence[float], q: float) -> float | None:
    """The sorted value at rank ``int(q * n)``, clamped to the last one."""
    if not values:
        return None
    s = sorted(values)
    return s[min(len(s) - 1, int(q * len(s)))]


def percentile(values: Sequence[float], q: float) -> float | None:
    """``nearest_rank``, or None if there are too few samples for ``q``."""
    return nearest_rank(values, q) if len(values) >= sample_floor(q) else None


def latency_percentile(clips: Sequence[ClipSample], q: float) -> float | None:
    """``nearest_rank`` of every repeat's latency, or None when fewer clips
    than the floor for ``q`` were timed."""
    timed = [c for c in clips if c.latencies]
    if len(timed) < sample_floor(q):
        return None
    return nearest_rank([v for c in timed for v in c.latencies], q)


def rtfx(clips: Sequence[ClipSample]) -> float | None:
    """Total audio over total processing: the Open ASR Leaderboard RTFx.

    Unlike a mean of per-clip ratios it weights each clip by its length, so it
    is the throughput a queue of these clips would see at batch size 1.
    """
    processing = sum(c.processing_seconds for c in clips)
    return sum(c.audio_seconds for c in clips) / processing if processing > 0 else None


def repeat_cv(groups: Sequence[Sequence[float]]) -> float | None:
    """Median within-clip coefficient of variation over clips with 2+ repeats.

    The spread between repeats of the same audio is the machine, not the
    model: a large value says the timings in this cell are noise-dominated.
    """
    cvs = [
        statistics.stdev(g) / statistics.fmean(g)
        for g in groups
        if len(g) > 1 and statistics.fmean(g) > 0
    ]
    return statistics.median(cvs) if cvs else None

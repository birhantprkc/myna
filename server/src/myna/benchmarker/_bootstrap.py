"""Bootstrap intervals and paired tests over clips.

The unit of resampling is the clip. A clip's repeats share its audio, so they
are correlated; drawing them as independent rows would shrink every interval
for nothing. The bootstrap draws folded ``ClipSample``s with replacement, so a
drawn clip brings all its repeats along.

Every statistic is a function of per-clip draw counts: a ratio metric is
``counts @ numerator / counts @ denominator``, and a percentile is the
nearest-rank value of the latency multiset in which each latency appears as
often as its clip was drawn. That gives the resampled statistics without
materialising B pooled vectors, and the same nearest-rank rule as the point
estimate the table prints.

numpy is imported here and nowhere else in the benchmarker, and only the
interval paths import this module, so the sweep and bench paths of
``myna-bench.pyz`` keep running on a machine without it.
"""

from __future__ import annotations

import math
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from myna.benchmarker._stats import ClipSample, sample_floor

RESAMPLES = 10_000
SEED = 0
CONFIDENCE = 0.95

# Resamples drawn at once; bounds the (chunk x clips) count matrix. A
# percentile's cumsum is (chunk x latencies) int64, the dominant cost: 2620
# clips x 5 repeats took ~5 s and ~470 MB peak per cell (2026-09-29).
_CHUNK = 1_000

Floats = NDArray[np.float64]
Counts = NDArray[np.int64]


@dataclass(frozen=True)
class Interval:
    """A point estimate and its bootstrap interval; all None when unscored."""

    estimate: float | None
    low: float | None
    high: float | None


@dataclass(frozen=True)
class Delta:
    """A paired difference (first minus second) over the clips both measured."""

    estimate: float | None
    low: float | None
    high: float | None
    p_value: float | None
    clips: int


NO_INTERVAL = Interval(None, None, None)


def _draws(n: int, resamples: int, seed: int) -> list[Counts]:
    """Per-clip draw counts for ``resamples`` bootstrap samples of ``n`` clips."""
    rng = np.random.default_rng(seed)
    chunks = []
    for start in range(0, resamples, _CHUNK):
        rows = min(_CHUNK, resamples - start)
        picks = rng.integers(n, size=(rows, n)) + (np.arange(rows) * n)[:, None]
        chunks.append(np.bincount(picks.ravel(), minlength=rows * n).reshape(rows, n))
    return chunks


def _ratio(num: Floats, den: Floats) -> Callable[[Counts], Floats]:
    def statistic(counts: Counts) -> Floats:
        top, bottom = counts @ num, counts @ den
        ratio: Floats = np.full(len(counts), np.nan)
        np.divide(top, bottom, out=ratio, where=bottom > 0)
        return ratio

    return statistic


def _quantile(latencies: Sequence[tuple[float, ...]], q: float) -> Callable[[Counts], Floats]:
    """Nearest-rank ``q`` of the pooled latencies, each weighted by its clip's
    draw count. NaN where the draw holds too few timed clips for ``q``."""
    timed = np.array([1 if lat else 0 for lat in latencies], dtype=np.int64)
    owner = np.array([i for i, lat in enumerate(latencies) for _ in lat])
    values = np.array([v for lat in latencies for v in lat])
    order = np.argsort(values)
    values, owner = values[order], owner[order]
    floor = sample_floor(q)

    def statistic(counts: Counts) -> Floats:
        if not len(values):
            return np.full(len(counts), np.nan)
        cumulative = np.cumsum(counts[:, owner], axis=1)
        total = cumulative[:, -1]
        # q < 1, so the rank is always inside the draw: no clamp needed.
        rank = np.floor(q * total)
        picked = values[(cumulative > rank[:, None]).argmax(axis=1)]
        return np.where(counts @ timed >= floor, picked, np.nan)

    return statistic


def _replicates(statistic: Callable[[Counts], Floats], draws: list[Counts]) -> Floats:
    return np.concatenate([statistic(c) for c in draws])


def _bounds(replicates: Floats) -> tuple[float | None, float | None]:
    kept = replicates[~np.isnan(replicates)]
    if not len(kept):
        return None, None
    tail = (1 - CONFIDENCE) / 2
    low, high = np.quantile(kept, [tail, 1 - tail])
    return float(low), float(high)


def _point(statistic: Callable[[Counts], Floats], n: int) -> float | None:
    value = float(statistic(np.ones((1, n), dtype=np.int64)).item())
    return None if math.isnan(value) else value


def _metrics(clips: Sequence[ClipSample]) -> dict[str, Callable[[Counts], Floats]]:
    def column(field: str) -> Floats:
        return np.array([getattr(c, field) for c in clips])

    latencies = [c.latencies for c in clips]
    return {
        "wer": _ratio(column("wer_edits"), column("ref_words")),
        "cer": _ratio(column("cer_edits"), column("ref_chars")),
        "rtfx": _ratio(column("audio_seconds"), column("processing_seconds")),
        "p50_final": _quantile(latencies, 0.5),
        "p95_final": _quantile(latencies, 0.95),
    }


def cell_intervals(
    clips: Sequence[ClipSample], *, resamples: int = RESAMPLES, seed: int = SEED
) -> dict[str, Interval]:
    """Percentile-bootstrap intervals for one cell's metrics, clips resampled."""
    metrics = _metrics(clips)
    if not clips:
        return dict.fromkeys(metrics, NO_INTERVAL)
    draws = _draws(len(clips), resamples, seed)
    out: dict[str, Interval] = {}
    for name, statistic in metrics.items():
        estimate = _point(statistic, len(clips))
        if estimate is None:
            out[name] = NO_INTERVAL
            continue
        out[name] = Interval(estimate, *_bounds(_replicates(statistic, draws)))
    return out


def paired(
    first: Mapping[str, ClipSample],
    second: Mapping[str, ClipSample],
    *,
    resamples: int = RESAMPLES,
    seed: int = SEED,
) -> dict[str, Delta]:
    """Paired bootstrap of ``first - second`` over the clips both measured.

    Both systems are resampled with the same draws, so clip difficulty cancels
    and only the difference between them varies. The p-value is two-sided,
    from the replicates re-centred on zero: how often a null world with this
    much clip-to-clip noise shows a difference at least this large.
    """
    common = sorted(first.keys() & second.keys())
    if not common:
        raise ValueError("no clip was measured by both systems")
    # A latency delta only over clips timed on both sides, or the two medians
    # of a draw would pool different clips.
    timed = [c for c in common if first[c].latencies and second[c].latencies]
    return {
        "wer": _delta("wer", first, second, common, resamples, seed),
        "cer": _delta("cer", first, second, common, resamples, seed),
        "p50_final": _delta("p50_final", first, second, timed, resamples, seed),
    }


def _delta(
    name: str,
    first: Mapping[str, ClipSample],
    second: Mapping[str, ClipSample],
    clips: Sequence[str],
    resamples: int,
    seed: int,
) -> Delta:
    if not clips:
        return Delta(None, None, None, None, 0)
    a = _metrics([first[c] for c in clips])[name]
    b = _metrics([second[c] for c in clips])[name]
    pa, pb = _point(a, len(clips)), _point(b, len(clips))
    if pa is None or pb is None:
        return Delta(None, None, None, None, len(clips))
    estimate = pa - pb
    draws = _draws(len(clips), resamples, seed)
    deltas = _replicates(a, draws) - _replicates(b, draws)
    deltas = deltas[~np.isnan(deltas)]
    extreme = int(np.count_nonzero(np.abs(deltas - estimate) >= abs(estimate)))
    p_value = (1 + extreme) / (1 + len(deltas))
    return Delta(estimate, *_bounds(deltas), p_value, len(clips))

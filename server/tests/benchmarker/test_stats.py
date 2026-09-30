"""Uncertainty for `myna-bench summarize` and `compare`.

A point estimate is only publishable with its interval, so the interval
machinery is pinned against known distributions: seeded simulations check that
a 95% interval covers the true value about 95% of the time, and that the paired
test finds a planted difference and does not invent one.
"""

from __future__ import annotations

import math
from dataclasses import replace

import numpy as np
import pytest

from myna.benchmarker._bootstrap import Interval, _bounds, cell_intervals, paired
from myna.benchmarker._stats import (
    ClipSample,
    latency_percentile,
    nearest_rank,
    percentile,
    repeat_cv,
    rtfx,
    sample_floor,
)

# Enough resamples for a stable interval, few enough to keep a simulation of
# hundreds of datasets inside a second or two.
QUICK = 400


def clip(**fields) -> ClipSample:
    return ClipSample(**fields)


# ─── point estimates ────────────────────────────────────────────────────────


def test_percentile_is_the_nearest_rank_the_table_always_used():
    values = [5.0, 1.0, 4.0, 2.0, 3.0]
    assert percentile(values, 0.5) == 3.0
    assert percentile(values, 0.0) == 1.0
    assert percentile(values, 0.9) == 5.0


@pytest.mark.parametrize(
    ("q", "expected"),
    [(0.0, 1.0), (0.5, 3.0), (0.95, 5.0), (1.0, 5.0)],
)
def test_nearest_rank_indexes_the_sorted_values_and_clamps_at_the_top(q, expected):
    assert nearest_rank([5.0, 1.0, 4.0, 2.0, 3.0], q) == expected


def test_nearest_rank_of_nothing_is_none():
    assert nearest_rank([], 0.5) is None


def test_percentile_of_nothing_is_none():
    assert percentile([], 0.5) is None


@pytest.mark.parametrize(
    ("q", "floor"),
    [(0.5, 1), (0.95, 60), (0.99, 300)],
)
def test_tail_percentiles_have_sample_floors(q, floor):
    assert sample_floor(q) == floor


@pytest.mark.parametrize(("q", "n"), [(0.95, 59), (0.99, 299)])
def test_a_tail_percentile_below_its_floor_is_refused(q, n):
    assert percentile([1.0] * n, q) is None


@pytest.mark.parametrize(("q", "n"), [(0.95, 60), (0.99, 300)])
def test_a_tail_percentile_at_its_floor_is_reported(q, n):
    assert percentile([1.0] * n, q) == 1.0


def test_a_latency_floor_counts_timed_clips_not_repeats():
    """Repeats of one audio are correlated: 20 clips x 3 repeats is still 20."""
    assert latency_percentile([clip(latencies=(1.0, 1.0, 1.0))] * 20, 0.95) is None
    assert latency_percentile([clip(latencies=(1.0,))] * 60, 0.95) == 1.0


def test_an_untimed_clip_does_not_count_toward_the_floor():
    clips = [clip(latencies=(1.0,))] * 59 + [clip()]
    assert latency_percentile(clips, 0.95) is None


def test_a_latency_percentile_pools_every_repeat():
    clips = [clip(latencies=(float(i), float(i) + 0.5)) for i in range(60)]
    pooled = [v for c in clips for v in c.latencies]
    assert latency_percentile(clips, 0.95) == percentile(pooled, 0.95)


def test_rtfx_is_total_audio_over_total_processing_not_a_mean_of_ratios():
    clips = [
        clip(audio_seconds=10.0, processing_seconds=1.0),  # 10x
        clip(audio_seconds=1.0, processing_seconds=1.0),  # 1x
    ]
    assert rtfx(clips) == pytest.approx(11.0 / 2.0)


def test_rtfx_without_timed_audio_is_none():
    assert rtfx([clip(), clip()]) is None


def test_repeat_cv_is_the_median_within_clip_spread():
    # CVs 0 (identical repeats), 0.1 and 0.2 (sample std / mean) -> median 0.1.
    groups = [(2.0, 2.0), (1.8, 2.2, 2.0), (1.6, 2.4, 2.0)]
    assert repeat_cv(groups) == pytest.approx(0.1)


def test_repeat_cv_skips_a_clip_whose_repeats_average_zero():
    assert repeat_cv([(0.0, 0.0), (1.8, 2.2, 2.0)]) == pytest.approx(0.1)


def test_repeat_cv_needs_a_clip_with_two_repeats():
    assert repeat_cv([(1.0,), (2.0,)]) is None


# ─── cell intervals ─────────────────────────────────────────────────────────


def test_intervals_are_reproducible_under_the_seed():
    clips = [clip(wer_edits=i % 3, ref_words=10) for i in range(40)]
    first = cell_intervals(clips, resamples=QUICK, seed=7)
    again = cell_intervals(clips, resamples=QUICK, seed=7)
    assert first == again


def test_an_interval_brackets_its_estimate():
    clips = [clip(wer_edits=i % 4, ref_words=12) for i in range(50)]
    wer = cell_intervals(clips, resamples=QUICK, seed=0)["wer"]
    assert wer.low <= wer.estimate <= wer.high
    assert wer.low < wer.high


def test_a_tail_interval_below_its_floor_is_refused():
    clips = [clip(latencies=(0.1 * i,)) for i in range(59)]
    intervals = cell_intervals(clips, resamples=QUICK, seed=0)
    assert intervals["p95_final"].estimate is None
    assert intervals["p95_final"].low is None
    assert intervals["p50_final"].estimate is not None


def test_no_clips_have_no_intervals():
    intervals = cell_intervals([], resamples=QUICK, seed=0)
    assert set(intervals.values()) == {Interval(None, None, None)}


def test_a_point_no_resample_can_reproduce_keeps_its_estimate_without_bounds():
    """The only timed clip can go undrawn: then no resample has a median."""
    clips = [clip(latencies=(0.1,)), clip()]
    for seed in range(64):
        p50 = cell_intervals(clips, resamples=1, seed=seed)["p50_final"]
        if p50.low is None:
            break
    else:
        pytest.fail("no seed left the latency clip undrawn")
    assert (p50.estimate, p50.high) == (0.1, None)


def test_a_one_word_reference_still_scores():
    assert cell_intervals([clip(wer_edits=1, ref_words=1)], resamples=QUICK)["wer"].estimate == 1.0


def test_a_draw_of_only_empty_references_is_left_out_not_infinite():
    """Insertions against an empty reference have no rate; a resample that
    drew only that clip says nothing about WER, rather than infinitely much."""
    clips = [clip(wer_edits=3, ref_words=0), clip(wer_edits=1, ref_words=10)]
    wer = cell_intervals(clips, resamples=QUICK, seed=0)["wer"]
    assert math.isfinite(wer.high)


def test_the_bounds_are_the_central_95_percent_of_the_replicates():
    assert _bounds(np.arange(1001.0)) == pytest.approx((25.0, 975.0))


def test_unscored_metrics_have_no_interval():
    intervals = cell_intervals([clip(), clip()], resamples=QUICK, seed=0)
    assert intervals["wer"].estimate is None
    assert intervals["rtfx"].estimate is None
    assert intervals["p50_final"].estimate is None


def test_the_whisper_normalised_rates_get_intervals_too():
    clips = [
        clip(wer_whisper_edits=1, ref_words_whisper=4, cer_whisper_edits=2, ref_chars_whisper=10)
    ]
    intervals = cell_intervals(clips * 3, resamples=QUICK, seed=0)
    assert intervals["wer_whisper"] == Interval(0.25, 0.25, 0.25)
    assert intervals["cer_whisper"] == Interval(0.2, 0.2, 0.2)


def test_a_cell_with_an_unscored_clip_has_no_whisper_interval():
    """As in the table: an interval over the scored subset is not the cell's."""
    clips = [clip(wer_whisper_edits=1, ref_words_whisper=4), clip(whisper_scored=False)]
    intervals = cell_intervals(clips, resamples=QUICK, seed=0)
    assert intervals["wer_whisper"] == Interval(None, None, None)
    assert intervals["cer_whisper"] == Interval(None, None, None)


def test_the_paired_test_compares_the_whisper_rates():
    a = {f"c{i}": clip(ref_words_whisper=10, ref_chars_whisper=40) for i in range(10)}
    b = {cid: replace(s, wer_whisper_edits=1, cer_whisper_edits=2) for cid, s in a.items()}
    deltas = paired(b, a, resamples=QUICK, seed=0)
    assert deltas["wer_whisper"].estimate == pytest.approx(0.1)
    assert deltas["cer_whisper"].estimate == pytest.approx(0.05)


def test_a_clips_repeats_travel_together():
    """Identical repeats are one observation, not five: resampling them as
    independent rows would narrow the interval five-fold for nothing."""
    rng = np.random.default_rng(3)
    base = [(int(rng.integers(0, 5)), int(rng.integers(8, 20))) for _ in range(40)]
    once = [clip(wer_edits=e, ref_words=w) for e, w in base]
    five = [clip(wer_edits=5 * e, ref_words=5 * w) for e, w in base]
    a = cell_intervals(once, resamples=QUICK, seed=1)["wer"]
    b = cell_intervals(five, resamples=QUICK, seed=1)["wer"]
    assert (a.low, a.high) == pytest.approx((b.low, b.high))


def test_the_resampled_percentile_pools_every_repeat_of_a_drawn_clip():
    """Drawn twice, a clip contributes both copies of each of its latencies."""
    clips = [clip(latencies=(float(i), float(i) + 0.5)) for i in range(60)]
    point = cell_intervals(clips, resamples=QUICK, seed=0)["p50_final"].estimate
    assert point == percentile([v for c in clips for v in c.latencies], 0.5)


@pytest.mark.parametrize("n", [59, 61, 63])
def test_the_resampled_median_uses_the_tables_rank_on_an_odd_count(n):
    """q * n not an integer: rounding the rank up would pick the next value."""
    clips = [clip(latencies=(float(i),)) for i in range(n)]
    point = cell_intervals(clips, resamples=QUICK, seed=0)["p50_final"].estimate
    assert point == latency_percentile(clips, 0.5)


@pytest.mark.parametrize("n", range(61, 80))
def test_the_resampled_p95_uses_the_tables_rank(n):
    clips = [clip(latencies=(float(i),)) for i in range(n)]
    point = cell_intervals(clips, resamples=QUICK, seed=0)["p95_final"].estimate
    assert point == latency_percentile(clips, 0.95)


def test_the_resampled_p95_floor_counts_clips_not_repeats():
    few = [clip(latencies=(float(i), float(i), float(i))) for i in range(20)]
    assert cell_intervals(few, resamples=QUICK, seed=0)["p95_final"] == Interval(None, None, None)


def _covered(interval, truth) -> bool:
    return interval.low <= truth <= interval.high


def test_the_wer_interval_covers_the_true_rate_at_its_nominal_level():
    rng = np.random.default_rng(2026)
    truth, sims, hits = 0.08, 300, 0
    for sim in range(sims):
        words = rng.integers(5, 40, size=60)
        edits = rng.binomial(words, truth)
        clips = [
            clip(wer_edits=int(e), ref_words=int(w)) for e, w in zip(edits, words, strict=True)
        ]
        hits += _covered(cell_intervals(clips, resamples=QUICK, seed=sim)["wer"], truth)
    assert 0.90 <= hits / sims <= 0.99


def test_the_rtfx_interval_covers_the_true_throughput_at_its_nominal_level():
    rng = np.random.default_rng(11)
    sims, hits = 300, 0
    # Processing = audio * rtf, rtf lognormal and independent of the audio, so
    # the population RTFx is E[audio] / E[audio * rtf] = 1 / E[rtf].
    mu, sigma = math.log(0.05), 0.4
    truth = 1.0 / math.exp(mu + sigma**2 / 2)
    for sim in range(sims):
        audio = rng.uniform(2.0, 20.0, size=60)
        rtf = rng.lognormal(mu, sigma, size=60)
        clips = [
            clip(audio_seconds=float(a), processing_seconds=float(a * r))
            for a, r in zip(audio, rtf, strict=True)
        ]
        hits += _covered(cell_intervals(clips, resamples=QUICK, seed=sim)["rtfx"], truth)
    assert 0.90 <= hits / sims <= 0.99


def test_the_median_latency_interval_covers_the_true_median():
    rng = np.random.default_rng(5)
    sims, hits, scale = 300, 0, 0.4
    truth = scale * math.log(2)  # median of an exponential
    for sim in range(sims):
        draws = rng.exponential(scale, size=80)
        clips = [clip(latencies=(float(d),)) for d in draws]
        hits += _covered(cell_intervals(clips, resamples=QUICK, seed=sim)["p50_final"], truth)
    assert 0.90 <= hits / sims <= 0.99


# ─── paired comparison ──────────────────────────────────────────────────────


def _system(rng, words, rate):
    return {
        f"c{i}": clip(
            wer_edits=int(rng.binomial(w, rate)),
            ref_words=int(w),
            latencies=(float(rng.exponential(0.3)),),
        )
        for i, w in enumerate(words)
    }


def test_the_paired_test_detects_a_planted_difference():
    rng = np.random.default_rng(9)
    words = rng.integers(5, 40, size=120)
    a = _system(rng, words, 0.05)
    # B makes three more errors on every fourth clip and is 0.2 s slower.
    b = {
        cid: clip(
            wer_edits=s.wer_edits + (3 if i % 4 == 0 else 0),
            ref_words=s.ref_words,
            latencies=tuple(v + 0.2 for v in s.latencies),
        )
        for i, (cid, s) in enumerate(a.items())
    }
    deltas = paired(b, a, resamples=2000, seed=0)
    wer = deltas["wer"]
    assert wer.estimate > 0
    assert wer.low > 0
    assert wer.p_value < 0.01
    assert deltas["p50_final"].estimate == pytest.approx(0.2)
    assert deltas["p50_final"].p_value < 0.01
    assert wer.clips == 120


def test_a_difference_no_resample_undoes_has_the_smallest_p_its_resamples_allow():
    """Every replicate of a pure shift is the shift itself, so none is as
    extreme as zero: p is 1 / (B + 1), whatever B is, chunked or not."""
    a = {f"c{i}": clip(latencies=(0.1 * i,)) for i in range(20)}
    b = {cid: clip(latencies=tuple(v + 1.0 for v in s.latencies)) for cid, s in a.items()}
    assert paired(b, a, resamples=1500, seed=0)["p50_final"].p_value == 1 / 1501


def test_the_paired_test_is_reproducible_under_the_seed():
    rng = np.random.default_rng(4)
    words = rng.integers(5, 40, size=30)
    a, b = _system(rng, words, 0.1), _system(rng, words, 0.12)
    assert paired(a, b, resamples=QUICK, seed=3) == paired(a, b, resamples=QUICK, seed=3)


def test_the_paired_test_does_not_invent_a_difference():
    """Two draws of the same system: rejections at 5% stay near 5%."""
    rng = np.random.default_rng(13)
    sims, rejected = 200, 0
    for sim in range(sims):
        words = rng.integers(5, 40, size=60)
        a = _system(rng, words, 0.1)
        b = _system(rng, words, 0.1)
        rejected += paired(a, b, resamples=QUICK, seed=sim)["wer"].p_value < 0.05
    assert rejected / sims <= 0.09


def test_identical_systems_have_no_difference_at_all():
    rng = np.random.default_rng(1)
    a = _system(rng, rng.integers(5, 40, size=30), 0.1)
    wer = paired(a, a, resamples=QUICK, seed=0)["wer"]
    assert (wer.estimate, wer.low, wer.high, wer.p_value) == (0.0, 0.0, 0.0, 1.0)


def test_the_paired_test_only_uses_clips_both_systems_measured():
    a = {"c1": clip(wer_edits=1, ref_words=10), "c2": clip(wer_edits=9, ref_words=10)}
    b = {"c1": clip(wer_edits=2, ref_words=10), "c3": clip(wer_edits=0, ref_words=10)}
    wer = paired(a, b, resamples=QUICK, seed=0)["wer"]
    assert wer.clips == 1
    assert wer.estimate == pytest.approx(-0.1)


def test_the_paired_test_refuses_disjoint_clips():
    with pytest.raises(ValueError, match="^no clip was measured by both systems$"):
        paired({"c1": clip()}, {"c2": clip()}, resamples=QUICK, seed=0)


def test_a_paired_latency_needs_latencies_on_both_sides():
    a = {"c1": clip(wer_edits=1, ref_words=10, latencies=(0.3,))}
    b = {"c1": clip(wer_edits=1, ref_words=10)}
    deltas = paired(a, b, resamples=QUICK, seed=0)
    latency = deltas["p50_final"]
    assert (latency.estimate, latency.p_value, latency.clips) == (None, None, 0)
    assert deltas["wer"].clips == 1


def test_a_paired_latency_leaves_out_a_clip_timed_on_one_side():
    """Both medians of a draw must pool the same clips: a's extra fast clip
    would otherwise shift its median by one rank and fake a difference."""
    a = {f"c{i}": clip(latencies=(float(i),)) for i in range(1, 11)}
    b = dict(a)
    a["c0"] = clip(latencies=(0.1,))
    b["c0"] = clip()
    latency = paired(a, b, resamples=QUICK, seed=0)["p50_final"]
    assert (latency.estimate, latency.clips) == (0.0, 10)

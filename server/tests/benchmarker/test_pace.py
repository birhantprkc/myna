"""The pace axis: which feeds a cell runs at, and whether the feed kept up.

A real-time row's latency is only a user's latency if the client delivered
audio on the capture clock. A harness that fell behind (a GC pause, a blocked
event loop, a saturated box) feeds a burst the backend never sees live, and the
finalize latency it measures belongs to neither pace.
"""

from __future__ import annotations

import pytest

from myna.benchmarker._pace import (
    MAX,
    REALTIME,
    feed_lag,
    paced_label,
    paces_for,
    parse_paces,
    starved,
)
from myna.testbed import FedChunk


def feed(*times, chunk=0.1, start=0.0):
    """Chunk k of ``chunk`` seconds, issued at ``times[k]``."""
    return [FedChunk(t=start + t, audio_end=round((k + 1) * chunk, 6)) for k, t in enumerate(times)]


def test_an_on_time_feed_has_no_lag():
    assert feed_lag(feed(0.1, 0.2, 0.3, 0.4)) == pytest.approx(0.0)


def test_lag_is_measured_from_the_first_chunk_so_session_setup_is_not_lag():
    assert feed_lag(feed(0.1, 0.2, 0.3, start=2.5)) == pytest.approx(0.0)


def test_lag_is_the_worst_delay_behind_the_capture_clock():
    assert feed_lag(feed(0.1, 0.2, 0.38, 0.4, 0.5)) == pytest.approx(0.08)


def test_a_feed_running_early_is_not_negative_lag():
    # Pacing never sends early; if a feed did, it is not starvation either.
    assert feed_lag(feed(0.1, 0.15, 0.2)) == pytest.approx(0.0)


def test_no_feed_has_no_lag():
    assert feed_lag([]) is None


def test_falling_behind_by_under_one_chunk_is_jitter():
    assert not starved(feed(0.1, 0.2, 0.39, 0.4))


def test_falling_behind_by_more_than_one_chunk_is_starvation():
    assert starved(feed(0.1, 0.2, 0.41, 0.42))


def test_one_chunk_is_the_feeds_own_chunk_size():
    assert not starved(feed(0.25, 0.5, 0.95, chunk=0.25))
    assert starved(feed(0.25, 0.5, 1.05, chunk=0.25))


def test_an_empty_feed_is_not_starved():
    assert not starved([])


def test_the_default_pace_is_max_so_a_tester_run_stays_short():
    assert parse_paces({}, (MAX,), "cfg") == (MAX,)


def test_a_target_can_override_the_configs_paces():
    assert parse_paces({"pace": ["max", "realtime"]}, (MAX,), "t") == (MAX, REALTIME)


@pytest.mark.parametrize(
    ("value", "message"),
    [
        (["max", "fast"], "unknown pace"),
        ([], "at least one"),
        ("realtime", "a list"),
        (["max", "max"], "twice"),
    ],
)
def test_a_bad_pace_list_is_refused(value, message):
    with pytest.raises(SystemExit, match=message):
        parse_paces({"pace": value}, (MAX,), "t")


def test_only_streaming_takes_realtime():
    assert paces_for("batch", (MAX, REALTIME)) == (MAX,)
    assert paces_for("streaming", (MAX, REALTIME)) == (MAX, REALTIME)


def test_batch_runs_at_max_even_when_only_realtime_is_asked_for():
    assert paces_for("batch", (REALTIME,)) == (MAX,)
    assert paces_for("streaming", (REALTIME,)) == (REALTIME,)


def test_max_leaves_the_label_unmarked_so_old_rows_still_compare():
    assert paced_label("s/cpu/m/streaming", MAX) == "s/cpu/m/streaming"


def test_realtime_marks_the_label_once():
    assert paced_label("s/cpu/m/streaming", REALTIME) == "s/cpu/m/streaming@realtime"
    assert paced_label("s/cpu/m/streaming@realtime", REALTIME) == "s/cpu/m/streaming@realtime"

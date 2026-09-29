"""Repeats, warmup and clip order: what a cell runs, in which order."""

from __future__ import annotations

import pytest

from myna.benchmarker._schedule import (
    COLD,
    MEASURED,
    WARMUP,
    Schedule,
    parse_schedule,
    plan_clips,
)

CLIPS = [f"clip-{i}" for i in range(8)]


def order(schedule, clips=CLIPS):
    return [(item.clip, item.repeat, item.phase) for item in plan_clips(clips, schedule)]


def test_the_default_schedule_is_one_pass_in_manifest_order():
    assert order(Schedule()) == [(c, 0, MEASURED) for c in CLIPS]


def test_a_cold_sample_is_tagged_cold():
    assert order(Schedule(), ["c"])[0] == ("c", 0, MEASURED)
    assert [(i.clip, i.phase) for i in plan_clips(["c"], Schedule(), cold=True)] == [("c", COLD)]


def test_every_repeat_is_a_full_pass_over_the_corpus():
    items = plan_clips(CLIPS, Schedule(repeats=3))
    for repeat in range(3):
        assert sorted(i.clip for i in items if i.repeat == repeat) == sorted(CLIPS)
    assert len(items) == 3 * len(CLIPS)


def test_repeats_are_interleaved_passes_not_back_to_back_copies_of_a_clip():
    items = plan_clips(CLIPS, Schedule(repeats=3))
    assert [i.repeat for i in items] == sorted(i.repeat for i in items)


def test_each_repeat_is_shuffled_differently():
    items = plan_clips(CLIPS, Schedule(repeats=3))
    passes = [[i.clip for i in items if i.repeat == r] for r in range(3)]
    assert len({tuple(p) for p in passes}) == 3


def test_the_seeded_order_is_reproducible():
    assert order(Schedule(repeats=3, seed=7)) == order(Schedule(repeats=3, seed=7))


def test_a_different_seed_gives_a_different_order():
    assert order(Schedule(repeats=3, seed=1)) != order(Schedule(repeats=3, seed=2))


def test_the_order_is_pinned_across_interpreters():
    """String seeding hashes with SHA-512, not hash(), so PYTHONHASHSEED and a
    new Python cannot reshuffle a recorded run."""
    first = [i.clip for i in plan_clips(CLIPS, Schedule(repeats=2, seed=0)) if i.repeat == 0]
    assert first == [
        "clip-0",
        "clip-2",
        "clip-1",
        "clip-4",
        "clip-6",
        "clip-7",
        "clip-3",
        "clip-5",
    ]


def test_warmup_clips_run_first_and_are_tagged():
    items = order(Schedule(warmup_clips=2))
    assert items[:2] == [("clip-0", 0, WARMUP), ("clip-1", 0, WARMUP)]
    assert items[2:] == [(c, 0, MEASURED) for c in CLIPS]


def test_warmup_asks_for_more_clips_than_there_are_runs_them_all_once():
    assert order(Schedule(warmup_clips=5), ["a", "b"])[:2] == [("a", 0, WARMUP), ("b", 0, WARMUP)]
    assert len(plan_clips(["a", "b"], Schedule(warmup_clips=5))) == 4


def test_a_cold_sample_never_repeats_or_warms_up():
    items = plan_clips(["c"], Schedule(repeats=3, warmup_clips=2), cold=True)
    assert [(i.clip, i.repeat, i.phase) for i in items] == [("c", 0, COLD)]


def test_the_budget_scales_with_the_repeats():
    """The budget is per pass over the corpus; warmup runs outside it, as the
    cold sample does."""
    assert Schedule(repeats=3, warmup_clips=4).budget(600.0) == 1800.0
    assert Schedule().budget(600.0) == 600.0


def test_the_schedule_is_recorded_as_plain_data():
    assert Schedule(repeats=3, warmup_clips=2, seed=9).as_dict() == {
        "repeats": 3,
        "warmup_clips": 2,
        "seed": 9,
    }


# ─── config ──────────────────────────────────────────────────────────────────


def test_a_config_without_schedule_keys_is_the_default():
    assert parse_schedule({}, Schedule(), "bench.yaml") == Schedule()


def test_a_target_overrides_the_global_schedule_key_by_key():
    base = parse_schedule({"repeats": 3, "warmup_clips": 2, "seed": 5}, Schedule(), "bench.yaml")
    assert parse_schedule({"repeats": 5}, base, "myna-whisper") == Schedule(5, 2, 5)


@pytest.mark.parametrize(
    ("spec", "message"),
    [
        ({"repeats": 0}, "repeats"),
        ({"repeats": "3"}, "repeats"),
        ({"repeats": True}, "repeats"),
        ({"warmup_clips": -1}, "warmup_clips"),
        ({"warmup_clips": 1.5}, "warmup_clips"),
        ({"seed": "x"}, "seed"),
    ],
)
def test_a_bad_schedule_value_is_refused_by_name(spec, message):
    with pytest.raises(SystemExit, match=f"myna-whisper: {message}"):
        parse_schedule(spec, Schedule(), "myna-whisper")

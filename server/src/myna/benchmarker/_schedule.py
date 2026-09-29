"""What one cell runs: repeats of the corpus, discarded warmup, clip order.

One pass per clip gives no variance estimate, and the first warm clips pay
one-off costs (ORT kernel setup per new input shape, page cache) that would
otherwise land in measured rows. So a cell can run ``warmup_clips`` first,
tagged ``warmup`` and kept out of every aggregate, then ``repeats`` full passes
over the corpus.

Repeats are interleaved passes, each in its own seeded shuffle, rather than N
back-to-back copies of a clip: drift over the sweep (thermals, a background
job) then spreads across clips instead of biasing whichever clip it hit. One
pass keeps manifest order, so a tester's default run is unchanged.
"""

from __future__ import annotations

import random
from dataclasses import dataclass
from typing import Any

COLD = "cold"
WARMUP = "warmup"
MEASURED = "measured"


@dataclass(frozen=True)
class Schedule:
    repeats: int = 1
    warmup_clips: int = 0
    seed: int = 0

    def budget(self, per_pass: float) -> float:
        """A cell's usability deadline. Warmup runs outside it, like the cold
        sample, so the verdict is about the measured passes only."""
        return per_pass * self.repeats

    def as_dict(self) -> dict[str, int]:
        return {"repeats": self.repeats, "warmup_clips": self.warmup_clips, "seed": self.seed}


@dataclass(frozen=True)
class Planned[T]:
    clip: T
    repeat: int
    phase: str


def plan_clips[T](clips: list[T], schedule: Schedule, *, cold: bool = False) -> list[Planned[T]]:
    """Every clip run a cell makes, in order.

    A cold sample is one clip measured once from a fresh load, so neither
    warmup nor repeats apply to it.
    """
    if cold:
        return [Planned(clip, 0, COLD) for clip in clips]
    plan = [Planned(clip, 0, WARMUP) for clip in clips[: schedule.warmup_clips]]
    for repeat in range(schedule.repeats):
        order = list(clips)
        if schedule.repeats > 1:
            # A str seed is hashed with SHA-512, so the order does not depend
            # on PYTHONHASHSEED or the interpreter that replays it.
            random.Random(f"{schedule.seed}/{repeat}").shuffle(order)
        plan += [Planned(clip, repeat, MEASURED) for clip in order]
    return plan


def _int(value: Any, where: str, key: str, minimum: int | None) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise SystemExit(f"{where}: {key} must be an integer, got {value!r}")
    if minimum is not None and value < minimum:
        raise SystemExit(f"{where}: {key} must be at least {minimum}, got {value}")
    return value


def parse_schedule(spec: dict[str, Any], base: Schedule, where: str) -> Schedule:
    """``base`` with whatever of ``repeats``/``warmup_clips``/``seed`` ``spec`` sets."""
    return Schedule(
        repeats=_int(spec.get("repeats", base.repeats), where, "repeats", 1),
        warmup_clips=_int(spec.get("warmup_clips", base.warmup_clips), where, "warmup_clips", 0),
        seed=_int(spec.get("seed", base.seed), where, "seed", None),
    )

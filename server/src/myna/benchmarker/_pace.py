"""The pace a cell feeds audio at, and whether the feed kept to it.

``max`` feeds as fast as the socket accepts: what makes a full matrix
affordable, and the right pace for accuracy and throughput. ``realtime`` feeds
on the capture clock (``myna.testbed.sources.paced``), which is the only pace
at which streaming finalize and commit latency is what a dictating user sees.
Batch decodes after end-of-audio either way, so only streaming takes it.

A realtime row is only live latency if the client delivered on time. A sender
that fell more than one chunk behind the capture clock fed the backend a burst
no microphone would, so the row is flagged ``pace_starved`` and its latency is
kept out of the aggregate.
"""

from __future__ import annotations

from collections.abc import Sequence
from typing import Any

from myna.testbed import FedChunk

MAX = "max"
REALTIME = "realtime"
PACES = (MAX, REALTIME)


def parse_paces(spec: dict[str, Any], base: tuple[str, ...], where: str) -> tuple[str, ...]:
    """``base``, or the ``pace:`` list ``spec`` sets instead."""
    if "pace" not in spec:
        return base
    value = spec["pace"]
    if not isinstance(value, list):
        raise SystemExit(f"{where}: pace must be a list such as [max, realtime], got {value!r}")
    if not value:
        raise SystemExit(f"{where}: pace needs at least one of {list(PACES)}")
    unknown = [p for p in value if p not in PACES]
    if unknown:
        raise SystemExit(f"{where}: unknown pace {unknown}; expected {list(PACES)}")
    if len(set(value)) != len(value):
        raise SystemExit(f"{where}: pace names the same value twice: {value}")
    return tuple(value)


def paces_for(mode: str, paces: tuple[str, ...]) -> tuple[str, ...]:
    """The paces a cell in ``mode`` runs at: batch is always ``max``, once."""
    return paces if mode == "streaming" else (MAX,)


def paced_label(label: str, pace: str) -> str:
    """``label`` marked with its pace, so rows fed at different paces never
    share a summary row. ``max`` is unmarked, so rows from before the pace
    axis still compare; an already-marked label is left alone."""
    suffix = f"@{pace}"
    return label if pace == MAX or label.endswith(suffix) else label + suffix


def feed_lag(feed: Sequence[FedChunk]) -> float | None:
    """How far, at worst, the feed ran behind the capture clock, in seconds.

    Measured from the first chunk, so session setup before it is not lag.
    """
    if not feed:
        return None
    t0, a0 = feed[0].t, feed[0].audio_end
    return max(0.0, *((c.t - t0) - (c.audio_end - a0) for c in feed))


def starved(feed: Sequence[FedChunk]) -> bool:
    """Whether the feed fell more than one of its own chunks behind."""
    lag = feed_lag(feed)
    return lag is not None and lag > feed[0].audio_end

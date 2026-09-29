"""The timed event stream of every clip, kept beside the results file.

A result row carries the latencies we thought of when it was written. The raw
stream it was derived from lets a later metric (partial stability, time to
first correct word) be computed from data already on disk instead of from a
rerun. ``summarize`` never reads this file; ``load_events`` is its only reader.

One gzipped JSON line per clip, keyed like its result row by (label, clip,
repeat). Times are seconds on the harness's monotonic clock, shifted so the
first audio chunk was issued at 0; ``audio_start`` is that instant measured
from session open, which is the origin the row's latencies use. An event that
beat the first chunk has a negative time.
"""

from __future__ import annotations

import gzip
import json
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

from myna.benchmarker._summarize import SCHEMA_VERSION, Record
from myna.core import event_from_wire, event_to_wire
from myna.testbed import FedChunk, ResultRecord, TimedEvent


def events_path_for(out: Path) -> Path:
    """Sidecar path for a results file. One rule, so writer and reader agree."""
    return out.parent / (out.stem + "-events.jsonl.gz")


def event_line(
    record: ResultRecord,
    *,
    label: str,
    clip: str,
    repeat: int,
    cold: bool,
    machine: str | None,
) -> Record:
    """One clip's stream as a JSON-ready line."""
    start = record.feed[0].t if record.feed else 0.0
    return {
        "schema_version": SCHEMA_VERSION,
        "machine": machine,
        "label": label,
        "clip": clip,
        "repeat": repeat,
        "cold": cold,
        "started_at": record.started_at,
        "audio_start": start,
        "audio_end": None if record.audio_end_t is None else record.audio_end_t - start,
        "feed": [[c.t - start, c.audio_end] for c in record.feed],
        "events": [[te.t - start, event_to_wire(te.event)] for te in record.events],
    }


class EventsFile:
    """Append-only gzipped JSONL writer; appending adds a gzip member.

    Each line is its own member, flushed as written, so a sweep killed midway
    still leaves every finished clip readable.
    """

    def __init__(self, path: Path):
        path.parent.mkdir(parents=True, exist_ok=True)
        self._fp = path.open("ab")

    def write(self, record: Mapping[str, object]) -> None:
        self._fp.write(gzip.compress((json.dumps(record) + "\n").encode("utf-8")))
        self._fp.flush()

    def close(self) -> None:
        self._fp.close()


@dataclass(frozen=True)
class ClipEvents:
    """One clip's stream read back, times relative to the first audio chunk."""

    machine: str | None
    label: str
    clip: str
    repeat: int
    cold: bool
    started_at: str
    audio_start: float
    audio_end: float | None
    feed: tuple[FedChunk, ...]
    events: tuple[TimedEvent, ...]


def load_events(path: Path) -> list[ClipEvents]:
    """Every clip stream in a sidecar, in the order written."""
    out: list[ClipEvents] = []
    with gzip.open(path, "rt", encoding="utf-8") as fp:
        for line in fp:
            raw = json.loads(line)
            events = []
            for t, wire in raw["events"]:
                event = event_from_wire(wire)
                if event is not None:
                    events.append(TimedEvent(t=t, event=event))
            out.append(
                ClipEvents(
                    machine=raw["machine"],
                    label=raw["label"],
                    clip=raw["clip"],
                    repeat=raw["repeat"],
                    cold=raw["cold"],
                    started_at=raw["started_at"],
                    audio_start=raw["audio_start"],
                    audio_end=raw["audio_end"],
                    feed=tuple(FedChunk(t=t, audio_end=end) for t, end in raw["feed"]),
                    events=tuple(events),
                )
            )
    return out

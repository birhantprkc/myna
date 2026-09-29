"""Adapter and candidate definitions.

A *candidate* is a specific (model, engine, streaming strategy) combination
under evaluation. An *adapter* wraps one candidate behind the IE114-shaped
``SttService`` interface so the harness can drive it without knowing anything
model-specific.
"""

from __future__ import annotations

import importlib.metadata
import sys
from collections.abc import AsyncIterator
from dataclasses import dataclass
from functools import cache
from typing import Protocol, runtime_checkable

from myna.core import EventSink, PcmChunk, SessionConfig, SttService


@dataclass(frozen=True)
class Candidate:
    """One point in the evaluation matrix.

    streaming_strategy examples:
    - "commit-on-finalize" — single final at end of audio (Phase 2)
    - "chunked-redecode"   — AED bolt-on streaming, e.g. LocalAgreement (Phase 3)
    - "native-transducer"  — natively streaming transducer (Phase 3+)
    - "scripted"           — fake adapter, no model
    """

    model: str
    engine: str
    streaming_strategy: str

    @property
    def id(self) -> str:
        return f"{self.model}/{self.engine}/{self.streaming_strategy}"


@runtime_checkable
class Adapter(SttService, Protocol):
    """An ``SttService`` that also identifies which candidate it wraps."""

    @property
    def candidate(self) -> Candidate: ...

    async def run_session(
        self,
        config: SessionConfig,
        audio: AsyncIterator[PcmChunk],
        emit: EventSink,
    ) -> None: ...


@cache
def _distribution_version(dist: str) -> str | None:
    try:
        return importlib.metadata.version(dist)
    except importlib.metadata.PackageNotFoundError:
        return None


def runtime_versions(*libraries: tuple[str, str]) -> dict[str, str]:
    """``{distribution: version}`` for each ``(module, distribution)`` already loaded.

    Only ``sys.modules`` is consulted, never an import: capabilities answers
    every connection's greeting on the event loop, and importing an inference
    stack there would put its cold import cost (seconds from a cold squashfs)
    in front of the client's capabilities timeout. Before the first model load
    the libraries are absent, so a benchmark reads this after a session.

    The version comes from the module actually loaded, not package metadata
    alone: a snap component layers ``onnxruntime-gpu`` over the base snap's
    ``onnxruntime`` on ``PYTHONPATH``, and both leave dist-info behind.
    """
    out: dict[str, str] = {}
    for module, dist in libraries:
        loaded = sys.modules.get(module)
        if loaded is None:
            continue
        version = getattr(loaded, "__version__", None) or _distribution_version(dist)
        if version:
            out[dist] = str(version)
    return out

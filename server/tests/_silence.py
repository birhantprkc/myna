"""Silent PCM for contract tests: an ``AudioSource`` of a fixed duration."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator

from myna.core import AudioFormat, PcmChunk

_CHUNK_SECONDS = 0.1


class SilenceSource:
    """``realtime=True`` paces chunks at capture rate, mimicking live
    push-to-talk audio; ``False`` streams as fast as the consumer accepts."""

    format = AudioFormat()

    def __init__(self, duration_seconds: float, *, realtime: bool = False) -> None:
        self._duration = duration_seconds
        self._realtime = realtime

    async def chunks(self) -> AsyncIterator[PcmChunk]:
        silence = bytes(int(self.format.bytes_per_second * _CHUNK_SECONDS))
        remaining = self._duration
        while remaining > 0:
            if self._realtime:
                await asyncio.sleep(min(_CHUNK_SECONDS, remaining))
            yield PcmChunk(data=silence, format=self.format)
            remaining -= _CHUNK_SECONDS

"""Audio sources for the harness.

All sources implement ``myna.core.AudioSource``. ``WavFileSource`` plays
fixture clips (see ``myna.testbed.corpus``). Live capture belongs to the
client, not here: ``client/myna-audio`` owns it natively (T49-T52).
"""

from __future__ import annotations

import asyncio
import time
import wave
from collections.abc import AsyncIterable, AsyncIterator, Awaitable, Callable
from pathlib import Path

from myna.core import AudioFormat, PcmChunk


async def paced(
    chunks: AsyncIterable[PcmChunk],
    *,
    clock: Callable[[], float] | None = None,
    sleep: Callable[[float], Awaitable[None]] | None = None,
) -> AsyncIterator[PcmChunk]:
    """Hand each chunk over when a microphone would: once its last sample is in.

    Chunk k is due at ``origin + audio_end(k)`` on a monotonic clock, with the
    origin taken when the first chunk is asked for. Due times are absolute, so
    the consumer's time on a chunk never accumulates into drift, and a consumer
    that fell behind gets the overdue chunks at once, as it would from a
    buffered capture device.
    """
    clock = clock or time.monotonic
    sleep = sleep or asyncio.sleep
    origin: float | None = None
    audio_end = 0.0
    async for chunk in chunks:
        if origin is None:
            origin = clock()
        audio_end += chunk.duration_seconds
        delay = origin + audio_end - clock()
        if delay > 0:
            await sleep(delay)
        yield chunk


class WavFileSource:
    """Streams a PCM WAV file as chunks in its native format.

    ``realtime=True`` paces chunks on the capture clock (see ``paced``),
    mimicking live audio; ``False`` streams as fast as the consumer accepts
    (batch accuracy runs). Resampling is not done here: candidates declare
    what they accept and adapters convert — the harness stays format-honest.

    Reads are synchronous (stdlib ``wave``); fixture clips are seconds long,
    so per-chunk reads are far below event-loop latency concerns.
    """

    def __init__(
        self,
        path: Path | str,
        *,
        chunk_seconds: float = 0.1,
        realtime: bool = False,
    ) -> None:
        self._path = Path(path)
        self._chunk_seconds = chunk_seconds
        self._realtime = realtime
        with wave.open(str(self._path), "rb") as wav:
            if wav.getcomptype() != "NONE":
                raise ValueError(f"{self._path}: only uncompressed PCM WAV is supported")
            self._format = AudioFormat(
                sample_rate_hz=wav.getframerate(),
                channels=wav.getnchannels(),
                sample_width_bytes=wav.getsampwidth(),
            )

    @property
    def format(self) -> AudioFormat:
        return self._format

    def chunks(self) -> AsyncIterator[PcmChunk]:
        return paced(self._read()) if self._realtime else self._read()

    async def _read(self) -> AsyncIterator[PcmChunk]:
        frames_per_chunk = max(1, round(self._format.sample_rate_hz * self._chunk_seconds))
        with wave.open(str(self._path), "rb") as wav:
            while data := wav.readframes(frames_per_chunk):
                yield PcmChunk(data=data, format=self._format)

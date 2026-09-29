"""Real-time pacing: chunks leave on the capture clock, never later.

A live microphone delivers chunk k when its last sample has been captured, at
``origin + audio_end(k)``. Pacing by sleeping one chunk's duration per chunk
adds the consumer's own time to every step, so a sweep "at real-time pace"
drifted slower than real time by however long each send took.
"""

from __future__ import annotations

import pytest

from myna.core import AudioFormat, PcmChunk
from myna.testbed.sources import paced

FORMAT = AudioFormat(sample_rate_hz=16_000, channels=1, sample_width_bytes=2)


def chunk(seconds: float = 0.1) -> PcmChunk:
    return PcmChunk(data=b"\0\0" * round(16_000 * seconds), format=FORMAT)


class FakeClock:
    """A monotonic clock that only moves when someone sleeps or works."""

    def __init__(self, now: float = 100.0) -> None:
        self.now = now
        self.sleeps: list[float] = []

    def __call__(self) -> float:
        return self.now

    async def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.now += seconds


async def source(chunks):
    for c in chunks:
        yield c


async def yield_times(clock, chunks, work=lambda i: 0.0):
    """When each chunk was handed over; ``work(i)`` is the consumer's time on it."""
    times = []
    async for i, _ in aenumerate(paced(source(chunks), clock=clock, sleep=clock.sleep)):
        times.append(round(clock.now, 6))
        clock.now += work(i)
    return times


async def aenumerate(it):
    i = 0
    async for item in it:
        yield i, item
        i += 1


async def test_each_chunk_leaves_when_its_last_sample_would_have_been_captured():
    clock = FakeClock()
    assert await yield_times(clock, [chunk()] * 4) == [100.1, 100.2, 100.3, 100.4]


async def test_the_consumers_own_time_does_not_accumulate_into_drift():
    clock = FakeClock()
    times = await yield_times(clock, [chunk()] * 5, work=lambda i: 0.03)
    assert times == [100.1, 100.2, 100.3, 100.4, 100.5]


async def test_uneven_chunks_are_scheduled_by_audio_position_not_count():
    clock = FakeClock()
    times = await yield_times(clock, [chunk(0.1), chunk(0.25), chunk(0.05)])
    assert times == [100.1, 100.35, 100.4]


async def test_a_late_sender_catches_up_without_sleeping_like_a_buffered_mic():
    clock = FakeClock()
    times = await yield_times(clock, [chunk()] * 5, work=lambda i: 0.35 if i == 0 else 0.0)
    # Chunk 0 held the consumer until 100.45: chunks 1-3 were already due, so
    # they go at once; chunk 4 is due at 100.5 and waits for it.
    assert times == [100.1, 100.45, 100.45, 100.45, 100.5]
    assert len(clock.sleeps) == 2


async def test_the_schedule_starts_when_the_first_chunk_is_asked_for():
    clock = FakeClock(now=5.0)
    stream = paced(source([chunk()] * 2), clock=clock, sleep=clock.sleep)
    clock.now = 7.0  # created early, iterated later: the origin is 7.0, not 5.0
    got = []
    async for _ in stream:
        got.append(round(clock.now, 6))
    assert got == [7.1, 7.2]


@pytest.mark.parametrize("realtime", [False, True])
async def test_wav_source_paces_only_when_asked(tmp_path, monkeypatch, realtime):
    import wave

    from myna.testbed import sources

    path = tmp_path / "t.wav"
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(16_000)
        wav.writeframes(b"\0\0" * 4800)
    clock = FakeClock()
    monkeypatch.setattr(sources.time, "monotonic", clock)
    monkeypatch.setattr(sources.asyncio, "sleep", clock.sleep)
    got = [c async for c in sources.WavFileSource(path, realtime=realtime).chunks()]
    assert len(got) == 3
    assert clock.now == pytest.approx(100.3 if realtime else 100.0)

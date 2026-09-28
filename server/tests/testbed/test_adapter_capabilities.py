"""Every adapter advertises its streaming mode in ``capabilities()`` so the
client can pick a default without opening a session."""

from __future__ import annotations

import pytest

pytest.importorskip("numpy", reason="adapter extras not installed")

from myna.testbed.fake import FakeAdapter
from myna.testbed.funasr import FunasrAdapter
from myna.testbed.parakeet import ParakeetAdapter
from myna.testbed.whisper import FasterWhisperAdapter


@pytest.mark.parametrize(
    ("adapter", "streams"),
    [
        (ParakeetAdapter(streaming=True), True),
        (ParakeetAdapter(streaming=False), False),
        (FasterWhisperAdapter("tiny", streaming=True), True),
        (FasterWhisperAdapter("tiny", streaming=False), False),
        (FunasrAdapter(), False),
        (FakeAdapter(), False),
    ],
    ids=["parakeet-stream", "parakeet-batch", "whisper-stream", "whisper-batch", "funasr", "fake"],
)
def test_capabilities_advertise_the_streaming_mode(adapter, streams):
    assert adapter.capabilities().streaming is streams

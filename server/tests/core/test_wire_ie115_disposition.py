"""Golden-frame tests for disposition encoding in IE115 wire protocol (T08, feature 007)."""

from myna.core import Disposition, TranscriptionFinal
from myna.core.wire_ie115 import Ie115Encoder


def encode_delta(event: TranscriptionFinal) -> dict:
    return Ie115Encoder().frames(event)[-1]


def test_disposition_encoding_committed():
    """Test that committed disposition is encoded in delta events."""
    event = TranscriptionFinal(
        text="Hello world",
        disposition=Disposition.COMMITTED,
        segment_index=0,
    )

    wire_frame = encode_delta(event)

    assert wire_frame["type"] == "conversation.item.input_audio_transcription.delta"
    assert wire_frame["item_id"].startswith("item_")  # identity: test_ie115_dialect
    assert wire_frame["delta"] == "Hello world"
    assert wire_frame["disposition"] == "committed"
    assert wire_frame["segment_index"] == 0


def test_disposition_encoding_unstable():
    """Test that unstable disposition is encoded in delta events."""
    event = TranscriptionFinal(
        text="Hello wor",
        disposition=Disposition.UNSTABLE,
    )

    wire_frame = encode_delta(event)

    assert wire_frame["type"] == "conversation.item.input_audio_transcription.delta"
    assert wire_frame["item_id"].startswith("item_")  # identity: test_ie115_dialect
    assert wire_frame["delta"] == "Hello wor"
    assert wire_frame["disposition"] == "unstable"
    assert "segment_index" not in wire_frame  # Only present for committed


def test_multiple_committed_segments():
    """Test encoding multiple committed segments with increasing segment_index."""
    segments = [
        TranscriptionFinal(text="Hello ", disposition=Disposition.COMMITTED, segment_index=0),
        TranscriptionFinal(text="world. ", disposition=Disposition.COMMITTED, segment_index=1),
        TranscriptionFinal(text="How are you?", disposition=Disposition.COMMITTED, segment_index=2),
    ]

    for i, seg in enumerate(segments):
        frame = encode_delta(seg)
        assert frame["disposition"] == "committed"
        assert frame["segment_index"] == i
        assert frame["delta"] == seg.text

"""Whisper's text normalisers, vendored from openai/whisper (MIT).

A secondary scoring convention beside ``myna.testbed.metrics.normalize``: the
Open ASR Leaderboard scores English with ``EnglishTextNormalizer`` and other
languages with ``BasicTextNormalizer``, so a WER under these is comparable
with published figures. Each module carries its upstream pin and licence.
"""

from myna.testbed.whisper_normalizers.basic import BasicTextNormalizer
from myna.testbed.whisper_normalizers.english import EnglishTextNormalizer

__all__ = ["BasicTextNormalizer", "EnglishTextNormalizer"]

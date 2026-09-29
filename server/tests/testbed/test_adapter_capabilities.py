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


# ─── runtime ─────────────────────────────────────────────────────────────────
#
# A benchmark row has to say which inference stack produced it, and only the
# server knows: a snap component can put a different onnxruntime on the path
# than the one the base snap ships. The libraries are faked in sys.modules, as
# a model load leaves them, so the suite needs none of them.


@pytest.fixture
def runtimes(monkeypatch):
    import sys
    from types import SimpleNamespace

    for module, version in {
        "onnxruntime": "1.23.0",
        "ctranslate2": "4.6.0",
        "faster_whisper": "1.2.0",
        "funasr_onnx": "0.4.1",
    }.items():
        monkeypatch.setitem(sys.modules, module, SimpleNamespace(__version__=version))


@pytest.mark.parametrize(
    ("adapter", "runtime"),
    [
        (
            ParakeetAdapter(),
            {"onnxruntime": "1.23.0", "execution_provider": "CPUExecutionProvider"},
        ),
        (
            ParakeetAdapter(device="cuda"),
            {"onnxruntime": "1.23.0", "execution_provider": "CUDAExecutionProvider"},
        ),
        (
            FasterWhisperAdapter("tiny", device="cuda"),
            {"ctranslate2": "4.6.0", "faster-whisper": "1.2.0", "device": "cuda"},
        ),
        (
            FunasrAdapter(),
            {
                "funasr-onnx": "0.4.1",
                "onnxruntime": "1.23.0",
                "execution_provider": "CPUExecutionProvider",
            },
        ),
    ],
    ids=["parakeet-cpu", "parakeet-cuda", "whisper-cuda", "funasr"],
)
def test_capabilities_name_the_inference_stack(runtimes, adapter, runtime):
    assert adapter.capabilities().runtime == runtime


def test_a_library_not_yet_loaded_is_left_out(monkeypatch):
    import sys

    monkeypatch.setitem(sys.modules, "onnxruntime", None)  # a failed import
    assert ParakeetAdapter().capabilities().runtime == {
        "execution_provider": "CPUExecutionProvider"
    }


class _ImportTrap:
    """A meta-path finder that records any import of ``names`` and refuses it.

    It records rather than raises, so a caller that swallows import errors
    cannot hide the attempt.
    """

    def __init__(self, names):
        self.names = names
        self.attempted = []

    def find_spec(self, name, path=None, target=None):
        if name.partition(".")[0] in self.names:
            self.attempted.append(name)
            raise ImportError(name)


@pytest.mark.parametrize(
    "adapter",
    [ParakeetAdapter(device="cuda"), FasterWhisperAdapter("tiny"), FunasrAdapter()],
    ids=["parakeet", "whisper", "funasr"],
)
def test_capabilities_never_import_the_inference_stack(monkeypatch, adapter):
    # Capabilities answers every greeting on the event loop, ahead of the
    # client's timeout: a cold import of the stack there stalls dictation.
    import sys

    stack = {"onnxruntime", "ctranslate2", "faster_whisper", "funasr_onnx"}
    for module in list(sys.modules):
        if module.partition(".")[0] in stack:
            monkeypatch.delitem(sys.modules, module)
    trap = _ImportTrap(stack)
    monkeypatch.setattr(sys, "meta_path", [trap, *sys.meta_path])
    assert set(adapter.capabilities().runtime) <= {"execution_provider", "device"}
    assert trap.attempted == []


def test_a_version_missing_from_the_module_falls_back_to_its_distribution(monkeypatch):
    import sys
    from types import SimpleNamespace

    from myna.testbed import adapter

    monkeypatch.setitem(sys.modules, "funasr_onnx", SimpleNamespace())
    monkeypatch.setattr(adapter, "_distribution_version", lambda dist: f"{dist}-9.9")
    assert adapter.runtime_versions(("funasr_onnx", "funasr-onnx")) == {
        "funasr-onnx": "funasr-onnx-9.9"
    }


def test_a_version_nobody_can_report_is_left_out(monkeypatch):
    import sys
    from types import SimpleNamespace

    from myna.testbed import adapter

    monkeypatch.setitem(sys.modules, "funasr_onnx", SimpleNamespace())
    assert adapter.runtime_versions(("funasr_onnx", "not-a-real-dist-xyz")) == {}

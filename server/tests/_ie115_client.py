"""Test-side IE115 client: the ``SttClient`` the parity tests drive the server
with, and the decoder turning its server frames back into internal events.
The Rust client (``client/myna-orchestrator/src/backend/ws_unix_ie115.rs``)
is the production counterpart."""

from __future__ import annotations

import base64
import contextlib
import json
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

from websockets.asyncio.client import ClientConnection, unix_connect
from websockets.exceptions import ConnectionClosed

from myna.core import (
    PHASE_TRANSCRIBING,
    Disposition,
    PcmChunk,
    Segment,
    SessionConfig,
    TranscriptionDone,
    TranscriptionError,
    TranscriptionEvent,
    TranscriptionFinal,
    TranscriptionProgress,
)
from myna.core import wire_ie115 as w

_TERMINAL = ("transcription.done", "transcription.error")
_STATE_TO_PHASE = {v: k for k, v in w._PHASE_TO_STATE.items()}


def pcm_to_append(chunk: PcmChunk) -> dict[str, Any]:
    """Encode a PcmChunk as an ``input_audio_buffer.append`` frame (base64)."""
    return {
        "type": w.INPUT_AUDIO_APPEND,
        "audio": base64.b64encode(chunk.data).decode("ascii"),
    }


def _segments(raw: Any) -> tuple[Segment, ...]:
    return tuple(Segment(**seg) for seg in raw or ())


class Ie115Decoder:
    """Decodes IE115 server frames into internal transcript events for **one
    utterance**: the ``completed`` answering the client's commit is the terminal
    ``done``. Never infer ``done`` from a server close."""

    def __init__(self) -> None:
        self._terminated = False

    def decode(self, frame: dict[str, Any]) -> list[TranscriptionEvent]:
        """Zero or more internal events for one IE115 frame; control frames
        yield nothing."""
        ftype = frame.get("type")
        if ftype == w.STATUS_EVENT:
            phase = _STATE_TO_PHASE.get(str(frame.get("state") or ""), PHASE_TRANSCRIBING)
            return [TranscriptionProgress(phase=phase, snippet=frame.get("snippet"))]
        if ftype == w.TRANSCRIPTION_DELTA:
            disposition = (
                Disposition.COMMITTED
                if frame.get("disposition", "committed") == "committed"
                else Disposition.UNSTABLE
            )
            return [
                TranscriptionFinal(
                    text=frame.get("delta") or "",
                    disposition=disposition,
                    segments=_segments(frame.get("segments")),
                )
            ]
        if ftype == w.TRANSCRIPTION_COMPLETED:
            self._terminated = True
            return [
                TranscriptionDone(
                    text=frame.get("transcript", ""),
                    segments=_segments(frame.get("segments")),
                )
            ]
        if ftype == w.ERROR:
            self._terminated = True
            err = frame.get("error") or {}
            return [
                TranscriptionError(
                    code=err.get("code", "server_error"), message=err.get("message", "")
                )
            ]
        return []

    def on_close(self) -> list[TranscriptionEvent]:
        """A close before the utterance's terminal is a failure, never a clean
        ``done``. No-op after a real terminal."""
        if self._terminated:
            return []
        self._terminated = True
        return [
            TranscriptionError(
                code="connection_closed",
                message="connection closed before the utterance completed",
            )
        ]


class WsUnixIe115Client:
    """``SttClient`` speaking the IE115 dialect over ws+unix. Sends
    ``session.update`` first (the shape-sniff trigger); audio goes as raw
    binary frames, or base64 ``input_audio_buffer.append`` when
    ``base64_audio=True``."""

    def __init__(self, socket_path: Path | str, *, base64_audio: bool = False) -> None:
        self._socket_path = str(socket_path)
        self._base64_audio = base64_audio

    async def open_session(self, config: SessionConfig) -> _Ie115Session:
        ws = await unix_connect(self._socket_path, ping_interval=None)
        await ws.send(
            json.dumps({"type": w.SESSION_UPDATE, "session": w.session_config_to_ie115(config)})
        )
        return _Ie115Session(ws, base64_audio=self._base64_audio)


class _Ie115Session:
    def __init__(self, ws: ClientConnection, *, base64_audio: bool) -> None:
        self._ws = ws
        self._base64_audio = base64_audio
        self._decoder = Ie115Decoder()

    async def send_audio(self, chunk: PcmChunk) -> None:
        # The server may end the session and close mid-stream; the reason
        # arrives on the events channel.
        with contextlib.suppress(ConnectionClosed):
            if self._base64_audio:
                await self._ws.send(json.dumps(pcm_to_append(chunk)))
            else:
                await self._ws.send(chunk.data)

    async def finish_audio(self) -> None:
        with contextlib.suppress(ConnectionClosed):
            await self._ws.send(json.dumps({"type": w.INPUT_AUDIO_COMMIT}))

    async def events(self) -> AsyncIterator[TranscriptionEvent]:
        try:
            async for frame in self._ws:
                for event in self._decoder.decode(json.loads(frame)):
                    yield event
                    if event.type in _TERMINAL:
                        return
        except ConnectionClosed:
            pass
        for event in self._decoder.on_close():
            yield event

    async def aclose(self) -> None:
        await self._ws.close()

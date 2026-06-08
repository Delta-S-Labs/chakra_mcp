"""Voice I/O — Sarvam STT + TTS, push-to-talk mic, speaker playback.

Wire reference (verified against docs.sarvam.ai):

  STT:  POST https://api.sarvam.ai/speech-to-text
        headers: api-subscription-key: <key>
        body:    multipart/form-data with `file=@…wav`
        response: {"transcript": "...", ...}

  TTS:  POST https://api.sarvam.ai/text-to-speech
        headers: api-subscription-key, content-type: application/json
        body:    {"text", "target_language_code", "model", "speaker",
                  "output_audio_codec"}
        response: {"audios": ["<base64 wav>"], ...}

The audio path is mono 16 kHz PCM throughout — what sounddevice records
and what Sarvam consumes by default.
"""

from __future__ import annotations

import asyncio
import base64
import io
import os
import wave
from dataclasses import dataclass

import httpx
import numpy as np
import sounddevice as sd

SARVAM_BASE = "https://api.sarvam.ai"
SAMPLE_RATE = 16_000  # mono PCM, matches Sarvam's expected input
CHANNELS = 1


@dataclass(frozen=True)
class VoiceConfig:
    api_key: str
    language: str = "en-IN"  # Sarvam English-India is the cleanest TTS voice
    speaker: str = "shubh"
    model: str = "bulbul:v3"


def voice_config_from_env() -> VoiceConfig:
    key = os.environ.get("SARVAM_API_KEY")
    if not key:
        raise RuntimeError("SARVAM_API_KEY not set (see .env.example)")
    return VoiceConfig(
        api_key=key,
        language=os.environ.get("SARVAM_LANG", "en-IN"),
        speaker=os.environ.get("SARVAM_SPEAKER", "shubh"),
        model=os.environ.get("SARVAM_MODEL", "bulbul:v3"),
    )


# ─── Recording (push-to-talk) ────────────────────────────────────

class Recorder:
    """Push-to-talk recorder.

    Call `start()` when the spacebar goes down, `stop()` when it lifts.
    `stop()` returns an in-memory WAV ready to hand to Sarvam STT.
    """

    def __init__(self, sample_rate: int = SAMPLE_RATE) -> None:
        self.sample_rate = sample_rate
        self._stream: sd.InputStream | None = None
        self._frames: list[np.ndarray] = []

    def _cb(self, indata, frames, time_, status) -> None:  # noqa: ARG002
        # Copy — sounddevice reuses the buffer.
        self._frames.append(indata.copy())

    def start(self) -> None:
        if self._stream is not None:
            return
        self._frames = []
        self._stream = sd.InputStream(
            samplerate=self.sample_rate,
            channels=CHANNELS,
            dtype="int16",
            callback=self._cb,
        )
        self._stream.start()

    def stop(self) -> bytes:
        if self._stream is None:
            return b""
        self._stream.stop()
        self._stream.close()
        self._stream = None
        if not self._frames:
            return b""
        audio = np.concatenate(self._frames, axis=0)
        # Reject sub-quarter-second blips — they're almost always
        # accidental space-presses and Sarvam returns "" on them.
        if len(audio) < self.sample_rate // 4:
            return b""
        return _pcm_to_wav_bytes(audio, self.sample_rate)


def _pcm_to_wav_bytes(pcm_int16: np.ndarray, rate: int) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(CHANNELS)
        w.setsampwidth(2)  # int16
        w.setframerate(rate)
        w.writeframes(pcm_int16.tobytes())
    return buf.getvalue()


# ─── Sarvam HTTP ─────────────────────────────────────────────────

class SarvamClient:
    """Thin HTTP wrapper around Sarvam STT + TTS."""

    def __init__(self, cfg: VoiceConfig) -> None:
        self.cfg = cfg
        self._http = httpx.AsyncClient(
            base_url=SARVAM_BASE,
            timeout=httpx.Timeout(30.0, connect=10.0),
            headers={"api-subscription-key": cfg.api_key},
        )

    async def aclose(self) -> None:
        await self._http.aclose()

    async def transcribe(self, wav_bytes: bytes) -> str:
        """Return the transcript of a single utterance, or '' if silent."""
        if not wav_bytes:
            return ""
        files = {"file": ("clip.wav", wav_bytes, "audio/wav")}
        r = await self._http.post("/speech-to-text", files=files)
        r.raise_for_status()
        return (r.json().get("transcript") or "").strip()

    async def synthesize(self, text: str) -> bytes:
        """Return WAV bytes for a piece of text."""
        if not text.strip():
            return b""
        body = {
            "text": text[:2500],  # bulbul:v3 cap
            "target_language_code": self.cfg.language,
            "model": self.cfg.model,
            "speaker": self.cfg.speaker,
            "output_audio_codec": "wav",
        }
        r = await self._http.post("/text-to-speech", json=body)
        r.raise_for_status()
        audios = r.json().get("audios") or []
        if not audios:
            return b""
        return base64.b64decode(audios[0])


# ─── Playback ────────────────────────────────────────────────────

def play_wav_bytes(wav_bytes: bytes) -> None:
    """Synchronously play a WAV. Blocks until playback finishes."""
    if not wav_bytes:
        return
    with wave.open(io.BytesIO(wav_bytes), "rb") as w:
        rate = w.getframerate()
        n = w.getnframes()
        frames = w.readframes(n)
    audio = np.frombuffer(frames, dtype=np.int16)
    sd.play(audio, rate)
    sd.wait()


async def play_wav_bytes_async(wav_bytes: bytes) -> None:
    """Async wrapper so TTS playback doesn't block the Textual event loop."""
    await asyncio.to_thread(play_wav_bytes, wav_bytes)


def stop_playback() -> None:
    """Interrupt any in-progress playback. Safe to call from the event
    loop (sounddevice's stop is non-blocking); it makes the `sd.wait()`
    inside `play_wav_bytes` return immediately so the speaking turn
    unwinds and the user can talk again."""
    try:
        sd.stop()
    except Exception:
        pass

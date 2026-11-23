"""Reference-clip transcription.

OmniVoice cloning quality depends heavily on the transcript of the reference
clip matching what is actually said. Generic Whisper is weak on colloquial
Tamil (it often hallucinates English or loops), so Tamil uses a Tamil
fine-tuned Whisper; other languages use Whisper large-v3-turbo with the
language forced."""
from __future__ import annotations

import threading
from typing import Optional

import numpy as np
import torch

from . import gpu

LANG_MODELS = {
    "ta": "vasista22/whisper-tamil-medium",
}
DEFAULT_MODEL = "openai/whisper-large-v3-turbo"

_pipes: dict[str, object] = {}
_lock = threading.Lock()


def _pipe(name: str):
    from transformers import pipeline

    if name not in _pipes:
        dev = "cuda:0" if torch.cuda.is_available() else "cpu"
        dtype = torch.float16 if dev.startswith("cuda") else torch.float32
        _pipes[name] = pipeline("automatic-speech-recognition", name, dtype=dtype, device=dev)
        gpu.mark_loaded("asr")
    return _pipes[name]


def transcribe(wave: np.ndarray, sr: int, language: Optional[str] = None) -> str:
    import librosa

    w16 = librosa.resample(wave.astype(np.float32), orig_sr=sr, target_sr=16000) if sr != 16000 else wave
    w16 = w16[: 16000 * 29]
    with _lock:
        if language in LANG_MODELS:
            out = _pipe(LANG_MODELS[language])({"array": w16, "sampling_rate": 16000})
        else:
            gen = {"task": "transcribe"}
            if language:
                gen["language"] = language
            out = _pipe(DEFAULT_MODEL)({"array": w16, "sampling_rate": 16000}, generate_kwargs=gen)
    return out["text"].strip()


def release() -> None:
    with _lock:
        _pipes.clear()
    if torch.cuda.is_available():
        torch.cuda.empty_cache()


gpu.register("asr", lambda: _pipes.clear())

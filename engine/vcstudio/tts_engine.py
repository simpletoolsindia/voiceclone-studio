"""Text-to-speech with zero-shot voice cloning, powered by OmniVoice
(k2-fsa/OmniVoice, Apache-2.0) - 600+ languages including Tamil."""
from __future__ import annotations

import io
import threading
from typing import Optional

import numpy as np
import torch

from . import gpu

MODEL_ID = "k2-fsa/OmniVoice"

_model = None
_lock = threading.Lock()


def _device_map() -> str:
    if torch.cuda.is_available():
        return "cuda:0"
    if getattr(torch.backends, "mps", None) and torch.backends.mps.is_available():
        return "mps"
    return "cpu"


def get_model():
    global _model
    with _lock:
        if _model is None:
            from omnivoice import OmniVoice

            dm = _device_map()
            dtype = torch.float16 if dm.startswith("cuda") else torch.float32
            _model = OmniVoice.from_pretrained(MODEL_ID, device_map=dm, dtype=dtype)
            gpu.mark_loaded("tts")
        return _model


def unload():
    global _model
    with _lock:
        _model = None


gpu.register("tts", unload)


def sampling_rate() -> int:
    return int(get_model().sampling_rate)


def make_prompt(wave: np.ndarray, sr: int, ref_text: Optional[str]):
    """Return an OmniVoice VoiceClonePrompt (serialisable with prompt_to_bytes)."""
    m = get_model()
    with _lock:
        return m.create_voice_clone_prompt(ref_audio=(torch.from_numpy(wave).float(), sr),
                                           ref_text=ref_text or None)


def prompt_to_bytes(prompt) -> bytes:
    buf = io.BytesIO()
    torch.save({"format_version": 1, "ref_audio_tokens": prompt.ref_audio_tokens.detach().cpu(),
                "ref_text": prompt.ref_text, "ref_rms": float(prompt.ref_rms)}, buf)
    return buf.getvalue()


def prompt_from_bytes(data: bytes):
    from omnivoice.models.omnivoice import VoiceClonePrompt

    d = torch.load(io.BytesIO(data), map_location="cpu", weights_only=True)
    return VoiceClonePrompt(ref_audio_tokens=d["ref_audio_tokens"], ref_text=d["ref_text"],
                            ref_rms=float(d["ref_rms"]))


def synthesize(text: str, prompt=None, language: Optional[str] = None, speed: Optional[float] = None,
               num_step: int = 32, guidance_scale: float = 2.0, seed: Optional[int] = None,
               instruct: Optional[str] = None) -> tuple[np.ndarray, int]:
    m = get_model()
    kw = dict(num_step=num_step, guidance_scale=guidance_scale)
    with _lock:
        audios = m.generate(text=text, language=language or None, voice_clone_prompt=prompt,
                            instruct=instruct or None, speed=speed, seed=seed, **kw)
    wav = audios[0]
    if isinstance(wav, torch.Tensor):
        wav = wav.squeeze().float().cpu().numpy()
    return np.asarray(wav, dtype=np.float32), int(m.sampling_rate)

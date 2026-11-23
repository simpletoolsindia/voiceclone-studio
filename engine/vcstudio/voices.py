"""Voice checkpoints and the high-level clone / speak / convert pipeline.

A voice checkpoint is a portable ``.vcpt`` file (a zip archive):

    manifest.json          name, language, transcript, creation info, format version
    reference_vc.wav       ~15 s clean clip (22.05 kHz) - conditioning for Seed-VC
    reference_tts.wav      ~9 s clean clip (24 kHz)     - prompt audio for OmniVoice
    tts_prompt.pt          cached OmniVoice VoiceClonePrompt (audio tokens + text)
    vc_quality.pt          cached Seed-VC speaker conditioning (file conversion)
    vc_realtime.pt         cached Seed-VC speaker conditioning (live conversion)

Only the reference clips are required; the cached tensors are rebuilt on
demand when missing, so a checkpoint made on one machine works anywhere.
"""
from __future__ import annotations

import io
import json
import re
import shutil
import threading
import time
import uuid
import zipfile
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Callable, Optional

import numpy as np
import torch

from .audio_utils import load_audio, pick_reference, rms_normalize, save_wav

FORMAT_VERSION = 1
CKPT_EXT = ".vcpt"
Progress = Optional[Callable[[float, str], None]]


@dataclass
class VoiceInfo:
    id: str
    name: str
    language: str = "ta"
    ref_text: str = ""
    created: float = field(default_factory=time.time)
    source_file: str = ""
    ref_window: list = field(default_factory=list)
    format_version: int = FORMAT_VERSION
    notes: str = ""
    kind: str = "cloned"        # cloned (from a recording) | designed (AI-designed character voice)
    preset: str = ""            # character preset key, e.g. "grandma"


class VoiceStore:
    def __init__(self, root: Path):
        self.root = Path(root)
        self.root.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()
        self._cache: dict[tuple[str, str], object] = {}

    # ------------------------------------------------------------ storage
    def path(self, vid: str) -> Path:
        if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", vid):
            raise ValueError("invalid voice id")
        return self.root / f"{vid}{CKPT_EXT}"

    def list(self) -> list[dict]:
        out = []
        for p in sorted(self.root.glob(f"*{CKPT_EXT}"), key=lambda p: p.stat().st_mtime, reverse=True):
            try:
                out.append(self.info(p.stem).__dict__)
            except Exception:  # corrupt file: skip, never crash the list
                continue
        return out

    def info(self, vid: str) -> VoiceInfo:
        with zipfile.ZipFile(self.path(vid)) as z:
            d = json.loads(z.read("manifest.json"))
        d["id"] = vid
        return VoiceInfo(**{k: v for k, v in d.items() if k in VoiceInfo.__dataclass_fields__})

    def read_member(self, vid: str, name: str) -> Optional[bytes]:
        with zipfile.ZipFile(self.path(vid)) as z:
            return z.read(name) if name in z.namelist() else None

    def _write(self, vid: str, info: VoiceInfo, members: dict[str, bytes]) -> None:
        tmp = self.path(vid).with_suffix(".tmp")
        with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as z:
            z.writestr("manifest.json", json.dumps(asdict(info), ensure_ascii=False, indent=2))
            for k, v in members.items():
                z.writestr(k, v)
        tmp.replace(self.path(vid))

    def _update_member(self, vid: str, name: str, data: bytes) -> None:
        with self._lock:
            p = self.path(vid)
            with zipfile.ZipFile(p) as z:
                members = {n: z.read(n) for n in z.namelist() if n not in ("manifest.json", name)}
            members[name] = data
            self._write(vid, self.info(vid), members)

    def delete(self, vid: str) -> None:
        self.path(vid).unlink(missing_ok=True)
        for k in [k for k in self._cache if k[0] == vid]:
            self._cache.pop(k, None)

    def rename(self, vid: str, name: str, ref_text: Optional[str] = None) -> VoiceInfo:
        with self._lock:
            info = self.info(vid)
            info.name = name or info.name
            with zipfile.ZipFile(self.path(vid)) as z:
                members = {n: z.read(n) for n in z.namelist() if n != "manifest.json"}
            if ref_text is not None and ref_text != info.ref_text:
                info.ref_text = ref_text
                members.pop("tts_prompt.pt", None)      # transcript changed: rebuild prompt
                self._cache.pop((vid, "tts"), None)
            self._write(vid, info, members)
            return info

    def export_to(self, vid: str, dest: Path) -> Path:
        dest = Path(dest)
        if dest.suffix.lower() != CKPT_EXT:
            dest = dest.with_suffix(CKPT_EXT)
        shutil.copyfile(self.path(vid), dest)
        return dest

    def import_from(self, src: Path) -> VoiceInfo:
        with zipfile.ZipFile(src) as z:
            names = z.namelist()
            if "manifest.json" not in names or not ({"reference_vc.wav", "reference_tts.wav"} & set(names)):
                raise ValueError("not a VoiceClone Studio checkpoint (.vcpt)")
            d = json.loads(z.read("manifest.json"))
            if int(d.get("format_version", 1)) > FORMAT_VERSION:
                raise ValueError("checkpoint was made by a newer version of the app")
            for n in names:  # zip-slip / junk protection: flat archive only
                if "/" in n or "\\" in n or n.startswith(".."):
                    raise ValueError("unexpected file inside checkpoint")
            members = {n: z.read(n) for n in names if n != "manifest.json"}
        vid = uuid.uuid4().hex[:12]
        d["id"] = vid
        info = VoiceInfo(**{k: v for k, v in d.items() if k in VoiceInfo.__dataclass_fields__})
        self._write(vid, info, members)
        return info

    # ------------------------------------------------------------ cloning
    def create(self, name: str, audio_path: str, language: str = "ta", ref_text: Optional[str] = None,
               progress: Progress = None) -> VoiceInfo:
        from . import seedvc_engine, tts_engine

        p = progress or (lambda f, m: None)
        p(0.05, "Loading audio")
        raw = load_audio(audio_path, 24000)
        return self.create_from_wave(name or Path(audio_path).stem, raw, 24000, language, ref_text,
                                     source_file=Path(audio_path).name, progress=p)

    def create_from_wave(self, name: str, wave: np.ndarray, sr: int, language: str = "ta",
                         ref_text: Optional[str] = None, source_file: str = "", kind: str = "cloned",
                         preset: str = "", progress: Progress = None) -> VoiceInfo:
        """Clone a voice from an in-memory recording.

        If `ref_text` is given it must be the transcript of the whole recording,
        so the clip is used untrimmed (up to 20 s); otherwise the cleanest
        window is picked automatically and transcribed."""
        import librosa
        from . import seedvc_engine, tts_engine

        p = progress or (lambda f, m: None)
        vid = uuid.uuid4().hex[:12]
        tts_sr, vc_sr = 24000, 22050
        raw_tts = wave if sr == tts_sr else librosa.resample(wave, orig_sr=sr, target_sr=tts_sr)
        if len(raw_tts) < tts_sr * 3:
            raise ValueError("Reference audio is too short - use at least 5 seconds of clear speech.")
        raw_vc = librosa.resample(raw_tts, orig_sr=tts_sr, target_sr=vc_sr)
        ref_vc, a, b = pick_reference(raw_vc, vc_sr, target_sec=8.0, min_sec=5.0)
        ref_vc = rms_normalize(ref_vc)
        if ref_text:
            ref_tts, ta, tb = raw_tts[: tts_sr * 20], 0.0, min(20.0, len(raw_tts) / tts_sr)
        else:
            ref_tts, ta, tb = pick_reference(raw_tts, tts_sr, target_sec=9.0, min_sec=4.0)
        ref_tts = rms_normalize(ref_tts)

        p(0.2, "Transcribing reference clip")
        if not ref_text:
            from . import asr
            ref_text = asr.transcribe(ref_tts, tts_sr, language)

        p(0.45, "Building text-to-speech voice prompt")
        prompt = tts_engine.make_prompt(ref_tts, tts_sr, ref_text)
        p(0.7, "Building voice-conversion speaker embedding")
        vcm = seedvc_engine.get_model("quality")
        vref = vcm.make_ref(ref_vc)

        members = {
            "reference_vc.wav": _wav_bytes(ref_vc, vc_sr),
            "reference_tts.wav": _wav_bytes(ref_tts, tts_sr),
            "tts_prompt.pt": tts_engine.prompt_to_bytes(prompt),
            "vc_quality.pt": _tensor_bytes(vref.to_state()),
        }
        info = VoiceInfo(id=vid, name=name, language=language, ref_text=prompt.ref_text,
                         source_file=source_file, kind=kind, preset=preset,
                         ref_window=[round(a, 2), round(b, 2), round(ta, 2), round(tb, 2)])
        self._write(vid, info, members)
        self._cache[(vid, "tts")] = prompt
        self._cache[(vid, "quality")] = vref
        p(1.0, "Voice saved")
        return info

    # ------------------------------------------------------------ cached conditioning
    def tts_prompt(self, vid: str):
        from . import tts_engine
        key = (vid, "tts")
        if key not in self._cache:
            data = self.read_member(vid, "tts_prompt.pt")
            if data is None:
                wav = _wav_from_bytes(self.read_member(vid, "reference_tts.wav"), 24000)
                prompt = tts_engine.make_prompt(wav, 24000, self.info(vid).ref_text or None)
                self._update_member(vid, "tts_prompt.pt", tts_engine.prompt_to_bytes(prompt))
            else:
                prompt = tts_engine.prompt_from_bytes(data)
            self._cache[key] = prompt
        return self._cache[key]

    def vc_ref(self, vid: str, preset: str = "quality"):
        from . import seedvc_engine
        key = (vid, preset)
        if key not in self._cache:
            model = seedvc_engine.get_model(preset)
            member = f"vc_{preset}.pt"
            data = self.read_member(vid, member)
            if data is None:
                wav = _wav_from_bytes(self.read_member(vid, "reference_vc.wav"), model.sr)
                ref = model.make_ref(wav)
                self._update_member(vid, member, _tensor_bytes(ref.to_state()))
            else:
                ref = seedvc_engine.VoiceRef.from_state(torch.load(io.BytesIO(data), map_location="cpu",
                                                                   weights_only=True), model.device)
            self._cache[key] = ref
        return self._cache[key]

    def reference_wav(self, vid: str, sr: int) -> np.ndarray:
        return _wav_from_bytes(self.read_member(vid, "reference_vc.wav"), sr)


# ---------------------------------------------------------------- pipeline ops
SENT_SPLIT = re.compile(r"(?<=[.!?।॥\n])\s+")


def chunk_text(text: str, max_chars: int = 220) -> list[str]:
    """Group sentences into chunks the TTS model handles reliably."""
    text = re.sub(r"[ \t]+", " ", text.strip())
    sents = [s.strip() for s in SENT_SPLIT.split(text) if s.strip()]
    chunks, cur = [], ""
    for s in sents:
        while len(s) > max_chars:  # very long sentence: split on commas
            cut = s.rfind(",", 0, max_chars)
            cut = cut if cut > 40 else max_chars
            if cur:
                chunks.append(cur)
                cur = ""
            chunks.append(s[: cut + 1].strip())
            s = s[cut + 1:].strip()
        if len(cur) + len(s) + 1 > max_chars and cur:
            chunks.append(cur)
            cur = s
        else:
            cur = f"{cur} {s}".strip()
    if cur:
        chunks.append(cur)
    return chunks


def speak(store: VoiceStore, vid: Optional[str], text: str, language: str = "ta", mode: str = "auto",
          speed: Optional[float] = None, steps: int = 32, vc_steps: int = 30, pause_ms: int = 350,
          progress: Progress = None, take: int = 0) -> tuple[np.ndarray, int]:
    """Text to speech in a cloned voice.

    modes:
      direct  - OmniVoice cloning from the voice prompt (keeps the speaker's style)
      stable  - neutral OmniVoice voice, then Seed-VC into the cloned voice
                (most robust when the reference transcript is imperfect)
      auto    - direct, validated per chunk; falls back to stable on a bad take
    """
    import librosa
    from . import tts_engine

    p = progress or (lambda f, m: None)
    chunks = chunk_text(text)
    if not chunks:
        raise ValueError("Enter some text to speak.")
    sr = tts_engine.sampling_rate()
    prompt = store.tts_prompt(vid) if vid and mode in ("direct", "auto") else None
    vref = store.vc_ref(vid, "quality") if vid and mode in ("stable", "auto") else None
    vcm = None
    if vref is not None:
        from . import seedvc_engine
        vcm = seedvc_engine.get_model("quality")

    pieces = []
    gap = np.zeros(int(sr * pause_ms / 1000), dtype=np.float32)
    for i, chunk in enumerate(chunks):
        p(i / len(chunks), f"Speaking part {i + 1} of {len(chunks)}")
        wav = None
        if mode in ("direct", "auto") and prompt is not None:
            neutral = None
            for seed in (1 + take * 101, 7 + take * 101, 23 + take * 101):
                cand, _ = tts_engine.synthesize(chunk, prompt, language, speed=speed, num_step=steps, seed=seed)
                if mode == "direct":
                    wav = cand
                    break
                if neutral is None:
                    neutral, _ = tts_engine.synthesize(chunk, None, language, speed=speed, num_step=steps, seed=1 + take * 101)
                ratio = len(cand) / max(1, len(neutral))
                if 0.6 <= ratio <= 1.7 and _looks_like_speech(cand, sr):
                    wav = cand
                    break
            if wav is None and mode == "auto":
                wav = _to_voice(vcm, vref, neutral, sr, vc_steps)
        if wav is None:  # stable mode, or no voice selected
            base, _ = tts_engine.synthesize(chunk, None, language, speed=speed, num_step=steps, seed=1 + take * 101)
            wav = _to_voice(vcm, vref, base, sr, vc_steps) if vref is not None else base
        pieces += [wav, gap]
    p(1.0, "Done")
    out = np.concatenate(pieces[:-1]) if len(pieces) > 1 else pieces[0]
    return rms_normalize(out, -18.0), sr


def _to_voice(vcm, vref, wav: np.ndarray, sr: int, steps: int) -> np.ndarray:
    import librosa
    if vcm is None or vref is None:
        return wav
    x = librosa.resample(wav, orig_sr=sr, target_sr=vcm.sr)
    y = vcm.convert_long(x, vref, steps=steps)
    return librosa.resample(y, orig_sr=vcm.sr, target_sr=sr).astype(np.float32)


def _looks_like_speech(wav: np.ndarray, sr: int) -> bool:
    from .audio_utils import frame_db
    if len(wav) < sr * 0.3:
        return False
    db = frame_db(wav, sr)
    voiced = db > np.percentile(db, 95) - 35
    return 0.35 <= voiced.mean() <= 0.99


def convert_file(store: VoiceStore, vid: str, input_path: str, steps: int = 30, cfg_rate: float = 0.7,
                 progress: Progress = None) -> tuple[np.ndarray, int]:
    """Speech-to-speech: re-voice a whole recording (story, podcast...) in the cloned voice."""
    from . import seedvc_engine

    p = progress or (lambda f, m: None)
    p(0.02, "Loading models")
    model = seedvc_engine.get_model("quality")
    ref = store.vc_ref(vid, "quality")
    p(0.05, "Loading audio")
    wav = load_audio(input_path, model.sr)
    peak = float(np.max(np.abs(wav))) or 1.0
    wav = wav / peak * 0.9
    out = model.convert_long(wav, ref, steps=steps, cfg_rate=cfg_rate,
                             progress=lambda f: p(0.05 + 0.93 * f, "Converting voice"))
    p(1.0, "Done")
    return rms_normalize(out, -18.0), model.sr


# ---------------------------------------------------------------- helpers
def _wav_bytes(wav: np.ndarray, sr: int) -> bytes:
    import soundfile as sf
    buf = io.BytesIO()
    sf.write(buf, wav.astype(np.float32), sr, format="WAV", subtype="PCM_16")
    return buf.getvalue()


def _wav_from_bytes(data: bytes, sr: int) -> np.ndarray:
    import soundfile as sf
    import librosa
    wav, file_sr = sf.read(io.BytesIO(data), dtype="float32")
    if wav.ndim > 1:
        wav = wav.mean(axis=1)
    if file_sr != sr:
        wav = librosa.resample(wav, orig_sr=file_sr, target_sr=sr)
    return wav.astype(np.float32)


def _tensor_bytes(state: dict) -> bytes:
    buf = io.BytesIO()
    torch.save(state, buf)
    return buf.getvalue()

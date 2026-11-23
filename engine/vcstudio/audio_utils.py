"""Audio helpers: loading, loudness normalisation, reference-clip selection and
silence-aware segmentation for long recordings."""
from __future__ import annotations

import subprocess
import shutil
from pathlib import Path

import numpy as np
import soundfile as sf


def load_audio(path: str | Path, sr: int) -> np.ndarray:
    """Load any audio file (wav/mp3/flac/m4a/ogg...) as mono float32 at `sr`.

    ffmpeg is used when available because it decodes every common format and
    resamples well; librosa is the fallback.
    """
    path = str(path)
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg:
        cmd = [ffmpeg, "-v", "error", "-nostdin", "-i", path, "-ac", "1", "-ar", str(sr),
               "-f", "f32le", "-"]
        out = subprocess.run(cmd, capture_output=True, check=True).stdout
        return np.frombuffer(out, dtype=np.float32).copy()
    import librosa

    return librosa.load(path, sr=sr, mono=True)[0].astype(np.float32)


def save_wav(path: str | Path, audio: np.ndarray, sr: int) -> None:
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    sf.write(str(path), np.asarray(audio, dtype=np.float32), sr, subtype="PCM_16")


def peak_normalize(audio: np.ndarray, peak: float = 0.95) -> np.ndarray:
    m = float(np.max(np.abs(audio))) if audio.size else 0.0
    return audio if m < 1e-6 else audio * (peak / m)


def rms_normalize(audio: np.ndarray, target_dbfs: float = -20.0, peak: float = 0.97) -> np.ndarray:
    """Bring speech to a consistent loudness without clipping."""
    rms = float(np.sqrt(np.mean(audio**2))) if audio.size else 0.0
    if rms < 1e-6:
        return audio
    gain = 10 ** (target_dbfs / 20) / rms
    out = audio * gain
    m = float(np.max(np.abs(out)))
    if m > peak:
        out = out * (peak / m)
    return out.astype(np.float32)


def frame_db(audio: np.ndarray, sr: int, frame_ms: float = 20.0) -> np.ndarray:
    hop = max(1, int(sr * frame_ms / 1000))
    n = len(audio) // hop
    if n == 0:
        return np.array([-120.0])
    frames = audio[: n * hop].reshape(n, hop)
    rms = np.sqrt(np.mean(frames**2, axis=1) + 1e-12)
    return 20 * np.log10(rms + 1e-12)


def speech_mask(audio: np.ndarray, sr: int, frame_ms: float = 20.0, rel_db: float = -35.0) -> np.ndarray:
    """Per-frame boolean voice-activity mask relative to the loud level of the file."""
    db = frame_db(audio, sr, frame_ms)
    loud = np.percentile(db, 95)
    return db > (loud + rel_db)


def pick_reference(audio: np.ndarray, sr: int, target_sec: float = 15.0,
                   min_sec: float = 6.0) -> tuple[np.ndarray, float, float]:
    """Choose the cleanest, most speech-dense window of about `target_sec`.

    Scoring favours windows that are mostly speech, have steady loudness
    (no shouting / music swells) and start and end in a pause so words are
    not cut. Returns (clip, start_sec, end_sec).
    """
    total = len(audio) / sr
    if total <= target_sec + 1:
        return audio, 0.0, total
    frame_ms = 20.0
    fps = 1000 / frame_ms
    db = frame_db(audio, sr, frame_ms)
    loud = np.percentile(db, 95)
    voiced = db > loud - 35
    win = int(target_sec * fps)
    step = int(0.5 * fps)
    best, best_score = (0, win), -1e9
    for s in range(0, len(db) - win, step):
        e = s + win
        seg_v = voiced[s:e]
        density = seg_v.mean()
        seg_db = db[s:e][seg_v]
        steadiness = -np.std(seg_db) if seg_db.size else -99
        # clipping / very hot frames are penalised
        hot = np.mean(db[s:e] > loud + 3)
        score = density * 10 + steadiness * 0.3 - hot * 20
        if score > best_score:
            best_score, best = score, (s, e)
    s, e = best
    # snap edges outward/inward to the nearest pause (within 1.5 s)
    s, e = _snap_to_pause(voiced, s, int(1.5 * fps), left=True), _snap_to_pause(voiced, e, int(1.5 * fps), left=False)
    if (e - s) / fps < min_sec:
        s, e = best
    a, b = int(s * frame_ms / 1000 * sr), int(e * frame_ms / 1000 * sr)
    return audio[a:b].copy(), a / sr, b / sr


def _snap_to_pause(voiced: np.ndarray, idx: int, radius: int, left: bool) -> int:
    lo, hi = max(0, idx - radius), min(len(voiced) - 1, idx + radius)
    cands = [i for i in range(lo, hi) if not voiced[i]]
    if not cands:
        return idx
    return min(cands, key=lambda i: abs(i - idx))


def split_on_silence(audio: np.ndarray, sr: int, max_sec: float = 14.0, min_sec: float = 2.0,
                     frame_ms: float = 20.0) -> list[tuple[int, int]]:
    """Split a long recording into (start, end) sample ranges no longer than
    `max_sec`, cutting inside the quietest pause available so that sentences
    stay whole. Silence between ranges is preserved by the caller."""
    n = len(audio)
    if n / sr <= max_sec:
        return [(0, n)]
    hop = int(sr * frame_ms / 1000)
    db = frame_db(audio, sr, frame_ms)
    segments = []
    start = 0
    max_f = int(max_sec * 1000 / frame_ms)
    min_f = int(min_sec * 1000 / frame_ms)
    nf = len(db)
    sf_ = 0
    while sf_ < nf:
        if nf - sf_ <= max_f:
            segments.append((sf_ * hop, n))
            break
        lo, hi = sf_ + min_f, sf_ + max_f
        # prefer the longest run of quiet frames in the search window
        window = db[lo:hi]
        smooth = np.convolve(window, np.ones(5) / 5, mode="same")
        cut = lo + int(np.argmin(smooth))
        segments.append((sf_ * hop, cut * hop))
        sf_ = cut
    del start
    return segments

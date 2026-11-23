"""Cartoon talking: audio-driven jaw animation for characters without a human face.

SadTalker and LatentSync only work on human faces, so cartoon animals, the moon
or a talking pot stayed silent-mouthed. Here an open-vocabulary detector
(OWLv2, Apache-2.0) finds the mouth, and the voice's loudness drives a classic
cartoon jaw: the lower jaw drops and a dark mouth with a tongue opens, about 25
times a second, following the syllables.

Two entry points:
  * animate_portrait(image, audio, out)  - Fast mode: a talking portrait video
  * animate_clip(video, audio, out, box) - Cinematic: the mouth is tracked
    through an animated shot (inside the speaker's box) and the jaw is added
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import Optional

import numpy as np

from . import gpu

OWL = "google/owlv2-base-patch16-ensemble"
FPS = 25
_model = None
_proc = None


def _unload():
    global _model, _proc
    _model, _proc = None, None


gpu.register("cartoon_talk", _unload)


def _load():
    global _model, _proc
    if _model is None:
        import torch
        from transformers import Owlv2ForObjectDetection, Owlv2Processor
        gpu.reserve("cartoon_talk")
        _proc = Owlv2Processor.from_pretrained(OWL)
        _model = Owlv2ForObjectDetection.from_pretrained(OWL, torch_dtype=torch.float16).to("cuda").eval()
        gpu.mark_loaded("cartoon_talk")
    return _model, _proc


def detect_mouth(img) -> Optional[tuple[float, float, float, float]]:
    """PIL image -> (cx, cy, width, score) of the most likely mouth, in pixels."""
    import torch
    model, proc = _load()
    q = [["a mouth", "a beak"]]
    inp = proc(text=q, images=img, return_tensors="pt").to("cuda")
    inp["pixel_values"] = inp["pixel_values"].half()
    with torch.no_grad():
        o = model(**inp)
    s = max(img.size)   # OWLv2 pads to a square
    r = proc.post_process_grounded_object_detection(o, threshold=0.04, target_sizes=torch.tensor([[s, s]]).cuda())[0]
    best_mouth, best_beak = None, None
    for box, sc, lab in zip(r["boxes"].tolist(), r["scores"].tolist(), r["labels"].tolist()):
        x0, y0, x1, y1 = box
        if x1 <= x0 or y1 <= y0:
            continue
        if int(lab) == 0 and (best_mouth is None or sc > best_mouth[3]):
            best_mouth = ((x0 + x1) / 2, (y0 + y1) / 2, x1 - x0, sc)
        if int(lab) == 1 and (best_beak is None or sc > best_beak[3]):
            # a beak opens along its lower third
            best_beak = ((x0 + x1) / 2, y0 + (y1 - y0) * 0.68, (x1 - x0) * 0.75, sc)
    if best_mouth and best_beak and best_beak[3] > best_mouth[3] * 2.5:
        return best_beak
    return best_mouth or best_beak


def portrait_mouth(image: str) -> Optional[tuple[float, float, float]]:
    """Mouth of a character portrait, cached next to it as <name>_mouth.json."""
    from PIL import Image
    cache = Path(image).with_name(Path(image).stem + "_mouth.json")
    if cache.exists():
        try:
            m = json.loads(cache.read_text())
            return (m["x"], m["y"], m["w"]) if m else None
        except Exception:
            pass
    m = detect_mouth(Image.open(image).convert("RGB"))
    cache.write_text(json.dumps({"x": m[0], "y": m[1], "w": m[2]} if m else {}))
    return m[:3] if m else None


# ------------------------------------------------------------------ audio -> jaw opening
def envelope(audio: str, fps: int = FPS) -> np.ndarray:
    """Per-video-frame mouth opening 0..1 from speech loudness (syllable-paced)."""
    import soundfile as sf
    y, sr = sf.read(audio, dtype="float32", always_2d=True)
    y = y.mean(axis=1)
    hop = sr // fps
    n = max(1, int(np.ceil(len(y) / hop)))
    y = np.pad(y, (0, n * hop - len(y)))
    rms = np.sqrt((y.reshape(n, hop) ** 2).mean(axis=1) + 1e-9)
    ref = np.percentile(rms, 92) + 1e-6
    e = np.clip((rms / ref - 0.12) / 0.88, 0, 1)
    out = np.zeros_like(e)
    v = 0.0
    for i, x in enumerate(e):          # quick open, slightly slower close: reads as talking, not flicker
        v = v + (x - v) * (0.75 if x > v else 0.45)
        out[i] = v
    return out ** 0.8


# ------------------------------------------------------------------ the jaw warp
def jaw(frame: np.ndarray, cx: float, cy: float, mw: float, o: float) -> np.ndarray:
    """Open the mouth at (cx, cy) by `o` pixels: drop the lower jaw and paint the opening."""
    import cv2
    if o < 1.0:
        return frame
    h, w = frame.shape[:2]
    x0, x1 = int(max(0, cx - 1.25 * mw)), int(min(w, cx + 1.25 * mw))
    y0, y1 = int(max(0, cy - 0.3 * mw)), int(min(h, cy + 2.4 * mw))
    if x1 - x0 < 4 or y1 - y0 < 4:
        return frame
    xs, ys = np.meshgrid(np.arange(x0, x1, dtype=np.float32), np.arange(y0, y1, dtype=np.float32))
    wx = 0.5 * (1 + np.cos(np.pi * np.clip((xs - cx) / (1.15 * mw), -1, 1)))
    t = (ys - cy) / mw
    wy = np.where(t < 0, 0.0, np.where(t < 1.3, 1.0, 0.5 * (1 + np.cos(np.pi * np.clip((t - 1.3) / 1.0, 0, 1)))))
    d = (o * wx * wy).astype(np.float32)
    roi = cv2.remap(frame, xs, ys - d, interpolation=cv2.INTER_LINEAR, borderMode=cv2.BORDER_REFLECT)
    # the opening: dark mouth, a hint of tongue, soft edges
    ax, ay = max(2.0, 0.30 * mw), o / 2 + 0.5
    ex, ey = cx - x0, cy - y0 + o / 2
    mask = np.zeros(roi.shape[:2], np.float32)
    cv2.ellipse(mask, (int(ex), int(ey)), (int(ax), int(ay)), 0, 0, 360, 1.0, -1, cv2.LINE_AA)
    tongue = np.zeros_like(mask)
    cv2.ellipse(tongue, (int(ex), int(ey + ay * 0.55)), (int(ax * 0.62), int(max(1, ay * 0.5))), 0, 0, 360, 1.0, -1, cv2.LINE_AA)
    k = int(max(3, mw * 0.05)) | 1
    mask = cv2.GaussianBlur(mask, (k, k), 0)[..., None]
    tongue = (cv2.GaussianBlur(tongue, (k, k), 0)[..., None]) * mask
    inner = np.array([38, 14, 22], np.float32)          # RGB dark mouth
    tcol = np.array([196, 84, 96], np.float32)          # tongue
    r = roi.astype(np.float32)
    r = r * (1 - mask) + inner * mask
    r = r * (1 - tongue * 0.85) + tcol * tongue * 0.85
    out = frame.copy()
    out[y0:y1, x0:x1] = np.clip(r, 0, 255).astype(np.uint8)
    return out


def _writer(out: str, w: int, h: int):
    import imageio_ffmpeg
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    flags = 0x08000000 if sys.platform == "win32" else 0
    return subprocess.Popen([imageio_ffmpeg.get_ffmpeg_exe(), "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24",
                             "-s", f"{w}x{h}", "-r", str(FPS), "-i", "-", "-c:v", "libx264", "-preset", "veryfast",
                             "-crf", "17", "-pix_fmt", "yuv420p", out], stdin=subprocess.PIPE, creationflags=flags)


def animate_portrait(image: str, audio: str, out: str) -> Optional[str]:
    """Still portrait + voice -> talking portrait video (same size as the image)."""
    from PIL import Image
    m = portrait_mouth(image)
    if not m:
        return None
    im = Image.open(image).convert("RGB")
    im = im.resize((im.width // 2 * 2, im.height // 2 * 2))
    base = np.asarray(im)
    cx, cy, mw = m
    amp = 0.42 * mw
    wr = _writer(out, base.shape[1], base.shape[0])
    for e in envelope(audio):
        wr.stdin.write(jaw(base, cx, cy, mw, amp * e).tobytes())
    wr.stdin.close()
    wr.wait()
    return out if wr.returncode == 0 else None


def animate_clip(video: str, audio: str, out: str, box: Optional[list] = None) -> Optional[str]:
    """Animated shot + voice -> the same shot with the speaker's mouth talking.

    box = speaker region [x, y, w, h] as fractions of the frame (from the keyframe
    layout); the mouth is searched inside it so the listener's mouth stays closed."""
    import imageio_ffmpeg
    from PIL import Image
    rd = imageio_ffmpeg.read_frames(video)
    meta = next(rd)
    w, h = meta["size"]
    frames = [np.frombuffer(f, np.uint8).reshape(h, w, 3) for f in rd]
    if not frames:
        return None
    bx = [0.0, 0.0, 1.0, 1.0] if not box else box
    X0, Y0 = int(max(0, bx[0]) * w), int(max(0, bx[1]) * h)
    X1, Y1 = int(min(1, bx[0] + bx[2]) * w), int(min(1, bx[1] + bx[3]) * h)
    env = envelope(audio)
    n = len(env)
    # ping-pong the shot to cover the whole line
    order = list(range(len(frames))) + list(range(len(frames) - 2, 0, -1))
    seq = [order[i % len(order)] for i in range(n)]
    # track the mouth every 4th source frame, interpolate the rest
    track: dict[int, tuple] = {}
    for fi in sorted(set(seq))[::4] + [max(seq)]:
        crop = Image.fromarray(frames[fi][Y0:Y1, X0:X1])
        m = detect_mouth(crop)
        if m and m[3] > 0.05:
            track[fi] = (m[0] + X0, m[1] + Y0, m[2])
    if not track:
        return None
    keys = sorted(track)
    arr = np.array([track[k] for k in keys])
    med = np.median(arr, axis=0)
    good = [k for k in keys if abs(track[k][0] - med[0]) < med[2] * 1.2 and abs(track[k][1] - med[1]) < med[2] * 1.2]
    keys = good or keys

    def at(fi):
        xs = np.array(keys, float)
        return tuple(np.interp(fi, xs, [track[k][j] for k in keys]) for j in range(3))
    wr = _writer(out, w // 2 * 2, h // 2 * 2)
    for i in range(n):
        f = frames[seq[i]][: h // 2 * 2, : w // 2 * 2]
        cx, cy, mw = at(seq[i])
        wr.stdin.write(np.ascontiguousarray(jaw(f, cx, cy, mw, 0.42 * mw * env[i])).tobytes())
    wr.stdin.close()
    wr.wait()
    return out if wr.returncode == 0 else None

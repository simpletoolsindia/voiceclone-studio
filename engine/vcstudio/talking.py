"""Talking-face animation (audio-driven portrait) via SadTalker.

SadTalker (Apache-2.0) needs old pinned dependencies, so it lives in its own
virtual environment (``engine/.venv-talk``) and runs as a subprocess; this
module drives the batch runner and turns its output into job progress."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Callable, Optional

from . import gpu

ENGINE = Path(__file__).resolve().parents[1]
SADTALKER = Path(os.environ.get("SADTALKER_DIR", ENGINE / "third_party" / "SadTalker"))
RUNNER = ENGINE / "scripts" / "talk_runner.py"


def talk_python() -> Path:
    sub = "Scripts/python.exe" if sys.platform == "win32" else "bin/python"
    return ENGINE / ".venv-talk" / sub


def available() -> bool:
    return talk_python().exists() and (SADTALKER / "checkpoints" / "SadTalker_V0.0.2_256.safetensors").exists()


gpu.register("talking", lambda: None)   # subprocess: memory is freed when it exits


def _prepare_image(src: str, tmp: Path, idx: int) -> str:
    """SadTalker cost grows with image size; 768 px is plenty for a face card."""
    from PIL import Image
    im = Image.open(src).convert("RGB")
    s = 768 / max(im.size)
    if s < 1:
        im = im.resize((int(im.width * s) // 2 * 2, int(im.height * s) // 2 * 2), Image.LANCZOS)
    out = tmp / f"face{idx}.png"
    im.save(out)
    return str(out)


def animate_many(jobs: list[dict], progress: Optional[Callable[[float, str], None]] = None,
                 enhancer: str = "gfpgan") -> list[Optional[str]]:
    """jobs: [{image, audio, out, expression}] -> list of output paths (None on failure)."""
    p = progress or (lambda f, m="": None)
    if not available():
        raise RuntimeError("Talking faces are not installed. Run engine/scripts/setup_engine.ps1 -TalkingFaces.")
    tmp = Path(tempfile.mkdtemp(prefix="vcs_talk_"))
    images: dict[str, str] = {}
    norm = []
    for j in jobs:
        if j["image"] not in images:
            images[j["image"]] = _prepare_image(j["image"], tmp, len(images))
        norm.append({**j, "image": images[j["image"]]})
    spec = tmp / "jobs.json"
    spec.write_text(json.dumps({"size": 256, "enhancer": enhancer, "jobs": norm}), encoding="utf-8")
    results: list[Optional[str]] = [None] * len(jobs)
    errors = []
    flags = 0x08000000 if sys.platform == "win32" else 0
    proc = subprocess.Popen([str(talk_python()), str(RUNNER), str(spec)], cwd=str(SADTALKER),
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, encoding="utf-8",
                            errors="replace", creationflags=flags)
    try:
        for line in proc.stdout:
            parts = line.strip().split(" ", 3)
            if parts[0] == "PROGRESS" and len(parts) >= 3:
                done, total = int(parts[1]), max(1, int(parts[2]))
                p(0.02 + 0.97 * done / total, parts[3] if len(parts) > 3 else "")
            elif parts[0] == "RESULT":
                results[int(parts[1])] = line.strip().split(" ", 2)[2]
            elif parts[0] == "FAILED":
                errors.append(line.strip())
        proc.wait()
    except BaseException:
        proc.kill()
        raise
    if all(r is None for r in results):
        raise RuntimeError("Talking-face animation failed: " + ("; ".join(errors[:2]) or "unknown error"))
    return results


def animate(image: str, audio: str, out: Path, expressive: float = 1.0,
            progress: Optional[Callable[[float, str], None]] = None) -> dict:
    res = animate_many([{"image": image, "audio": audio, "out": str(out), "expression": expressive}], progress)
    return {"path": res[0]}

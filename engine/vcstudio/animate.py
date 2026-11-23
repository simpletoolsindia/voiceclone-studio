"""Cinematic shot animation.

1. keyframe(): compose a shot - the empty location plate with character
   cutouts placed for a two-shot or close-up (soft contact shadows).
2. i2v(): LTX-Video 2B distilled (0.9.8) animates the keyframe from a text
   prompt describing the action - natural head, hand and body motion, blinks,
   moving environment. ~4 GB, fully on the GPU, 8 denoising steps. The T5 text
   encoder runs on its own, then is unloaded, so peak RAM/VRAM stay low.
3. lipsync_many(): LatentSync 1.5 (own venv, subprocess) re-draws the
   speaker's mouth to match the Tamil line.

LTX-Video weights: LTX-Video Open Weights License (free incl. commercial use
for entities under $10M annual revenue). LatentSync: OpenRAIL++.
"""
from __future__ import annotations

import gc
import json
import os
import subprocess
import sys
import tempfile
import threading
from pathlib import Path
from typing import Callable, Optional

import torch

from . import gpu

LTX_REPO = "Lightricks/LTX-Video"
LTX_FILE = "https://huggingface.co/Lightricks/LTX-Video/blob/main/ltxv-2b-0.9.8-distilled.safetensors"
DISTILLED_TIMESTEPS = [1000, 993, 987, 981, 975, 909, 725, 0.03]
NEGATIVE = ("worst quality, static, frozen, blurry, jittery, flicker, distorted face, deformed hands, extra fingers, "
            "morphing, melting, text, subtitles, watermark")
FPS = 24

ENGINE = Path(__file__).resolve().parents[1]
LATENTSYNC = Path(os.environ.get("LATENTSYNC_DIR", ENGINE / "third_party" / "LatentSync"))

_pipe = None
_lock = threading.Lock()


# ------------------------------------------------------------------ keyframes
def keyframe(plate: str, actors: list[dict], size: tuple[int, int], out: str) -> dict:
    """actors: [{image, mask, x, y, size, dim}] in output pixel coordinates (listener first)."""
    from PIL import Image, ImageEnhance, ImageFilter

    w, h = size
    bg = Image.open(plate).convert("RGB")
    s = max(w / bg.width, h / bg.height)
    bg = bg.resize((round(bg.width * s), round(bg.height * s)), Image.LANCZOS)
    bg = bg.crop(((bg.width - w) // 2, (bg.height - h) // 2, (bg.width - w) // 2 + w, (bg.height - h) // 2 + h))
    canvas = bg.convert("RGBA")
    for a in actors:
        sz = int(a["size"])
        img = Image.open(a["image"]).convert("RGB").resize((sz, sz), Image.LANCZOS)
        if a.get("dim"):
            img = ImageEnhance.Brightness(img).enhance(0.88)
        mask = Image.open(a["mask"]).convert("L").resize((sz, sz), Image.LANCZOS)
        shadow = Image.new("RGBA", (sz, sz), (0, 0, 0, 0))
        shadow.putalpha(mask.point(lambda v: int(v * 0.45)).filter(ImageFilter.GaussianBlur(18)))
        canvas.alpha_composite(shadow, (int(a["x"]) + 20, int(a["y"]) + 14))
        person = img.convert("RGBA")
        person.putalpha(mask)
        canvas.alpha_composite(person, (int(a["x"]), int(a["y"])))
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    canvas.convert("RGB").save(out, quality=95)
    return {"path": out}


# ------------------------------------------------------------------ LTX image-to-video
def _t5_cache_dir() -> Path:
    from .config import data_dir
    d = data_dir() / "cache" / "t5"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _encode(prompts: list[str]) -> list[tuple[torch.Tensor, torch.Tensor]]:
    """T5 prompt embeddings, cached on disk so the 9.5 GB text encoder is only
    loaded when a prompt has never been seen before."""
    import hashlib

    keys = [hashlib.sha1(("ltx-t5-v1|" + p).encode("utf-8")).hexdigest() for p in prompts]
    cache = _t5_cache_dir()
    out: list = [None] * len(prompts)
    missing = []
    for i, k in enumerate(keys):
        f = cache / f"{k}.pt"
        if f.exists():
            try:
                d = torch.load(f, map_location="cuda", weights_only=True)
                out[i] = (d["e"], d["m"])
                continue
            except Exception:
                pass
        missing.append(i)
    if missing:
        from transformers import T5EncoderModel, T5TokenizerFast

        tok = T5TokenizerFast.from_pretrained(LTX_REPO, subfolder="tokenizer")
        te = T5EncoderModel.from_pretrained(LTX_REPO, subfolder="text_encoder", torch_dtype=torch.bfloat16).to("cuda")
        with torch.no_grad():
            for i in missing:
                ids = tok(prompts[i], padding="max_length", max_length=128, truncation=True, return_tensors="pt").to("cuda")
                e, m = te(ids.input_ids, attention_mask=ids.attention_mask)[0], ids.attention_mask
                out[i] = (e, m)
                torch.save({"e": e.cpu(), "m": m.cpu()}, cache / f"{keys[i]}.pt")
        del te
        gc.collect()
        torch.cuda.empty_cache()
    return out


def _get_pipe():
    global _pipe
    if _pipe is None:
        from diffusers import AutoencoderKLLTXVideo, LTXConditionPipeline, LTXVideoTransformer3DModel

        transformer = LTXVideoTransformer3DModel.from_single_file(LTX_FILE, torch_dtype=torch.bfloat16)
        vae = AutoencoderKLLTXVideo.from_single_file(LTX_FILE, torch_dtype=torch.bfloat16)
        pipe = LTXConditionPipeline.from_pretrained(LTX_REPO, transformer=transformer, vae=vae, text_encoder=None,
                                                    tokenizer=None, torch_dtype=torch.bfloat16).to("cuda")
        from diffusers import FlowMatchEulerDiscreteScheduler
        # the distilled model uses fixed timesteps, so no resolution-dependent shift
        pipe.scheduler = FlowMatchEulerDiscreteScheduler.from_config(pipe.scheduler.config, use_dynamic_shifting=False)
        pipe.vae.enable_tiling()
        _pipe = pipe
        gpu.mark_loaded("video")
    return _pipe


def unload():
    global _pipe
    _pipe = None


gpu.register("video", unload)


def frames_for(seconds: float, cap: int = 97) -> int:
    """LTX needs 8k+1 frames. Clips are capped at 4 s: identity holds best and a clip takes ~11 s on a
    12 GB card; longer lines are extended by LatentSync (ping-pong) / the compositor."""
    n = int(round(seconds * FPS))
    n = max(25, min(cap, n))
    return (n - 1) // 8 * 8 + 1


def i2v_many(shots: list[dict], progress: Optional[Callable[[float, str], None]] = None) -> list[Optional[str]]:
    """shots: [{image, prompt, seconds, width, height, out}] -> output paths."""
    from diffusers.pipelines.ltx.pipeline_ltx_condition import LTXVideoCondition
    from diffusers.utils import export_to_video
    from PIL import Image

    p = progress or (lambda f, m="": None)
    n = len(shots)
    p(0.01, "Reading the shot descriptions")
    with _lock:
        embeds = _encode([s["prompt"] for s in shots] + [NEGATIVE])
        neg = embeds.pop()
        p(0.05, "Loading the animation model")
        pipe = _get_pipe()
        results: list[Optional[str]] = []
        for i, (s, (pe, pm)) in enumerate(zip(shots, embeds)):
            base = 0.05 + 0.95 * i / n
            span = 0.95 / n

            def cb(_pipe, step, _t, kw, base=base, span=span, i=i):
                p(base + span * (step + 1) / len(DISTILLED_TIMESTEPS), f"Animating shot {i + 1} of {n}", s["image"])
                return kw

            try:
                w, h = int(s["width"]) // 32 * 32, int(s["height"]) // 32 * 32
                img = Image.open(s["image"]).convert("RGB").resize((w, h), Image.LANCZOS)
                frames = pipe(conditions=[LTXVideoCondition(image=img, frame_index=0)],
                              prompt_embeds=pe, prompt_attention_mask=pm,
                              negative_prompt_embeds=neg[0], negative_prompt_attention_mask=neg[1],
                              width=w, height=h, num_frames=frames_for(float(s["seconds"])),
                              timesteps=DISTILLED_TIMESTEPS, decode_timestep=0.05, decode_noise_scale=0.025,
                              image_cond_noise_scale=0.0, guidance_scale=1.0, guidance_rescale=0.7,
                              generator=torch.Generator("cuda").manual_seed(int(s.get("seed", 7))),
                              callback_on_step_end=cb, output_type="pil").frames[0]
                Path(s["out"]).parent.mkdir(parents=True, exist_ok=True)
                export_to_video(frames, s["out"], fps=FPS)
                results.append(s["out"])
            except torch.cuda.OutOfMemoryError:
                torch.cuda.empty_cache()
                results.append(None)
            p(base + span, f"Animated shot {i + 1} of {n}")
    return results


# ------------------------------------------------------------------ lip-sync
def sync_python() -> Path:
    sub = "Scripts/python.exe" if sys.platform == "win32" else "bin/python"
    return ENGINE / ".venv-sync" / sub


def lipsync_available() -> bool:
    return sync_python().exists() and (LATENTSYNC / "checkpoints" / "latentsync_unet.pt").exists()


gpu.register("lipsync", lambda: None)   # subprocess: memory is freed when it exits


def lipsync_many(jobs: list[dict], progress: Optional[Callable[[float, str], None]] = None) -> list[Optional[str]]:
    """jobs: [{video, audio, out}] -> output paths (None on failure)."""
    p = progress or (lambda f, m="": None)
    if not lipsync_available():
        raise RuntimeError("Lip-sync is not installed. Run engine/scripts/setup_engine.ps1 -Cinematic.")
    spec = Path(tempfile.mkdtemp(prefix="vcs_sync_")) / "jobs.json"
    spec.write_text(json.dumps({"steps": 20, "guidance": 1.5, "jobs": jobs}), encoding="utf-8")
    results: list[Optional[str]] = [None] * len(jobs)
    flags = 0x08000000 if sys.platform == "win32" else 0
    env = {**os.environ, "PYTHONIOENCODING": "utf-8"}
    proc = subprocess.Popen([str(sync_python()), str(ENGINE / "scripts" / "sync_runner.py"), str(spec)],
                            cwd=str(LATENTSYNC), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
                            encoding="utf-8", errors="replace", creationflags=flags, env=env)
    from .jobs import queue as _queue
    _queue.register_child(proc)
    try:
        for line in proc.stdout:
            parts = line.strip().split(" ", 3)
            if parts[0] == "PROGRESS" and len(parts) >= 3:
                p(0.02 + 0.97 * int(parts[1]) / max(1, int(parts[2])), parts[3] if len(parts) > 3 else "")
            elif parts[0] == "RESULT":
                k = int(parts[1])
                results[k] = line.strip().split(" ", 2)[2]
                if jobs[k].get("image"):
                    p(0.02 + 0.97 * (k + 1) / max(1, len(jobs)), "", jobs[k]["image"])
        proc.wait()
    except BaseException:
        try:
            import psutil
            psutil.Process(proc.pid).resume()
        except Exception:
            pass
        proc.kill()
        raise
    finally:
        _queue.unregister_child(proc)
    return results

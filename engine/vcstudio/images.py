"""Image generation for characters, scenes and thumbnails.

Z-Image-Turbo (Tongyi-MAI, Apache-2.0, 6B, 8-9 steps) with model CPU
offload, so it runs on 12 GB cards. Outputs are free to use commercially.
"""
from __future__ import annotations

import threading
import time
from pathlib import Path
from typing import Callable, Optional

import torch

from . import gpu

MODEL_ID = "Tongyi-MAI/Z-Image-Turbo"
_pipe = None
_lock = threading.Lock()

STYLES = {
    "animated": "3D animated feature film style, Pixar-quality rendering, expressive, soft global illumination, rich colours",
    "cinematic": "cinematic film still, photorealistic, shallow depth of field, 35mm lens, natural skin texture, film grain",
    "storybook": "hand-painted storybook illustration, warm gouache textures, gentle lighting, whimsical",
}
QUALITY = "highly detailed, sharp focus, beautiful composition, no text, no watermark, no captions"


def get_pipe():
    global _pipe
    with _lock:
        if _pipe is None:
            from diffusers import ZImagePipeline

            pipe = ZImagePipeline.from_pretrained(MODEL_ID, torch_dtype=torch.bfloat16)
            if torch.cuda.is_available():
                free, total = torch.cuda.mem_get_info()
                # The full pipeline needs ~20 GB in bf16. On smaller cards store the
                # 6B transformer in FP8 (computed in bf16) and offload idle parts:
                # ~8 GB peak and ~17 s per 1024px image on an RTX 4070 SUPER.
                # Without FP8 the transformer spills into shared memory (>10x slower).
                if total > 22e9:
                    pipe.to("cuda")
                else:
                    pipe.transformer.enable_layerwise_casting(storage_dtype=torch.float8_e4m3fn,
                                                              compute_dtype=torch.bfloat16)
                    pipe.enable_model_cpu_offload()
            _pipe = pipe
            gpu.mark_loaded("image")
        return _pipe


def unload():
    global _pipe
    with _lock:
        _pipe = None


gpu.register("image", unload)


def build_prompt(prompt: str, style: str, kind: str = "scene") -> str:
    style_txt = STYLES.get(style, STYLES["animated"])
    if kind == "portrait":
        return (f"Head and shoulders portrait of {prompt}, facing the camera, looking directly at the viewer, "
                f"mouth closed, gentle expression, face centred and well lit, softly blurred Tamil Nadu background, "
                f"{style_txt}, {QUALITY}")
    if kind == "thumbnail":
        return f"{prompt}, dramatic YouTube thumbnail composition, strong emotions, vivid colours, {style_txt}, {QUALITY}"
    return f"{prompt}, {style_txt}, wide establishing shot, {QUALITY}"


def generate(prompt: str, out: Path, width: int = 1344, height: int = 768, seed: Optional[int] = None,
             style: str = "animated", kind: str = "scene", steps: int = 9,
             progress: Optional[Callable[[float, str], None]] = None) -> dict:
    p = progress or (lambda f, m="": None)
    p(0.02, "Loading image model (first time takes a minute)")
    pipe = get_pipe()
    seed = int(seed if seed is not None else time.time() * 1000) % (2**31)
    width, height = (max(512, min(2048, v)) // 64 * 64 for v in (width, height))
    full = build_prompt(prompt, style, kind)

    def cb(pipe_, i, t, kw):
        p(0.1 + 0.85 * (i + 1) / steps, f"Painting image - step {i + 1}/{steps}")
        return kw

    p(0.1, "Painting image")
    with _lock:
        img = pipe(prompt=full, width=width, height=height, num_inference_steps=steps, guidance_scale=0.0,
                   generator=torch.Generator("cpu").manual_seed(seed),
                   callback_on_step_end=cb).images[0]
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    img.save(out)
    p(1.0, "Image ready")
    return {"path": str(out), "seed": seed, "width": width, "height": height}

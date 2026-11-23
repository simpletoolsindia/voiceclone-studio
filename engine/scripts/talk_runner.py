"""Batch talking-face renderer (runs inside the isolated SadTalker venv).

Loads SadTalker once and animates many (portrait, audio) pairs, caching the
3D face fit per portrait so a character who speaks 30 lines is analysed only
once. Reads a JSON job list, prints machine-readable progress:

    PROGRESS <done> <total> <message>
    RESULT <index> <output path>
    FAILED <index> <message>

Usage (cwd = SadTalker checkout):
    python talk_runner.py jobs.json
jobs.json: {"size": 256, "enhancer": "gfpgan", "jobs": [{"image":..., "audio":..., "out":..., "expression": 1.0}]}
"""
import json
import os
import shutil
import sys
import tempfile
import traceback

import torch

sys.path.insert(0, os.getcwd())
from src.utils.preprocess import CropAndExtract  # noqa: E402
from src.test_audio2coeff import Audio2Coeff  # noqa: E402
from src.facerender.animate import AnimateFromCoeff  # noqa: E402
from src.generate_batch import get_data  # noqa: E402
from src.generate_facerender_batch import get_facerender_data  # noqa: E402
from src.utils.init_path import init_path  # noqa: E402


def say(*parts):
    print(*parts, flush=True)


def main():
    spec = json.load(open(sys.argv[1], encoding="utf-8"))
    jobs = spec["jobs"]
    size = int(spec.get("size", 256))
    enhancer = spec.get("enhancer", "gfpgan") or None
    preprocess = spec.get("preprocess", "full")
    device = "cuda" if torch.cuda.is_available() else "cpu"
    total = len(jobs)
    say("PROGRESS", 0, total, "Loading talking-face models")
    paths = init_path("./checkpoints", os.path.join(os.getcwd(), "src/config"), size, False, preprocess)
    pre = CropAndExtract(paths, device)
    a2c = Audio2Coeff(paths, device)
    anim = AnimateFromCoeff(paths, device)
    work = tempfile.mkdtemp(prefix="talk_")
    face_cache = {}
    for i, job in enumerate(jobs):
        try:
            say("PROGRESS", i, total, f"Animating line {i + 1} of {total}")
            img, audio, out = job["image"], job["audio"], job["out"]
            jdir = os.path.join(work, f"j{i}")
            os.makedirs(jdir, exist_ok=True)
            if img not in face_cache:
                fdir = os.path.join(work, f"face{len(face_cache)}")
                os.makedirs(fdir, exist_ok=True)
                coeff, crop_pic, crop_info = pre.generate(img, fdir, preprocess, source_image_flag=True, pic_size=size)
                if coeff is None:
                    raise RuntimeError("no face found in the portrait - use a clear, front-facing face")
                face_cache[img] = (coeff, crop_pic, crop_info)
            coeff, crop_pic, crop_info = face_cache[img]
            batch = get_data(coeff, audio, device, None, still=True)
            coeff_path = a2c.generate(batch, jdir, int(job.get("pose_style", 0)), None)
            data = get_facerender_data(coeff_path, crop_pic, coeff, audio, int(spec.get("batch_size", 8)),
                                       None, None, None, expression_scale=float(job.get("expression", 1.0)),
                                       still_mode=True, preprocess=preprocess, size=size)
            result = anim.generate(data, jdir, img, crop_info, enhancer=enhancer, background_enhancer=None,
                                   preprocess=preprocess, img_size=size)
            os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
            shutil.move(result, out)
            shutil.rmtree(jdir, ignore_errors=True)
            say("RESULT", i, out)
        except Exception as e:  # keep going: one bad line must not kill the whole story
            traceback.print_exc()
            say("FAILED", i, f"{type(e).__name__}: {e}".replace("\n", " "))
    say("PROGRESS", total, total, "Talking faces done")
    shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()

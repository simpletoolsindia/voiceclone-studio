"""Batch lip-sync with LatentSync 1.5 (runs inside engine/.venv-sync, cwd = LatentSync checkout).

Loads the models once and lip-syncs many (video, audio) pairs. Progress protocol:
    PROGRESS <done> <total> <message>
    RESULT <index> <output path>
    FAILED <index> <message>
Usage: python sync_runner.py jobs.json
jobs.json: {"steps": 20, "guidance": 1.5, "jobs": [{"video": ..., "audio": ..., "out": ...}]}
"""
import json
import os
import shutil
import sys
import tempfile
import traceback

import torch

sys.path.insert(0, os.getcwd())
from omegaconf import OmegaConf  # noqa: E402
from diffusers import AutoencoderKL, DDIMScheduler  # noqa: E402
from latentsync.models.unet import UNet3DConditionModel  # noqa: E402
from latentsync.pipelines.lipsync_pipeline import LipsyncPipeline  # noqa: E402
from latentsync.whisper.audio2feature import Audio2Feature  # noqa: E402
from accelerate.utils import set_seed  # noqa: E402


def say(*parts):
    print(*parts, flush=True)


def main():
    spec = json.load(open(sys.argv[1], encoding="utf-8"))
    jobs = spec["jobs"]
    total = len(jobs)
    say("PROGRESS", 0, total, "Loading lip-sync model")
    config = OmegaConf.load("configs/unet/stage2.yaml")       # LatentSync 1.5 (256 px, whisper-tiny)
    dtype = torch.float16
    scheduler = DDIMScheduler.from_pretrained("configs")
    audio_encoder = Audio2Feature(model_path="checkpoints/whisper/tiny.pt", device="cuda",
                                  num_frames=config.data.num_frames, audio_feat_length=config.data.audio_feat_length)
    vae = AutoencoderKL.from_pretrained("stabilityai/sd-vae-ft-mse", torch_dtype=dtype)
    vae.config.scaling_factor = 0.18215
    vae.config.shift_factor = 0
    unet, _ = UNet3DConditionModel.from_pretrained(OmegaConf.to_container(config.model),
                                                   "checkpoints/latentsync_unet.pt", device="cpu")
    pipe = LipsyncPipeline(vae=vae, audio_encoder=audio_encoder, unet=unet.to(dtype=dtype), scheduler=scheduler).to("cuda")
    try:
        from DeepCache import DeepCacheSDHelper
        helper = DeepCacheSDHelper(pipe=pipe)
        helper.set_params(cache_interval=3, cache_branch_id=0)
        helper.enable()
    except Exception:
        pass
    work = tempfile.mkdtemp(prefix="sync_")
    for i, job in enumerate(jobs):
        try:
            say("PROGRESS", i, total, f"Lip-syncing line {i + 1} of {total}")
            set_seed(1247)
            tmp = os.path.join(work, f"j{i}")
            os.makedirs(tmp, exist_ok=True)
            out = job["out"]
            os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
            pipe(video_path=job["video"], audio_path=job["audio"], video_out_path=out,
                 num_frames=config.data.num_frames, num_inference_steps=int(job.get("steps", spec.get("steps", 20))),
                 guidance_scale=float(job.get("guidance", spec.get("guidance", 1.5))), weight_dtype=dtype,
                 width=config.data.resolution, height=config.data.resolution,
                 mask_image_path=config.data.mask_image_path, temp_dir=tmp)
            shutil.rmtree(tmp, ignore_errors=True)
            say("RESULT", i, out)
        except Exception as e:   # one bad shot must not stop the story
            traceback.print_exc()
            say("FAILED", i, f"{type(e).__name__}: {e}".replace("\n", " "))
    say("PROGRESS", total, total, "Lip-sync done")
    shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()

"""Local GPU inference worker for the VoiceClone Studio desktop app.

Started by the Rust app as a child process; listens only on 127.0.0.1 and
requires the per-launch token the app passes in (header ``X-Token`` or the
``token`` query parameter for WebSockets), so web pages in a browser cannot
drive it.

    python -m vcstudio.server --port 0 --token <secret> [--data-dir DIR]

Prints ``VCSTUDIO_READY port=<port>`` once listening.
"""
from __future__ import annotations

import argparse
import asyncio
import json
import os
import secrets
import socket
import sys
import time
import uuid
from pathlib import Path
from typing import Optional

import numpy as np
from fastapi import Depends, FastAPI, HTTPException, Request, WebSocket, WebSocketDisconnect
from pydantic import BaseModel

from . import __version__, gpu
from .config import data_dir
from .jobs import queue, sub_progress

TOKEN = os.environ.get("VCSTUDIO_TOKEN", "")
DATA = data_dir()
app = FastAPI(title="VoiceClone Studio engine", version=__version__)
_store = None


def store():
    global _store
    if _store is None:
        from .voices import VoiceStore
        _store = VoiceStore(DATA / "voices")
    return _store


def auth(request: Request):
    tok = request.headers.get("x-token") or request.query_params.get("token")
    if not TOKEN or not secrets.compare_digest(tok or "", TOKEN):
        raise HTTPException(401, "bad token")


def out_path(kind: str, ext: str = ".wav", name: Optional[str] = None) -> Path:
    d = DATA / "outputs" / kind
    d.mkdir(parents=True, exist_ok=True)
    stem = name or time.strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:4]
    return d / f"{stem}{ext}"


def _job(kind, title, fn):
    return queue.submit(kind, title, fn).public()


# --------------------------------------------------------------------- basic
@app.get("/health")
def health():
    return {"ok": True, "version": __version__, "data_dir": str(DATA)}


@app.get("/gpu", dependencies=[Depends(auth)])
def gpu_status():
    return gpu.status()


@app.post("/gpu/unload", dependencies=[Depends(auth)])
def gpu_unload():
    queue.submit("system", "Free GPU memory", lambda p: gpu.unload_all())
    return {"ok": True}


# --------------------------------------------------------------------- jobs
@app.get("/jobs", dependencies=[Depends(auth)])
def jobs():
    return queue.list()


@app.get("/jobs/{jid}", dependencies=[Depends(auth)])
def job(jid: str):
    j = queue.get(jid)
    if not j:
        raise HTTPException(404, "no such job")
    return j.public()


@app.post("/jobs/{jid}/cancel", dependencies=[Depends(auth)])
def cancel(jid: str):
    return {"ok": queue.cancel(jid)}


@app.post("/jobs/clear", dependencies=[Depends(auth)])
def clear():
    queue.clear_finished()
    return {"ok": True}


# --------------------------------------------------------------------- voices
class CloneReq(BaseModel):
    name: str
    path: str
    language: str = "ta"
    ref_text: Optional[str] = None


class VoicePatch(BaseModel):
    name: Optional[str] = None
    ref_text: Optional[str] = None


class PathReq(BaseModel):
    path: str


class PresetReq(BaseModel):
    keys: list[str] = []
    takes: int = 4


@app.get("/voices", dependencies=[Depends(auth)])
def voices():
    return store().list()


@app.get("/presets", dependencies=[Depends(auth)])
def presets():
    from .characters import PRESETS
    return {k: {kk: v[kk] for kk in ("label", "label_ta", "instruct", "line", "face")} for k, v in PRESETS.items()}


@app.post("/voices/clone", dependencies=[Depends(auth)])
def clone(r: CloneReq):
    if not Path(r.path).is_file():
        raise HTTPException(400, "audio file not found")

    def run(p):
        gpu.reserve("tts", "vc", "asr")
        return store().create(r.name, r.path, r.language, r.ref_text, progress=p).__dict__
    return _job("clone", f"Clone voice: {r.name}", run)


@app.post("/voices/presets", dependencies=[Depends(auth)])
def build_presets(r: PresetReq):
    from .characters import PRESETS, build_preset_voice
    keys = r.keys or list(PRESETS)
    bad = [k for k in keys if k not in PRESETS]
    if bad:
        raise HTTPException(400, f"unknown presets {bad}")

    def run(p):
        gpu.reserve("tts", "vc", "asr")
        out = []
        for i, k in enumerate(keys):
            info, score = build_preset_voice(store(), k, r.takes, sub_progress(p, i / len(keys), (i + 1) / len(keys)))
            out.append({**info.__dict__, "clarity": round(score, 3)})
        return out
    return _job("presets", f"Design {len(keys)} character voice(s)", run)


@app.patch("/voices/{vid}", dependencies=[Depends(auth)])
def patch_voice(vid: str, r: VoicePatch):
    return store().rename(vid, r.name, r.ref_text).__dict__


@app.delete("/voices/{vid}", dependencies=[Depends(auth)])
def delete_voice(vid: str):
    store().delete(vid)
    return {"ok": True}


@app.post("/voices/{vid}/export", dependencies=[Depends(auth)])
def export_voice(vid: str, r: PathReq):
    return {"path": str(store().export_to(vid, Path(r.path)))}


@app.post("/voices/import", dependencies=[Depends(auth)])
def import_voice(r: PathReq):
    try:
        return store().import_from(Path(r.path)).__dict__
    except (ValueError, OSError) as e:
        raise HTTPException(400, str(e))


@app.get("/voices/{vid}/sample", dependencies=[Depends(auth)])
def voice_sample(vid: str):
    """Write the reference clip to disk and return its path (for playback in the app)."""
    data = store().read_member(vid, "reference_tts.wav")
    p = out_path("samples", name=vid)
    p.write_bytes(data)
    return {"path": str(p)}


# --------------------------------------------------------------------- tts / convert
class TTSReq(BaseModel):
    text: str
    voice_id: Optional[str] = None
    language: str = "ta"
    mode: str = "auto"
    speed: Optional[float] = None
    out: Optional[str] = None


class ConvertReq(BaseModel):
    voice_id: str
    input: str
    steps: int = 30
    cfg_rate: float = 0.7
    out: Optional[str] = None


@app.post("/tts", dependencies=[Depends(auth)])
def tts(r: TTSReq):
    from .voices import speak
    from .audio_utils import save_wav

    def run(p):
        gpu.reserve("tts", "vc")
        wav, sr = speak(store(), r.voice_id, r.text, r.language, r.mode, r.speed, progress=p)
        dest = Path(r.out) if r.out else out_path("tts")
        save_wav(dest, wav, sr)
        return {"path": str(dest), "duration": round(len(wav) / sr, 2)}
    return _job("tts", "Text to speech: " + r.text[:40], run)


@app.post("/convert", dependencies=[Depends(auth)])
def convert(r: ConvertReq):
    from .voices import convert_file
    from .audio_utils import save_wav
    if not Path(r.input).is_file():
        raise HTTPException(400, "input file not found")

    def run(p):
        gpu.reserve("vc")
        wav, sr = convert_file(store(), r.voice_id, r.input, r.steps, r.cfg_rate, progress=p)
        dest = Path(r.out) if r.out else out_path("converted", name=Path(r.input).stem + "-converted")
        save_wav(dest, wav, sr)
        return {"path": str(dest), "duration": round(len(wav) / sr, 2)}
    return _job("convert", f"Convert {Path(r.input).name}", run)


# --------------------------------------------------------------------- images / talking heads
class ImageReq(BaseModel):
    prompt: str
    negative: str = ""
    width: int = 1344
    height: int = 768
    seed: Optional[int] = None
    out: Optional[str] = None
    style: str = "animated"
    kind: str = "scene"          # scene | portrait | thumbnail


class TalkReq(BaseModel):
    image: str
    audio: str
    out: Optional[str] = None
    expressive: float = 1.0


@app.post("/images", dependencies=[Depends(auth)])
def image(r: ImageReq):
    from . import images

    def run(p):
        gpu.reserve("image")
        dest = Path(r.out) if r.out else out_path("images", ".png")
        return images.generate(r.prompt, dest, r.width, r.height, r.seed, r.style, r.kind, progress=p)
    return _job("image", "Image: " + r.prompt[:40], run)


@app.post("/talking", dependencies=[Depends(auth)])
def talking(r: TalkReq):
    from . import talking as th

    def run(p):
        gpu.reserve("talking")
        dest = Path(r.out) if r.out else out_path("talking", ".mp4")
        return th.animate(r.image, r.audio, dest, r.expressive, progress=p)
    return _job("talking", f"Animate face: {Path(r.image).name}", run)


class TalkBatchReq(BaseModel):
    jobs: list[dict]


@app.post("/talking/batch", dependencies=[Depends(auth)])
def talking_batch(r: TalkBatchReq):
    from . import talking as th

    def run(p):
        gpu.reserve("talking")
        return {"paths": th.animate_many(r.jobs, progress=p)}
    return _job("talking", f"Animate {len(r.jobs)} talking face(s)", run)


@app.get("/capabilities", dependencies=[Depends(auth)])
def capabilities():
    from . import talking as th
    return {"talking_faces": th.available(), **gpu.status()}


# --------------------------------------------------------------------- live
@app.get("/live/info", dependencies=[Depends(auth)])
def live_info():
    return {"protocol": "send JSON config first, then binary float32 mono blocks of block_size samples"}


@app.websocket("/live/ws")
async def live_ws(ws: WebSocket):
    tok = ws.query_params.get("token", "")
    if not TOKEN or not secrets.compare_digest(tok, TOKEN):
        await ws.close(code=4401)
        return
    await ws.accept()
    from .live_vc import LiveConfig, LiveSession

    loop = asyncio.get_running_loop()
    try:
        cfg_msg = json.loads(await ws.receive_text())
        voice_id = cfg_msg.pop("voice_id")
        cfg = LiveConfig(**{k: v for k, v in cfg_msg.items() if k in LiveConfig.__dataclass_fields__})
        await ws.send_text(json.dumps({"status": "loading"}))
        sess = await loop.run_in_executor(None, LiveSession, store(), voice_id, cfg)
        await ws.send_text(json.dumps({"status": "ready", "block_size": sess.block_size,
                                       "sample_rate": cfg.sample_rate}))
        n = 0
        while True:
            data = await ws.receive_bytes()
            block = np.frombuffer(data, dtype=np.float32)
            out = await loop.run_in_executor(None, sess.process, block)
            await ws.send_bytes(out.astype(np.float32).tobytes())
            n += 1
            if n % 20 == 0:
                await ws.send_text(json.dumps({"infer_ms": round(sess.last_infer_ms, 1)}))
    except WebSocketDisconnect:
        pass
    except Exception as e:
        try:
            await ws.send_text(json.dumps({"status": "error", "error": f"{type(e).__name__}: {e}"}))
            await ws.close()
        except Exception:
            pass


# --------------------------------------------------------------------- main
def main():
    global TOKEN
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=0)
    ap.add_argument("--token", default=os.environ.get("VCSTUDIO_TOKEN", ""))
    a = ap.parse_args()
    TOKEN = a.token or secrets.token_urlsafe(24)
    import uvicorn

    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind(("127.0.0.1", a.port))
    port = sock.getsockname()[1]
    print(f"VCSTUDIO_READY port={port}", flush=True)
    config = uvicorn.Config(app, log_level="warning", ws_max_size=8 * 1024 * 1024)
    uvicorn.Server(config).run(sockets=[sock])


if __name__ == "__main__":
    sys.exit(main())

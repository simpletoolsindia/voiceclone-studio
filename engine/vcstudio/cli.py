"""Command-line interface (same engine the desktop app uses).

  python -m vcstudio.cli clone   --name "My voice" --audio voice.mp3 [--lang ta] [--text "transcript"]
  python -m vcstudio.cli list
  python -m vcstudio.cli speak   --voice ID --text "..." | --text-file story.txt  --out out.wav [--mode auto]
  python -m vcstudio.cli convert --voice ID --input story.wav --out out.wav [--steps 30]
  python -m vcstudio.cli export  --voice ID --out my_voice.vcpt
  python -m vcstudio.cli import  --file my_voice.vcpt
"""
from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

from .config import data_dir


def _progress(f, msg):
    print(f"\r[{f * 100:5.1f}%] {msg:50s}", end="", flush=True)


def main(argv=None):
    ap = argparse.ArgumentParser(prog="vcstudio")
    ap.add_argument("--data-dir", default=None)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("clone"); c.add_argument("--name", required=True); c.add_argument("--audio", required=True)
    c.add_argument("--lang", default="ta"); c.add_argument("--text", default=None)
    sub.add_parser("list")
    s = sub.add_parser("speak"); s.add_argument("--voice"); s.add_argument("--text"); s.add_argument("--text-file")
    s.add_argument("--out", required=True); s.add_argument("--lang", default="ta")
    s.add_argument("--mode", default="auto", choices=["auto", "direct", "stable"]); s.add_argument("--speed", type=float)
    v = sub.add_parser("convert"); v.add_argument("--voice", required=True); v.add_argument("--input", required=True)
    v.add_argument("--out", required=True); v.add_argument("--steps", type=int, default=30)
    v.add_argument("--cfg", type=float, default=0.7)
    e = sub.add_parser("export"); e.add_argument("--voice", required=True); e.add_argument("--out", required=True)
    i = sub.add_parser("import"); i.add_argument("--file", required=True)
    a = ap.parse_args(argv)

    from .voices import VoiceStore, speak, convert_file
    from .audio_utils import save_wav
    store = VoiceStore(Path(a.data_dir or data_dir()) / "voices")
    t0 = time.time()
    if a.cmd == "clone":
        info = store.create(a.name, a.audio, a.lang, a.text, progress=_progress)
        print(f"\nVoice id: {info.id}\nTranscript of reference: {info.ref_text}")
    elif a.cmd == "list":
        for v in store.list():
            print(f"{v['id']}  {v['name']}  [{v['language']}]")
    elif a.cmd == "speak":
        text = Path(a.text_file).read_text(encoding="utf-8") if a.text_file else a.text
        wav, sr = speak(store, a.voice, text, a.lang, a.mode, a.speed, progress=_progress)
        save_wav(a.out, wav, sr)
        print(f"\nSaved {a.out} ({len(wav) / sr:.1f}s)")
    elif a.cmd == "convert":
        wav, sr = convert_file(store, a.voice, a.input, a.steps, a.cfg, progress=_progress)
        save_wav(a.out, wav, sr)
        print(f"\nSaved {a.out} ({len(wav) / sr:.1f}s)")
    elif a.cmd == "export":
        print(store.export_to(a.voice, Path(a.out)))
    elif a.cmd == "import":
        info = store.import_from(Path(a.file))
        print(f"Imported voice id: {info.id} ({info.name})")
    print(f"took {time.time() - t0:.1f}s")


if __name__ == "__main__":
    sys.exit(main())

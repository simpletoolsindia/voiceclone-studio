# Architecture & developer guide

## Overview

```
┌──────────────── Tauri 2 app (Rust) ─────────────────┐         ┌──── Python GPU worker (engine/vcstudio) ────┐
│ ui/ (HTML/CSS/JS)        engine.rs  supervisor      │  HTTP   │ server.py  FastAPI on 127.0.0.1 + token     │
│ render.rs  story pipeline, steps, ETA, previews     │ ──────► │ jobs.py    one GPU job at a time, progress  │
│ video.rs   ffmpeg compositor (NVENC), shot grammar  │ ◄────── │ gpu.py     unloads models between stages    │
│ live.rs    mic/speaker via cpal ── WebSocket ──────►│         │ tts_engine OmniVoice   seedvc_engine Seed-VC│
│ story.rs   projects, script parser, AI writer       │         │ images     Z-Image     animate LTX + Latent │
│ minimax.rs optional H3 API   settings.rs  paths.rs  │         │ talking    SadTalker   fx  cutouts, emoji   │
└─────────────────────────────────────────────────────┘         └─────────────────────────────────────────────┘
```

**Why Rust and Python?** The neural models only exist as PyTorch code, and their time is spent in CUDA kernels, which run at the same speed whatever language calls them. Rust owns everything where the host language matters:

- real-time audio threads
- orchestration, progress and caching
- video encoding through ffmpeg
- process supervision and the UI shell

SadTalker and LatentSync need old, pinned dependencies, so each lives in its own virtual environment (`.venv-talk`, `.venv-sync`) and runs as a batch subprocess (`scripts/talk_runner.py`, `scripts/sync_runner.py`).

## Story render pipeline (`render.rs`)

Each render builds a **plan of steps** with estimated durations from what actually needs doing. Cached assets cost nothing. Steps report their own progress, the overall bar is weighted by the estimates, and the ETA re-calibrates from how fast finished steps really ran.

| Step | What happens | Engine |
|---|---|---|
| voices | Make sure every character has a voice (builds missing presets) | OmniVoice voice design + Tamil ASR scoring |
| speech | One WAV per dialogue line (Auto mode: direct clone, validated, falls back to neutral TTS → Seed-VC) | OmniVoice / Seed-VC |
| faces | Front-facing portrait per character | Z-Image-Turbo (FP8, CPU offload) |
| scenes | Establishing image per scene, plus an **empty set** (people removed from the prompt) for dialogue shots | Z-Image-Turbo |
| thumbnail | Thumbnail background, with the Tamil title added by ffmpeg/libass | Z-Image-Turbo |
| cutouts | Alpha mask per portrait | BiRefNet-portrait (rembg, ONNX) |
| setup | **Keyframe** per dialogue line: empty set + speaker (+ listener) cutouts, two-shot or close-up; insert images | PIL |
| animate | Image-to-video per keyframe / scene / insert (≤ 4 s, 8 steps) | LTX-Video 2B distilled 0.9.8 |
| lipsync | Mouth re-drawn to the Tamil line; the clip ping-pongs if the line is longer | LatentSync 1.5 |
| fx | Emoji stickers, procedural SFX (whoosh, ding, pop, tick, rain, night) | Pillow, ffmpeg |
| compose | One segment per shot: captions (karaoke, complex Tamil shaping), emoji, SFX, ambience; 3 in parallel | ffmpeg + NVENC |
| finalize | Concat, music with sidechain ducking, loudnorm −14 LUFS, chapters description | ffmpeg |

Fast mode replaces setup / animate / lipsync with SadTalker talking faces composited as cutouts.

### Shot grammar (`video.rs::plan`)

1. **Cold open**: the most dramatic line.
2. **Title card**: with a ding.
3. Per scene:
   - a **scene card** (whoosh, ambience starts)
   - lines alternate **two-shot → close-ups (shot/reverse-shot)**, with a two-shot again every 4th line
   - narration shows the animated establishing shot
   - inserts are animated close-ups
   - questions are countdown cards
4. **End card**: a comment question, a pulsing SUBSCRIBE button and an AI disclosure.

## Fixes after rendering (`edit.rs`)

Each render saves a **timeline**: every shot's time range plus its scene and line. `edit::interpret(project, time, request)` maps a plain-language request (English or Tamil keywords, `m:ss` inside the text, character names or roles such as *Appa* or *Paati*) to a `FixPlan`:

- the action: lipsync, voice, text, emotion, animate, look, scene or delete
- the editable value
- the affected shots and a time estimate

`edit::apply` changes the story and clears only the affected assets: a new `take` reseeds voice and animation, `sync_boost` strengthens lip-sync, and a look change clears that character's face and shots. The next render regenerates only those, and each render is saved as a new version.

## Cache

Content-addressed and shared by all stories, in `%APPDATA%\VoiceCloneStudio\cache\<kind>`:

| Kind | Key |
|---|---|
| `tts` | voice + text + take + speed + language |
| `images` | prompt + kind + style + size + seed |
| `clips` | keyframe pixels + prompt + seed + length |
| `sync` | clip bytes + audio bytes + steps + guidance |
| `segments` | every input of a composed shot (file signatures, text, effects) |
| `t5` | prompt embeddings, so the text encoder isn't loaded when all prompts are known |
| `intro` | normalised channel intro/outro |

## Data layout (`%APPDATA%\VoiceCloneStudio`)

```
settings.json                 app settings (API keys stay local)
voices/<id>.vcpt              voice checkpoints
projects/<id>/project.json    story (characters, scenes, lines, asset paths)
projects/<id>/audio|faces|scenes|keyframes|shots|talk|segments/
projects/<id>/<title>.mp4, thumbnail.jpg, description.txt
assets/sfx, assets/emoji      shared generated assets
outputs/                      TTS / conversion / image results from the other pages
engine/                       bundled engine (installed builds)
```

### Voice checkpoint (`.vcpt`)

A zip archive:

- `manifest.json`: name, language, transcript, kind, preset, format version
- `reference_vc.wav`: about 8 s, the Seed-VC reference
- `reference_tts.wav`: up to 20 s, the OmniVoice prompt
- optional caches: `tts_prompt.pt`, `vc_quality.pt`, `vc_realtime.pt`

Only the WAVs are required; the caches are rebuilt on demand. Imports reject nested paths and archives from newer versions.

## Engine API (selection)

All routes require the `X-Token` header (the app generates it per launch). Long tasks return a job; poll `GET /jobs/{id}` for `progress`, `stage`, `eta_sec` and `preview`.

| Route | Purpose |
|---|---|
| `GET /voices`, `POST /voices/clone`, `/voices/presets`, `/voices/import`, `/voices/{id}/export` | Voices and checkpoints |
| `POST /tts`, `POST /convert` | Speech, conversion |
| `POST /images`, `/matte`, `/emoji` | Images, cutouts, stickers |
| `POST /animate/keyframe`, `/animate/shots`, `/lipsync`, `/talking/batch` | Animation |
| `WS /live/ws?token=` | Live conversion: JSON config, then float32 blocks both ways |
| `GET /jobs`, `POST /jobs/{id}/cancel`, `GET /gpu`, `GET /capabilities` | Jobs and status |

## Building from source

Prerequisites:

- Rust (stable, MSVC toolchain)
- Node.js 20+
- Git and `uv`
- ffmpeg on PATH
- Visual Studio Build Tools (C++), needed for insightface

```powershell
git clone https://github.com/simpletoolsindia/voiceclone-studio.git
cd voiceclone-studio
powershell -ExecutionPolicy Bypass -File engine\scripts\setup_engine.ps1 -TalkingFaces -Cinematic
cd app
npm install
npm run dev        # development (uses ..\engine automatically)
npm run build      # installers: app\src-tauri\target\release\bundle\nsis\*.exe and msi\*.msi
```

### Headless CLI

```powershell
voiceclone-studio --import examples\paatiyin_manthira_petti.txt   # -> project id
voiceclone-studio --render <project-id>                            # render with progress output
voiceclone-studio --write "story idea" ollama                      # AI-written story
voiceclone-studio --list
cd engine; .venv\Scripts\python -m vcstudio.cli clone --name "Me" --audio me.mp3 --lang ta
.venv\Scripts\python -m vcstudio.cli convert --voice <id> --input story.wav --out story_in_my_voice.wav
```

### Notes and gotchas

- Never `uv pip install -U` inside `engine\.venv`, because it can replace the CUDA PyTorch build. Install with `-c constraints.txt`.
- Tamil subtitles need libass **complex shaping** (`ass=...:shaping=complex`) and zero letter spacing.
- Z-Image-Turbo must use FP8 layerwise casting on 12 GB cards. Without it, the model spills into shared memory and runs more than 10× slower.
- Generic Whisper hallucinates on colloquial Tamil, so reference transcripts use `vasista22/whisper-tamil-medium`.
- OmniVoice's `child` voice tag lands at 430–630 Hz, which sounds cartoonish. The child presets use settings measured at 250–340 Hz.

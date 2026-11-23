# VoiceClone Studio

**Free, open-source desktop studio for voice cloning and Tamil story videos — runs locally on your GPU.**

Built with **Rust + Tauri 2** (app, real-time audio, video compositing, job orchestration) and a local
**Python GPU worker** (the neural models). Inspired by [debpalash/VoiceStudio](https://github.com/debpalash/VoiceStudio).

| | |
|---|---|
| 🗣️ **Text to speech** | Type Tamil (or 600+ languages) and hear it in any cloned voice — OmniVoice |
| 🧬 **Voice cloning → checkpoint** | Clone from any MP3/WAV into a portable `.vcpt` checkpoint |
| ⬇⬆ **Download / upload checkpoints** | Share voices between machines and people |
| 🔁 **Convert audio** | Re-voice a recording (story, podcast) in a cloned voice, keeping timing and emotion — Seed-VC |
| 🎧 **Live voice** | Microphone → cloned voice in real time (~0.4–0.9 s latency), Rust/cpal audio |
| 🎬 **Story Studio** | Write a Tamil story with AI (local Ollama or Claude), or paste a script; characters get AI faces + voices; renders a YouTube / Shorts video with talking faces, scene art, subtitles, music and a thumbnail |
| 👨‍👩‍👧 **9 Tamil character voices** | man, girl, boy, aunty, uncle, grandpa, grandma, child boy, child girl — designed, clarity- and pitch-checked, replaceable |
| ⏳ **Background jobs** | Every long task shows live %, stage and ETA; keeps running when you close the window (system tray) |

## Quick start (Windows + NVIDIA GPU)

Requirements: Windows 10/11, NVIDIA GPU with ≥ 8 GB VRAM (12 GB recommended), ~60 GB free disk,
[Rust](https://rustup.rs), [Node.js](https://nodejs.org), Git. Optional: [Ollama](https://ollama.com) for offline story writing.

```powershell
git clone <this repo> voiceclone
cd voiceclone
# 1. AI engine (Python envs, PyTorch CUDA, models fetch on first use)
powershell -ExecutionPolicy Bypass -File engine\scripts\setup_engine.ps1 -TalkingFaces
# 2. Desktop app
cd app
npm install
npm run dev          # development
npm run build        # installer in app\src-tauri\target\release\bundle
```

The installed app can also install the engine itself: **Settings → Install / repair engine**.

### Headless / batch

```powershell
voiceclone-studio --import examples\paatiyin_manthira_petti.txt   # prints a project id
voiceclone-studio --render <project-id>                            # renders the video with progress
voiceclone-studio --write "story idea" ollama                      # AI-written story
cd engine; .venv\Scripts\python -m vcstudio.cli clone --name "Me" --audio me.mp3 --lang ta
.venv\Scripts\python -m vcstudio.cli convert --voice <id> --input story.wav --out story_in_my_voice.wav
```

## Story script format

```text
# Title
> One-line summary
@பாட்டி = grandma | kind elderly woman, silver hair, cotton saree
@கவின் = child_boy
[Scene: Rainy evening | cozy living room, warm lamp light, rain on the window]
Narrator: சென்னையில ஒரு மழை நாள் சாயங்காலம்...
பாட்டி (warm): கவின்! கொஞ்சம் இங்க வா.
கவின் (excited): வரேன் பாட்டி!
```

Unknown names become characters automatically (பாட்டி → grandma, தாத்தா → grandpa, அம்மா → aunty, …).
See [`examples/`](examples).

## Video style

Built from what retains viewers on story channels (animated captions, a visual change at least every
~4 s, sparse sound effects, a hook in the first 3 s) and the shift in Tamil AI shorts from plain
"talking-head" photos to cinematic, in-scene storytelling:

* **Cold open** – the most dramatic line plays first, then the title card.
* **In-scene characters** – each portrait is cut out (BiRefNet) and placed in an empty version of the
  location: a *two-shot* with the speaker lip-synced and the listener reacting, then *shot / reverse-shot
  close-ups*. Narration shows the full painted scene.
* **Motion everywhere** – Ken Burns camera moves, a punch-in on long lines, breathing listeners.
* **Karaoke captions** – Tamil subtitles highlight word by word (libass with complex shaping).
* **Emoji reactions** – strong emotions pop a sticker next to the speaker (😮 😂 ❤️ 🤩 …).
* **Sound design** – whoosh transitions, a title ding, rain / night ambience from the scene description
  (procedural, royalty-free), music ducked under voices, −14 LUFS.
* **End card** – a comment question, a pulsing SUBSCRIBE button, and an AI-content disclosure.
* **YouTube description** – chapters (from scene timings), characters and hashtags, ready to paste.

## How it works

```
┌──────────── Tauri app (Rust) ─────────────┐        ┌──── Python GPU worker (127.0.0.1, token) ────┐
│ UI (HTML/JS)                               │  HTTP  │ OmniVoice   – Tamil TTS + zero-shot cloning   │
│ engine supervisor · job tracker            │ ─────► │ Seed-VC     – voice conversion (file + live)  │
│ live audio: cpal ring buffers ◄─ WebSocket │ ◄───── │ Z-Image-Turbo – faces, scenes, thumbnails     │
│ story render orchestrator                  │        │ SadTalker   – talking faces (own venv)        │
│ ffmpeg compositor (NVENC), MiniMax client  │        │ Tamil Whisper – reference transcripts         │
└────────────────────────────────────────────┘        └───────────────────────────────────────────────┘
```

Why not 100 % Rust? The neural models only exist as PyTorch code. Their time is spent in CUDA kernels,
which run at the same speed whatever language calls them, so Rust owns everything where it
does matter: real-time audio threads, orchestration, video encoding and the UI shell.

GPU jobs run one at a time and each stage unloads the previous models, so everything fits in 12 GB.
On an RTX 4070 SUPER: voice conversion ≈ 5× faster than real time, an image ≈ 17 s, a talking-face line ≈ 1 min.

### Voice checkpoint (`.vcpt`)

A zip with `manifest.json`, `reference_vc.wav` (~8 s, Seed-VC), `reference_tts.wav` (≤ 20 s, OmniVoice),
and cached embeddings (`tts_prompt.pt`, `vc_quality.pt`, `vc_realtime.pt`). Only the WAVs are required; caches are rebuilt on demand.

### Tamil quality notes

* Generic Whisper hallucinates on colloquial Tamil, so reference transcripts use the Tamil fine-tuned
  `vasista22/whisper-tamil-medium`. You can correct a transcript in **Voices → Edit** for a better clone.
* TTS **Auto** mode clones directly (keeps the speaker's style) and checks each sentence; a bad take
  falls back to neutral TTS → Seed-VC, which is always intelligible.
* Character voices are generated from known sentences, scored for clarity with Tamil ASR and for pitch range by age/gender.

### Optional cloud features

* **Claude** story writing (best modern Tamil): add an Anthropic API key in Settings.
* **MiniMax H3** cinematic scene clips: add a MiniMax API key (≈ $0.04–0.08 per video second). H3 is open-weight
  but needs ~4 data-centre GPUs, so it is offered via API only. Voices always stay local.

## Licenses & responsible use

Code: **GPL-3.0-or-later** (Seed-VC is GPL-3.0). Models: OmniVoice (Apache-2.0), Seed-VC (GPL-3.0),
Z-Image-Turbo (Apache-2.0), SadTalker (Apache-2.0), whisper-tamil-medium (Apache-2.0). Models are downloaded
from their official sources, not redistributed.

Only clone voices you own or have permission to use, and disclose AI-generated content when publishing
(YouTube requires labelling realistic synthetic media).

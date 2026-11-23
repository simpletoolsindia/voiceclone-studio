# Changelog

## 0.1.1 — 2026-11-23

- **Pause / Resume** for renders and engine jobs, in the progress panel, the jobs box and the Jobs page. GPU work stops at a safe point, helper processes are suspended, and time-left skips paused time.
- **Fix after rendering**, **smart cache**, **downloads** (srt, mp3, Shorts teaser), **channel intro/outro** and **version history** (see 0.1.0 notes).
- Fixed: the app crashed when clicking Render video or Live Start.

## 0.1.0 — 2026-11-23

First public release.

- **Story Studio:**
  - AI story writing (Ollama offline or Claude) built on screenplay rules
  - plain-text scripts with action beats, insert shots and viewer questions
  - characters with AI faces and voices
- **Cinematic animation:** every shot is animated (LTX-Video 2B) and lip-synced to Tamil (LatentSync); a ⚡ Fast mode uses talking faces.
- **Video polish:**
  - characters placed inside scenes, with two-shots and close-ups
  - karaoke Tamil subtitles, emoji reactions, procedural sound effects and ambience
  - cold-open hook, end card with a subscribe button, music ducking, −14 LUFS
  - thumbnail, and a YouTube description with chapters
- **Voices:**
  - cloning into portable `.vcpt` checkpoints (download/upload)
  - nine designed Tamil character voices with natural child pitch
  - text to speech, file conversion, live microphone conversion
- **Progress:** a detailed panel with a step checklist, live preview, ETA, GPU memory and taskbar progress; jobs keep running in the system tray.
- **Fix after rendering:**
  - plain-language requests ("at 1:22 change Appa's shirt to blue", "fix the lip-sync", "say it slower")
  - a clickable shot timeline with quick fixes
  - only affected shots are regenerated; every render is kept as a version
- **Smart cache:** voices, images, animated shots, lip-syncs, composed shots and prompt embeddings are reused across stories; channel intro/outro support.
- **Downloads:** MP4, thumbnail, Tamil `.srt` subtitles, MP3 and a 9:16 Shorts teaser.
- **Installer:** one Windows installer with a first-run one-click AI engine setup, or an existing engine folder.

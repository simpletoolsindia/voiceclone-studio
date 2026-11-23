# Installing VoiceClone Studio

## 1. Check your PC

| | Minimum | Recommended |
|---|---|---|
| OS | Windows 10 64-bit | Windows 11 |
| GPU | NVIDIA, 8 GB VRAM | NVIDIA RTX 30/40/50, 12 GB+ VRAM |
| RAM | 16 GB | 32 GB |
| Disk | 60 GB free | 100 GB free on an SSD |
| Internet | Needed once, for setup | |

Update your **NVIDIA driver** first: [nvidia.com/drivers](https://www.nvidia.com/drivers). The app works without an NVIDIA GPU, but video rendering would take many hours.

## 2. Install the app

1. Download **`VoiceClone Studio_x.y.z_x64-setup.exe`** from the [latest release](../../../releases/latest).
2. Double-click it. If Windows SmartScreen appears, click **More info → Run anyway** (the installer isn't code-signed yet).
3. Finish the wizard. You'll get Start menu and desktop shortcuts called **VoiceClone Studio**.

## 3. First launch: set up the AI engine (one time)

When the app opens it shows **"Welcome to VoiceClone Studio — one-time setup"**.

1. Keep both options ticked:
   - **Talking faces (fast mode)** adds about 4 GB.
   - **Cinematic animation + lip-sync** adds about 10 GB.
2. Click **⬇ Set up AI engine**.
3. Watch the progress bar and log. The setup installs:
   - `uv`, `ffmpeg` and Python (if missing)
   - PyTorch with CUDA for your GPU
   - the engine's Python packages
   - Seed-VC, SadTalker and LatentSync, in their own isolated environments
4. When it says **✓ Setup complete**, the engine starts and the status pill turns green: **AI engine ready**.

AI models (OmniVoice, Z-Image-Turbo, LTX-Video and others) download automatically **the first time each feature is used**, so your first render takes longer. The total is about 25–60 GB.

The cinematic lip-sync environment compiles one package (insightface). If setup fails at that step, install **[Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)** with the *Desktop development with C++* workload, then click **Set up AI engine** again.

### Already have the engine (for example a source checkout)?

Click **"I already have an engine folder…"**, or go to **Settings → AI engine → Engine folder**. Choose the folder that contains `vcstudio` and `.venv`, then restart the app. Nothing is downloaded again.

## 4. Optional extras

- **Offline story writing:** install [Ollama](https://ollama.com), then run `ollama pull gemma4` (or `qwen3.5:9b`). Pick it in **Settings → Story writer**.
- **Best Tamil writing:** paste an Anthropic API key in **Settings → Story writer** and choose **Claude** when writing.
- **MiniMax H3 cinematic scenes (paid cloud):** paste a MiniMax API key in Settings and turn it on per story.

## Where things are stored

| What | Where |
|---|---|
| App | `C:\Program Files\VoiceClone Studio` (or your chosen folder) |
| Engine, Python environments | `%APPDATA%\VoiceCloneStudio\engine` |
| AI model cache | `%USERPROFILE%\.cache\huggingface` |
| Voices (`.vcpt`), stories, videos | `%APPDATA%\VoiceCloneStudio` (**Settings → Open data folder**) |

## Troubleshooting

| Problem | Fix |
|---|---|
| "Engine not installed" | Run the one-time setup (step 3), or point Settings at an existing engine folder. |
| Setup stops at PyTorch or packages | Check your internet and free disk space, then click **Set up AI engine** again. It resumes where it stopped. |
| `insightface` build error | Install Visual Studio Build Tools (C++), then run the setup again. |
| Render is very slow / PC freezes | Close other GPU apps (games, browsers with many video tabs). Use **⚡ Fast** animation mode. 32 GB RAM is recommended. |
| "CUDA available: False" in the log | Update the NVIDIA driver, then run the setup again. |
| Tamil text looks broken | Windows includes the needed font (Nirmala UI). On other systems install *Noto Sans Tamil*. |
| Live voice echoes | Use headphones. |
| Anything else | **Settings → AI engine** shows the engine log. Copy it into a GitHub issue. |

## Uninstall

Use **Windows Settings → Apps → VoiceClone Studio → Uninstall**. To also remove downloaded models and your data, delete `%APPDATA%\VoiceCloneStudio` and `%USERPROFILE%\.cache\huggingface`. Back up your `.vcpt` voices and videos first.

## Build from source

See [ARCHITECTURE.md → Building](ARCHITECTURE.md#building-from-source).

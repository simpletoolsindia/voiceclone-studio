<#
  VoiceClone Studio - AI engine installer (Windows, NVIDIA GPU recommended).

  Creates engine\.venv (Python 3.11) with CUDA PyTorch + OmniVoice + Seed-VC +
  Z-Image deps, fetches Seed-VC, and optionally SadTalker (talking faces) in
  its own engine\.venv-talk (Python 3.10). Models download on first use from
  Hugging Face / GitHub releases.

  Usage:  powershell -ExecutionPolicy Bypass -File setup_engine.ps1 [-TalkingFaces] [-Cpu]
#>
param([switch]$TalkingFaces, [switch]$Cinematic, [switch]$Cpu)
$ErrorActionPreference = "Stop"
$Engine = Split-Path -Parent $PSScriptRoot
Set-Location $Engine
function Step($m) { Write-Output "==> $m" }

# Pinned upstream revisions (tested)
$SeedVcCommit = "51383efd921027683c89e5348211d93ff12ac2a8"
$SadTalkerCommit = "cd4c0465ae0b54a6f85af57f5c65fec9fe23e7f8"

Step "Checking tools"
if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
  Step "Installing uv (Python package manager)"
  powershell -ExecutionPolicy Bypass -c "irm https://astral.sh/uv/install.ps1 | iex"
  $env:Path = "$env:USERPROFILE\.local\bin;$env:Path"
}
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { throw "git is required: winget install Git.Git" }
if (-not (Get-Command ffmpeg -ErrorAction SilentlyContinue)) {
  Step "Installing ffmpeg"
  winget install --id Gyan.FFmpeg -e --accept-source-agreements --accept-package-agreements | Out-Null
}

$TorchIndex = if ($Cpu) { "https://download.pytorch.org/whl/cpu" } else { "https://download.pytorch.org/whl/cu128" }

Step "Creating Python 3.11 environment"
if (-not (Test-Path ".venv\Scripts\python.exe")) { uv venv --python 3.11 .venv }
$Py = Join-Path $Engine ".venv\Scripts\python.exe"

Step "Installing PyTorch ($(if ($Cpu) {'CPU'} else {'CUDA 12.8'}))"
uv pip install --python $Py torch==2.7.1 torchaudio==2.7.1 torchvision==0.22.1 --index-url $TorchIndex
if ($LASTEXITCODE -ne 0) { throw "PyTorch install failed" }

Step "Installing engine packages"
uv pip install --python $Py -r requirements.txt -c constraints.txt
if ($LASTEXITCODE -ne 0) { throw "package install failed" }

Step "Fetching Seed-VC (voice conversion)"
New-Item -ItemType Directory -Force third_party | Out-Null
if (-not (Test-Path "third_party\seed-vc\.git")) {
  git clone https://github.com/Plachtaa/seed-vc.git third_party/seed-vc
}
git -C third_party/seed-vc fetch --depth 1 origin $SeedVcCommit 2>$null
git -C third_party/seed-vc checkout -q $SeedVcCommit 2>$null
& $Py scripts\patch_seedvc.py third_party\seed-vc

if ($TalkingFaces) {
  Step "Fetching SadTalker (talking faces)"
  if (-not (Test-Path "third_party\SadTalker\.git")) {
    git clone https://github.com/OpenTalker/SadTalker.git third_party/SadTalker
  }
  git -C third_party/SadTalker checkout -q $SadTalkerCommit 2>$null
  Step "Creating talking-face environment (Python 3.10)"
  if (-not (Test-Path ".venv-talk\Scripts\python.exe")) { uv venv --python 3.10 .venv-talk }
  $Pt = Join-Path $Engine ".venv-talk\Scripts\python.exe"
  $TalkIndex = if ($Cpu) { "https://download.pytorch.org/whl/cpu" } else { "https://download.pytorch.org/whl/cu121" }
  uv pip install --python $Pt torch==2.1.2 torchvision==0.16.2 torchaudio==2.1.2 --index-url $TalkIndex
  uv pip install --python $Pt "numpy==1.23.5" "setuptools<70" wheel cython
  uv pip install --python $Pt --no-build-isolation basicsr==1.4.2
  uv pip install --python $Pt "numpy==1.23.5" face_alignment==1.3.5 imageio==2.19.3 imageio-ffmpeg==0.4.7 librosa==0.9.2 numba resampy==0.3.1 pydub==0.25.1 scipy==1.10.1 kornia==0.6.8 tqdm yacs==0.1.8 pyyaml joblib==1.1.0 scikit-image==0.19.3 facexlib==0.3.0 gfpgan av safetensors pillow
  if ($LASTEXITCODE -ne 0) { throw "talking-face packages failed" }

  Step "Downloading talking-face checkpoints"
  $ck = "third_party\SadTalker\checkpoints"; $gw = "third_party\SadTalker\gfpgan\weights"
  New-Item -ItemType Directory -Force $ck, $gw | Out-Null
  $files = @{
    "$ck\mapping_00109-model.pth.tar" = "https://github.com/OpenTalker/SadTalker/releases/download/v0.0.2-rc/mapping_00109-model.pth.tar"
    "$ck\mapping_00229-model.pth.tar" = "https://github.com/OpenTalker/SadTalker/releases/download/v0.0.2-rc/mapping_00229-model.pth.tar"
    "$ck\SadTalker_V0.0.2_256.safetensors" = "https://github.com/OpenTalker/SadTalker/releases/download/v0.0.2-rc/SadTalker_V0.0.2_256.safetensors"
    "$ck\BFM_Fitting.zip" = "https://github.com/OpenTalker/SadTalker/releases/download/v0.0.2/BFM_Fitting.zip"
    "$ck\hub.zip" = "https://github.com/OpenTalker/SadTalker/releases/download/v0.0.2/hub.zip"
    "$gw\alignment_WFLW_4HG.pth" = "https://github.com/xinntao/facexlib/releases/download/v0.1.0/alignment_WFLW_4HG.pth"
    "$gw\detection_Resnet50_Final.pth" = "https://github.com/xinntao/facexlib/releases/download/v0.1.0/detection_Resnet50_Final.pth"
    "$gw\GFPGANv1.4.pth" = "https://github.com/TencentARC/GFPGAN/releases/download/v1.3.0/GFPGANv1.4.pth"
    "$gw\parsing_parsenet.pth" = "https://github.com/xinntao/facexlib/releases/download/v0.2.2/parsing_parsenet.pth"
  }
  foreach ($k in $files.Keys) {
    if (-not (Test-Path $k) -or (Get-Item $k).Length -eq 0) {
      Write-Output "    downloading $(Split-Path -Leaf $k)"
      curl.exe -sSL --fail -o $k $files[$k]
      if ($LASTEXITCODE -ne 0) { throw "download failed: $($files[$k])" }
    }
  }
  foreach ($z in @("BFM_Fitting", "hub")) {
    if (-not (Test-Path "$ck\$z")) { Expand-Archive -Force "$ck\$z.zip" $ck }
  }
}

if ($Cinematic) {
  Step "Fetching LatentSync (lip-sync for cinematic shots)"
  if (-not (Test-Path "third_party\LatentSync\.git")) {
    git clone https://github.com/bytedance/LatentSync.git third_party/LatentSync
  }
  git -C third_party/LatentSync checkout -q a229c3948406bc2cf6eaf4873e662e70c6a04746 2>$null
  Step "Creating lip-sync environment (Python 3.10; needs Visual Studio C++ Build Tools for insightface)"
  if (-not (Test-Path ".venv-sync\Scripts\python.exe")) { uv venv --python 3.10 .venv-sync }
  $Ps = Join-Path $Engine ".venv-sync\Scripts\python.exe"
  uv pip install --python $Ps torch==2.5.1 torchvision==0.20.1 torchaudio==2.5.1 --index-url https://download.pytorch.org/whl/cu121
  uv pip install --python $Ps cython "numpy==1.26.4" setuptools wheel
  Get-Content third_party\LatentSync\requirements.txt | Where-Object { $_ -notmatch '^(gradio|--extra)' } | Set-Content -Encoding utf8 "$env:TEMP\vcs_sync_reqs.txt"
  uv pip install --python $Ps -r "$env:TEMP\vcs_sync_reqs.txt"
  if ($LASTEXITCODE -ne 0) { throw "lip-sync packages failed" }
  & $Py -c "from huggingface_hub import hf_hub_download as d; [d('ByteDance/LatentSync-1.5', f, local_dir=r'third_party\LatentSync\checkpoints') for f in ('latentsync_unet.pt','whisper/tiny.pt')]"
  & $Py -c "from huggingface_hub import snapshot_download as s; s('google-t5/t5-base', allow_patterns=['*.json','*.model'])"
}

Step "Checking GPU"
& $Py -c "import torch; print('CUDA available:', torch.cuda.is_available(), torch.cuda.get_device_name(0) if torch.cuda.is_available() else '')"
Write-Output "Engine installed. AI models (~35 GB) download automatically the first time each feature is used."

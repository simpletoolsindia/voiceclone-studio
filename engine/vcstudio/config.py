import os
import sys
from pathlib import Path


def data_dir() -> Path:
    """Per-user data folder (voices, outputs). Override with VCSTUDIO_DATA_DIR."""
    env = os.environ.get("VCSTUDIO_DATA_DIR")
    if env:
        p = Path(env)
    elif sys.platform == "win32":
        p = Path(os.environ.get("APPDATA", Path.home())) / "VoiceCloneStudio"
    elif sys.platform == "darwin":
        p = Path.home() / "Library" / "Application Support" / "VoiceCloneStudio"
    else:
        p = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share")) / "VoiceCloneStudio"
    p.mkdir(parents=True, exist_ok=True)
    return p

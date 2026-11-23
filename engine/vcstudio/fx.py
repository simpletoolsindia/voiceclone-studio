"""Visual effect assets: character cutout masks and emoji reaction stickers."""
from __future__ import annotations

import sys
import threading
from pathlib import Path
from typing import Callable, Optional

_session = None
_lock = threading.Lock()


def matte(image: str, out: Path, progress: Optional[Callable[[float, str], None]] = None) -> dict:
    """Save a soft alpha mask (grayscale PNG) of the person in `image`.

    BiRefNet-portrait via rembg (MIT; ONNX weights only). Talking faces are
    rendered in SadTalker's still mode, which animates the face inside a fixed
    silhouette, so one mask per portrait fits every frame of every line."""
    from PIL import Image, ImageFilter
    from rembg import new_session, remove

    global _session
    p = progress or (lambda f, m="": None)
    p(0.05, "Cutting out character")
    with _lock:
        if _session is None:
            _session = new_session("birefnet-portrait")
        im = Image.open(image).convert("RGB")
        mask = remove(im, session=_session, only_mask=True)
    # tiny feather so edges blend into any background
    mask = mask.convert("L").filter(ImageFilter.GaussianBlur(1.2))
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    mask.save(out)
    p(1.0, "Cutout ready")
    return {"path": str(out)}


EMOJI_FONT = {"win32": r"C:\Windows\Fonts\seguiemj.ttf",
              "darwin": "/System/Library/Fonts/Apple Color Emoji.ttc"}.get(sys.platform,
                                                                       "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf")


def emoji(char: str, out: Path, size: int = 220) -> dict:
    """Render a colour emoji sticker with a soft white outline and shadow."""
    from PIL import Image, ImageDraw, ImageFilter, ImageFont

    out = Path(out)
    if out.exists():
        return {"path": str(out)}
    font = ImageFont.truetype(EMOJI_FONT, 109)          # bitmap/COLR emoji fonts render at 109 px
    canvas = Image.new("RGBA", (180, 180), (0, 0, 0, 0))
    ImageDraw.Draw(canvas).text((90, 90), char, font=font, embedded_color=True, anchor="mm")
    alpha = canvas.split()[-1]
    outline = alpha.filter(ImageFilter.MaxFilter(9)).filter(ImageFilter.GaussianBlur(1.5))
    shadow = alpha.filter(ImageFilter.MaxFilter(9)).filter(ImageFilter.GaussianBlur(6))
    img = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    img.paste((0, 0, 0, 110), (4, 6), shadow)
    img.paste((255, 255, 255, 255), (0, 0), outline)
    img.alpha_composite(canvas)
    img = img.resize((size, size), Image.LANCZOS)
    out.parent.mkdir(parents=True, exist_ok=True)
    img.save(out)
    return {"path": str(out)}

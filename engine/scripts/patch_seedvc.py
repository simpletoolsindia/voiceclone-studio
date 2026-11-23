"""Small compatibility patches for upstream Seed-VC with current huggingface_hub
(>=1.0 dropped the `proxies` / `resume_download` arguments). Idempotent."""
import re
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parents[1] / "third_party" / "seed-vc")
p = root / "modules" / "bigvgan" / "bigvgan.py"
s = p.read_text(encoding="utf-8")
s2 = s.replace("proxies: Optional[Dict],", "proxies: Optional[Dict] = None,")
s2 = s2.replace("resume_download: bool,", "resume_download: bool = False,")
s2 = re.sub(r"\n\s*proxies=proxies,", "", s2)
s2 = re.sub(r"\n\s*resume_download=resume_download,", "", s2)
if s2 != s:
    p.write_text(s2, encoding="utf-8")
    print("patched", p)
else:
    print("already patched", p)

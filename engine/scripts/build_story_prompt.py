"""Generate app/ui/story_prompt.js from docs/STORY_PROMPT.md (single source of truth)."""
import json
import re
from pathlib import Path

root = Path(__file__).resolve().parents[2]
md = (root / "docs" / "STORY_PROMPT.md").read_text(encoding="utf-8")
m = re.search(r"````text\n(.*?)\n````", md, re.S)
assert m, "prompt block not found"
(root / "app" / "ui" / "story_prompt.js").write_text(
    "// Generated from docs/STORY_PROMPT.md by engine/scripts/build_story_prompt.py - edit the .md, not this file.\n"
    f"const STORY_PROMPT_TEMPLATE = {json.dumps(m.group(1), ensure_ascii=False)};\n", encoding="utf-8")
print("story_prompt.js written,", len(m.group(1)), "chars")

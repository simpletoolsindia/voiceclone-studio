"""Built-in Tamil character presets.

Each preset has an OmniVoice voice-design instruction, a modern colloquial
Tamil sample line (used as the cloning reference, so its transcript is known
exactly) and a portrait prompt for the image model. Every generated voice is
saved as an ordinary .vcpt checkpoint, so it can be exported, shared, or
replaced by cloning a real recording."""
from __future__ import annotations

from typing import Callable, Optional

import numpy as np

PRESETS: dict[str, dict] = {
    "man": {
        "f0": (85, 160),
        "label": "Tamil man", "label_ta": "ஆண்",
        "instruct": "male, young adult, low pitch, indian accent",
        "line": "டேய் மச்சான், இன்னைக்கு செம்ம வேலை இருக்கு. ஆனா பரவாயில்ல, சாயங்காலம் டீ கடையில மீட் பண்ணலாம், ஓகேவா?",
        "face": "handsome Tamil man in his late twenties, short black hair, trimmed beard, casual cotton shirt",
    },
    "girl": {
        "f0": (165, 290),
        "label": "Tamil girl", "label_ta": "பெண்",
        "instruct": "female, young adult, moderate pitch, indian accent",
        "line": "ஹேய், நீ இதை பாத்தியா? சூப்பரா இருக்குல்ல! நான் இதை உடனே என் ஃப்ரெண்ட்ஸ்கிட்ட ஷேர் பண்ணப் போறேன்.",
        "face": "beautiful young Tamil woman in her early twenties, long black braided hair, small bindi, jasmine flowers in hair, kurti",
    },
    "boy": {
        "f0": (105, 200),
        "label": "Tamil boy", "label_ta": "பையன்",
        "instruct": "male, teenager, moderate pitch, indian accent",
        "line": "அம்மா, ப்ளீஸ் இன்னும் ஒரு பத்து நிமிஷம் விளையாடிட்டு வரேன். ப்ராமிஸ், அப்புறம் ஹோம்வொர்க் எல்லாம் முடிச்சிடுவேன்.",
        "face": "cheerful Tamil teenage boy around fifteen, messy black hair, school uniform shirt",
    },
    "aunty": {
        "f0": (160, 280),
        "label": "Tamil aunty", "label_ta": "அத்தை / ஆன்ட்டி",
        "instruct": "female, middle-aged, moderate pitch, indian accent",
        "line": "அடடா, என்னம்மா இவ்வளவு லேட்டா வர்ற? சாப்பிட்டியா இல்லையா? வா வா, சூடா தோசை ஊத்தித் தர்றேன்.",
        "face": "warm middle-aged Tamil woman in her late forties, silk saree, gold earrings, kumkum, hair in a bun",
    },
    "uncle": {
        "f0": (80, 150),
        "label": "Tamil uncle", "label_ta": "மாமா / அங்கிள்",
        "instruct": "male, middle-aged, low pitch, indian accent",
        "line": "பாருப்பா, வாழ்க்கையில பொறுமை ரொம்ப முக்கியம். அவசரப்பட்டா எதுவுமே சரியா நடக்காது, புரியுதா?",
        "face": "friendly middle-aged Tamil man around fifty, thick moustache, greying hair, white veshti and shirt, vibhuti on forehead",
    },
    "grandpa": {
        "f0": (65, 150),
        "label": "Tamil grandpa", "label_ta": "தாத்தா",
        "instruct": "male, elderly, low pitch, indian accent",
        "line": "அந்தக் காலத்துல நாங்க எல்லாம் நடந்தே பள்ளிக்கூடம் போவோம்டா. இப்போ உங்களுக்கு எல்லாமே ஈஸியா கிடைக்குது.",
        "face": "wise elderly Tamil grandfather in his seventies, white hair, white moustache, round spectacles, white veshti and towel on shoulder",
    },
    "grandma": {
        "f0": (145, 260),
        "label": "Tamil grandma", "label_ta": "பாட்டி",
        "instruct": "female, elderly, moderate pitch, indian accent",
        "line": "வாடா கண்ணு, பாட்டி உனக்கு ஒரு அழகான கதை சொல்றேன். கேக்குறியா? ஒரு ஊர்ல ஒரு ராஜா இருந்தாராம்.",
        "face": "kind elderly Tamil grandmother in her seventies, silver hair in a bun, cotton saree, nose ring, gentle smile",
    },
    "child_boy": {
        "f0": (230, 480),
        "label": "Tamil child (boy)", "label_ta": "சிறுவன்",
        "instruct": "male, child, high pitch",
        "line": "ஐ, பாட்டி! எனக்கு அந்த சிங்கம் கதை தான் வேணும். சிங்கம் எப்படி கர்ஜிக்கும்னு சொல்லுங்க!",
        "face": "adorable seven year old Tamil boy, big curious eyes, short hair, colourful t-shirt",
    },
    "child_girl": {
        "f0": (250, 520),
        "label": "Tamil child (girl)", "label_ta": "சிறுமி",
        "instruct": "female, child, very high pitch",
        "line": "அம்மா அம்மா, பாருங்க! நான் ஒரு அழகான பட்டாம்பூச்சி வரைஞ்சிருக்கேன். உங்களுக்கு பிடிச்சிருக்கா?",
        "face": "adorable six year old Tamil girl, two pigtails with ribbons, pavadai dress, bright smile",
    },
}

NARRATOR_PRESET = "grandma"   # default storyteller


def median_f0(wav: np.ndarray, sr: int) -> float:
    import librosa
    w = librosa.resample(wav, orig_sr=sr, target_sr=16000) if sr != 16000 else wav
    f0 = librosa.pyin(w, fmin=60, fmax=600, sr=16000)[0]
    return float(np.nanmedian(f0)) if np.any(~np.isnan(f0)) else 0.0


def _score_take(wav: np.ndarray, sr: int, text: str, f0_range=None) -> float:
    """Higher is better: intelligibility (Tamil ASR CER), with sanity checks on
    length and on pitch matching the character's age/gender."""
    import jiwer
    from . import asr
    from .voices import _looks_like_speech

    if not _looks_like_speech(wav, sr):
        return -1.0
    dur = len(wav) / sr
    exp = len(text) / 14.0            # ~14 Tamil chars per second of natural speech
    if not (0.55 * exp <= dur <= 1.8 * exp):
        return -0.5
    if f0_range:
        f0 = median_f0(wav, sr)
        if not (f0_range[0] <= f0 <= f0_range[1]):
            return -0.3
    hyp = asr.transcribe(wav, sr, "ta")
    norm = lambda s: "".join(c for c in s if not c.isspace() and c not in ".,!?")
    return 1.0 - min(1.0, jiwer.cer(norm(text), norm(hyp)))


def build_preset_voice(store, key: str, takes: int = 6, progress: Optional[Callable] = None):
    """Design one character voice, keep the most intelligible take and save it
    as a checkpoint whose transcript is the exact sample line."""
    from . import tts_engine

    p = progress or (lambda f, m="": None)
    pr = PRESETS[key]
    best, best_score = None, -9.0
    for i in range(takes):
        p(0.8 * i / takes, f"{pr['label']}: designing voice take {i + 1}/{takes}")
        wav, sr = tts_engine.synthesize(pr["line"], None, "ta", instruct=pr["instruct"], seed=100 + i)
        sc = _score_take(wav, sr, pr["line"], pr.get("f0"))
        if sc > best_score:
            best, best_score = (wav, sr), sc
    p(0.85, f"{pr['label']}: saving checkpoint")
    wav, sr = best
    info = store.create_from_wave(pr["label"], wav, sr, "ta", ref_text=pr["line"], kind="designed", preset=key)
    p(1.0, f"{pr['label']} ready (clarity {best_score:.2f})")
    return info, best_score

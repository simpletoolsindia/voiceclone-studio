"""Seed-VC voice conversion (speech-to-speech), zero-shot.

Two model presets are exposed:

* ``quality``  - DiT + Whisper-small content encoder + BigVGAN 22 kHz. Used for
  converting files (e.g. a narrated story) into the cloned voice.
* ``realtime`` - tiny DiT + XLSR content encoder + HiFT vocoder. Fast enough to
  run on live microphone input with ~0.3-0.5 s latency on a modern GPU.

Seed-VC is GPL-3.0 (https://github.com/Plachtaa/seed-vc); it is fetched by the
setup script into ``engine/third_party/seed-vc`` rather than vendored.
"""
from __future__ import annotations

import os
import sys
import threading
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Optional

import numpy as np
import torch
import torchaudio
import yaml

from . import gpu

SEED_VC_DIR = Path(os.environ.get(
    "SEED_VC_DIR", Path(__file__).resolve().parents[1] / "third_party" / "seed-vc"))
if str(SEED_VC_DIR) not in sys.path:
    sys.path.insert(0, str(SEED_VC_DIR))

PRESETS = {
    "quality": ("DiT_seed_v2_uvit_whisper_small_wavenet_bigvgan_pruned.pth",
                "config_dit_mel_seed_uvit_whisper_small_wavenet.yml"),
    "realtime": ("DiT_uvit_tat_xlsr_ema.pth", "config_dit_mel_seed_uvit_xlsr_tiny.yml"),
}


def _device() -> torch.device:
    if torch.cuda.is_available():
        return torch.device("cuda")
    if getattr(torch.backends, "mps", None) and torch.backends.mps.is_available():
        return torch.device("mps")
    return torch.device("cpu")


@dataclass
class VoiceRef:
    """Pre-computed speaker conditioning for one voice (stored in checkpoints)."""
    prompt_condition: torch.Tensor
    mel2: torch.Tensor
    style2: torch.Tensor

    def to_state(self) -> dict:
        return {k: getattr(self, k).detach().cpu() for k in ("prompt_condition", "mel2", "style2")}

    @classmethod
    def from_state(cls, state: dict, device) -> "VoiceRef":
        return cls(**{k: state[k].to(device) for k in ("prompt_condition", "mel2", "style2")})


class SeedVCModel:
    def __init__(self, preset: str = "quality", device: Optional[torch.device] = None):
        from huggingface_hub import hf_hub_download
        from modules.commons import build_model, load_checkpoint, recursive_munch
        from modules.audio import mel_spectrogram
        from modules.campplus.DTDNN import CAMPPlus

        self.preset = preset
        self.device = device or _device()
        self.lock = threading.Lock()
        ckpt_name, cfg_name = PRESETS[preset]
        ckpt = hf_hub_download("Plachta/Seed-VC", ckpt_name)
        cfg_path = hf_hub_download("Plachta/Seed-VC", cfg_name)
        config = yaml.safe_load(open(cfg_path, "r", encoding="utf-8"))
        mp = recursive_munch(config["model_params"])
        mp.dit_type = "DiT"
        model = build_model(mp, stage="DiT")
        model, *_ = load_checkpoint(model, None, ckpt, load_only_params=True,
                                    ignore_modules=[], is_distributed=False)
        for k in model:
            model[k].eval().to(self.device)
        model.cfm.estimator.setup_caches(max_batch_size=1, max_seq_length=8192)
        self.model = model

        sp = config["preprocess_params"]["spect_params"]
        self.sr = config["preprocess_params"]["sr"]
        self.hop = sp["hop_length"]
        self.mel_args = {
            "n_fft": sp["n_fft"], "win_size": sp["win_length"], "hop_size": sp["hop_length"],
            "num_mels": sp["n_mels"], "sampling_rate": self.sr, "fmin": sp.get("fmin", 0),
            "fmax": None if sp.get("fmax", "None") == "None" else 8000, "center": False,
        }
        self.to_mel = lambda x: mel_spectrogram(x, **self.mel_args)

        campplus = CAMPPlus(feat_dim=80, embedding_size=192)
        campplus.load_state_dict(torch.load(hf_hub_download("funasr/campplus", "campplus_cn_common.bin"),
                                            map_location="cpu"))
        self.campplus = campplus.eval().to(self.device)

        self.vocoder = self._load_vocoder(mp)
        self.semantic_fn = self._load_semantic(mp, config)

    # ------------------------------------------------------------------ loaders
    def _load_vocoder(self, mp):
        from huggingface_hub import hf_hub_download
        vtype = mp.vocoder.type
        if vtype == "bigvgan":
            from modules.bigvgan import bigvgan
            voc = bigvgan.BigVGAN.from_pretrained(mp.vocoder.name, use_cuda_kernel=False)
            voc.remove_weight_norm()
            return voc.eval().to(self.device)
        if vtype == "hifigan":
            from modules.hifigan.generator import HiFTGenerator
            from modules.hifigan.f0_predictor import ConvRNNF0Predictor
            hc = yaml.safe_load(open(SEED_VC_DIR / "configs" / "hifigan.yml", "r", encoding="utf-8"))
            gen = HiFTGenerator(**hc["hift"], f0_predictor=ConvRNNF0Predictor(**hc["f0_predictor"]))
            gen.load_state_dict(torch.load(hf_hub_download("FunAudioLLM/CosyVoice-300M", "hift.pt"),
                                           map_location="cpu"))
            return gen.eval().to(self.device)
        raise ValueError(f"Unsupported vocoder {vtype}")

    def _load_semantic(self, mp, config) -> Callable[[torch.Tensor], torch.Tensor]:
        st = mp.speech_tokenizer
        dev = self.device
        if st.type == "whisper":
            from transformers import AutoFeatureExtractor, WhisperModel
            name = getattr(st, "whisper_name", None) or getattr(st, "name", None) or "openai/whisper-small"
            wm = WhisperModel.from_pretrained(name, torch_dtype=torch.float16).to(dev)
            del wm.decoder
            fe = AutoFeatureExtractor.from_pretrained(name)

            def fn(w16: torch.Tensor) -> torch.Tensor:
                inp = fe([w16.squeeze(0).cpu().numpy()], return_tensors="pt",
                         return_attention_mask=True, sampling_rate=16000)
                feats = wm._mask_input_features(inp.input_features, attention_mask=inp.attention_mask).to(dev)
                out = wm.encoder(feats.to(wm.encoder.dtype), return_dict=True)
                return out.last_hidden_state.float()[:, : w16.size(-1) // 320 + 1]
            return fn
        if st.type == "xlsr":
            from transformers import Wav2Vec2FeatureExtractor, Wav2Vec2Model
            name = config["model_params"]["speech_tokenizer"]["name"]
            layer = config["model_params"]["speech_tokenizer"]["output_layer"]
            fe = Wav2Vec2FeatureExtractor.from_pretrained(name)
            wm = Wav2Vec2Model.from_pretrained(name)
            wm.encoder.layers = wm.encoder.layers[:layer]
            wm = wm.to(dev).eval().half()

            def fn(w16: torch.Tensor) -> torch.Tensor:
                inp = fe([w16[i].cpu().numpy() for i in range(len(w16))], return_tensors="pt",
                         return_attention_mask=True, padding=True, sampling_rate=16000).to(dev)
                return wm(inp.input_values.half()).last_hidden_state.float()
            return fn
        raise ValueError(f"Unsupported speech tokenizer {st.type}")

    # --------------------------------------------------------------- reference
    @torch.inference_mode()
    def make_ref(self, ref_wave: np.ndarray, max_sec: float = 15.0) -> VoiceRef:
        """`ref_wave` must be mono float32 at self.sr."""
        ref = torch.from_numpy(ref_wave[: int(self.sr * max_sec)]).float().to(self.device)[None]
        w16 = torchaudio.functional.resample(ref, self.sr, 16000)
        s_ori = self.semantic_fn(w16)
        feat = torchaudio.compliance.kaldi.fbank(w16, num_mel_bins=80, dither=0, sample_frequency=16000)
        feat = feat - feat.mean(dim=0, keepdim=True)
        style2 = self.campplus(feat.unsqueeze(0))
        mel2 = self.to_mel(ref)
        prompt = self.model.length_regulator(s_ori, ylens=torch.LongTensor([mel2.size(2)]).to(self.device),
                                             n_quantizers=3, f0=None)[0]
        return VoiceRef(prompt, mel2, style2)

    # --------------------------------------------------------------- convert
    @torch.inference_mode()
    def convert_segment(self, wave: np.ndarray, ref: VoiceRef, steps: int = 30,
                        cfg_rate: float = 0.7, length_adjust: float = 1.0) -> np.ndarray:
        """Convert one segment (<= ~15 s, mono float32 at self.sr)."""
        src = torch.from_numpy(wave).float().to(self.device)[None]
        w16 = torchaudio.functional.resample(src, self.sr, 16000)
        s_alt = self.semantic_fn(w16)
        n_frames = int(np.ceil(len(wave) / self.hop * length_adjust))
        cond = self.model.length_regulator(s_alt, ylens=torch.LongTensor([n_frames]).to(self.device),
                                           n_quantizers=3, f0=None)[0]
        cat = torch.cat([ref.prompt_condition, cond], dim=1)
        use_amp = self.device.type == "cuda"
        with torch.autocast(device_type=self.device.type, dtype=torch.float16, enabled=use_amp):
            mel = self.model.cfm.inference(cat, torch.LongTensor([cat.size(1)]).to(self.device),
                                           ref.mel2, ref.style2, None, steps,
                                           inference_cfg_rate=cfg_rate)
        mel = mel[:, :, ref.mel2.size(-1):]
        out = self.vocoder(mel.float()).squeeze().float().cpu().numpy()
        return out[: int(len(wave) * length_adjust)] if length_adjust == 1.0 else out

    def convert_long(self, wave: np.ndarray, ref: VoiceRef, steps: int = 30, cfg_rate: float = 0.7,
                     progress: Optional[Callable[[float], None]] = None) -> np.ndarray:
        """Convert a recording of any length by cutting it at natural pauses,
        converting each piece and stitching with short crossfades."""
        from .audio_utils import split_on_silence

        segs = split_on_silence(wave, self.sr, max_sec=14.0, min_sec=4.0)
        out = np.zeros(len(wave), dtype=np.float32)
        fade = int(0.01 * self.sr)
        with self.lock:
            for i, (a, b) in enumerate(segs):
                piece = wave[a:b]
                if np.sqrt(np.mean(piece**2)) < 1e-4:   # pure silence: keep it silent
                    conv = np.zeros_like(piece)
                else:
                    conv = self.convert_segment(piece, ref, steps, cfg_rate)
                    if len(conv) < len(piece):
                        conv = np.pad(conv, (0, len(piece) - len(conv)))
                    conv = conv[: len(piece)]
                    ramp = np.linspace(0, 1, fade, dtype=np.float32)
                    conv[:fade] *= ramp
                    conv[-fade:] *= ramp[::-1]
                out[a:b] = conv
                if progress:
                    progress((i + 1) / len(segs))
        return out


_models: dict[str, SeedVCModel] = {}
_models_lock = threading.Lock()


def get_model(preset: str = "quality") -> SeedVCModel:
    with _models_lock:
        if preset not in _models:
            _models[preset] = SeedVCModel(preset)
            gpu.mark_loaded("vc")
        return _models[preset]


def unload():
    with _models_lock:
        _models.clear()


gpu.register("vc", unload)

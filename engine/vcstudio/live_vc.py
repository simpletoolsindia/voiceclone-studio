"""Live voice conversion session (GPU side).

The Rust app owns the microphone and speakers (cpal, real-time ring buffers)
and streams fixed-size float32 mono blocks to this session over a local
WebSocket; each block is converted and returned. Port of Seed-VC's real-time
pipeline: a sliding context window feeds the content encoder, the DiT
generates the newest block, and SOLA crossfading stitches blocks seamlessly.
"""
from __future__ import annotations

import time
from dataclasses import dataclass

import numpy as np
import torch
import torch.nn.functional as F
import torchaudio.transforms as tat


@dataclass
class LiveConfig:
    sample_rate: int = 48000
    diffusion_steps: int = 8
    cfg_rate: float = 0.7
    block_time: float = 0.25      # seconds per block (main latency knob)
    crossfade_time: float = 0.05
    extra_time_ce: float = 2.5    # left context for the content encoder
    extra_time: float = 0.5       # left context for the DiT
    extra_time_right: float = 0.02
    gate_db: float = -50.0        # input quieter than this is treated as silence


class LiveSession:
    def __init__(self, store, voice_id: str, cfg: LiveConfig):
        from . import gpu, seedvc_engine

        gpu.reserve("vc")
        self.cfg = cfg
        self.model = m = seedvc_engine.get_model("realtime")
        self.ref = store.vc_ref(voice_id, "realtime")
        dev = self.dev = m.device
        sr = self.sr = cfg.sample_rate
        zc = self.zc = sr // 50
        r = lambda t: int(np.round(t * sr / zc)) * zc
        self.block_frame = r(cfg.block_time)
        self.block_frame_16k = 320 * self.block_frame // zc
        crossfade = r(cfg.crossfade_time)
        self.sola_buffer_frame = min(crossfade, 4 * zc)
        self.sola_search_frame = zc
        extra = r(cfg.extra_time_ce)
        extra_right = r(cfg.extra_time_right)
        total = extra + crossfade + self.sola_search_frame + self.block_frame + extra_right
        self.input_wav = torch.zeros(total, device=dev)
        self.input_wav_res = torch.zeros(320 * total // zc, device=dev)
        self.sola_buffer = torch.zeros(self.sola_buffer_frame, device=dev)
        self.skip_head = extra // zc
        self.skip_tail = extra_right // zc
        self.return_length = (self.block_frame + self.sola_buffer_frame + self.sola_search_frame) // zc
        fade = torch.sin(0.5 * np.pi * torch.linspace(0.0, 1.0, self.sola_buffer_frame, device=dev)) ** 2
        self.fade_in, self.fade_out = fade, 1 - fade
        self.resample_in = tat.Resample(sr, 16000).to(dev)
        self.resample_out = tat.Resample(m.sr, sr).to(dev) if m.sr != sr else None
        self.ce_dit_diff = cfg.extra_time_ce - cfg.extra_time
        self.silent_blocks = 0
        self.last_infer_ms = 0.0

    @property
    def block_size(self) -> int:
        return self.block_frame

    @torch.inference_mode()
    def _infer(self) -> torch.Tensor:
        m, ref = self.model, self.ref
        s_alt = m.semantic_fn(self.input_wav_res.unsqueeze(0))
        diff = int(self.ce_dit_diff * 50)
        s_alt = s_alt[:, diff:]
        tl = int((self.skip_head + self.return_length + self.skip_tail - diff) / 50 * m.sr // m.hop)
        cond = m.model.length_regulator(s_alt, ylens=torch.LongTensor([tl]).to(self.dev), n_quantizers=3, f0=None)[0]
        cat = torch.cat([ref.prompt_condition, cond], dim=1)
        mel = m.model.cfm.inference(cat, torch.LongTensor([cat.size(1)]).to(self.dev), ref.mel2, ref.style2,
                                    None, n_timesteps=self.cfg.diffusion_steps, inference_cfg_rate=self.cfg.cfg_rate)
        wav = m.vocoder(mel[:, :, ref.mel2.size(-1):]).squeeze()
        out_len = self.return_length * m.sr // 50
        tail = self.skip_tail * m.sr // 50
        wav = wav[-out_len - tail: -tail] if tail > 0 else wav[-out_len:]
        return self.resample_out(wav) if self.resample_out is not None else wav

    @torch.inference_mode()
    def process(self, block: np.ndarray) -> np.ndarray:
        """block: float32 mono, exactly `block_size` samples -> converted block."""
        t0 = time.perf_counter()
        bf, zc = self.block_frame, self.zc
        x = np.asarray(block, dtype=np.float32)[:bf]
        if len(x) < bf:
            x = np.pad(x, (0, bf - len(x)))
        level = 20 * np.log10(np.sqrt(np.mean(x**2)) + 1e-9)
        self.input_wav[:-bf] = self.input_wav[bf:].clone()
        self.input_wav[-bf:] = torch.from_numpy(x).to(self.dev)
        self.input_wav_res[:-self.block_frame_16k] = self.input_wav_res[self.block_frame_16k:].clone()
        self.input_wav_res[-320 * (bf // zc + 1):] = self.resample_in(self.input_wav[-bf - 2 * zc:])[320:]

        self.silent_blocks = self.silent_blocks + 1 if level < self.cfg.gate_db else 0
        if self.silent_blocks > 2:   # keep converting briefly after speech ends, then mute
            infer_wav = torch.zeros(bf + self.sola_buffer_frame + self.sola_search_frame, device=self.dev)
        else:
            infer_wav = self._infer()

        conv_in = infer_wav[None, None, : self.sola_buffer_frame + self.sola_search_frame]
        nom = F.conv1d(conv_in, self.sola_buffer[None, None, :])
        den = torch.sqrt(F.conv1d(conv_in**2, torch.ones(1, 1, self.sola_buffer_frame, device=self.dev)) + 1e-8)
        corr = nom[0, 0] / den[0, 0]
        off = int(torch.argmax(corr).item()) if corr.numel() > 1 else 0
        infer_wav = infer_wav[off:]
        infer_wav[: self.sola_buffer_frame] *= self.fade_in
        infer_wav[: self.sola_buffer_frame] += self.sola_buffer * self.fade_out
        self.sola_buffer[:] = infer_wav[bf: bf + self.sola_buffer_frame]
        out = infer_wav[:bf].clamp(-1, 1).float().cpu().numpy()
        self.last_infer_ms = (time.perf_counter() - t0) * 1000
        return out

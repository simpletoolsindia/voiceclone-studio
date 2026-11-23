"""GPU memory coordination.

A 12 GB card cannot hold OmniVoice, Seed-VC, the image model and the
talking-face model at the same time. Each engine registers an unload
function; before a heavy stage starts it calls ``reserve(group)`` which
unloads every other group."""
from __future__ import annotations

import gc
import threading
from typing import Callable

_unloaders: dict[str, Callable[[], None]] = {}
_lock = threading.Lock()
_active: set[str] = set()


def register(group: str, unload: Callable[[], None]) -> None:
    _unloaders[group] = unload


def mark_loaded(group: str) -> None:
    _active.add(group)


def reserve(*groups: str) -> None:
    """Free every loaded model group except `groups`."""
    with _lock:
        for g in list(_active):
            if g not in groups and g in _unloaders:
                try:
                    _unloaders[g]()
                finally:
                    _active.discard(g)
        empty_cache()


def unload_all() -> None:
    reserve()


def empty_cache() -> None:
    gc.collect()
    try:
        import torch
        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    except Exception:
        pass


def status() -> dict:
    out = {"loaded": sorted(_active)}
    try:
        import torch
        if torch.cuda.is_available():
            free, total = torch.cuda.mem_get_info()
            out.update(gpu=torch.cuda.get_device_name(0), vram_total_gb=round(total / 1e9, 1),
                       vram_free_gb=round(free / 1e9, 1))
    except Exception:
        pass
    return out

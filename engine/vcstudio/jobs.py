"""Background job queue with progress, stage and ETA reporting.

Every long operation (cloning, TTS, conversion, image/video rendering) runs as
a job. GPU jobs run one at a time (a 12 GB card cannot hold every model at
once), so they queue; the UI polls ``/jobs`` to draw progress bars and can
cancel a job between steps.
"""
from __future__ import annotations

import threading
import time
import traceback
import uuid
from collections import OrderedDict
from dataclasses import dataclass, field, asdict
from typing import Any, Callable, Optional


class Cancelled(Exception):
    pass


@dataclass
class Job:
    id: str
    kind: str
    title: str
    status: str = "queued"          # queued | running | done | error | cancelled
    progress: float = 0.0           # 0..1
    stage: str = "Waiting in queue"
    created: float = field(default_factory=time.time)
    started: Optional[float] = None
    finished: Optional[float] = None
    result: Any = None
    preview: Optional[str] = None      # image of what is being generated right now
    error: Optional[str] = None
    cancel_requested: bool = False

    def public(self) -> dict:
        d = asdict(self)
        d.pop("cancel_requested")
        d["percent"] = round(self.progress * 100, 1)
        d["eta_sec"] = self.eta()
        d["elapsed_sec"] = round((self.finished or time.time()) - self.started, 1) if self.started else 0
        return d

    def eta(self) -> Optional[float]:
        if self.status != "running" or not self.started or self.progress < 0.03:
            return None
        el = time.time() - self.started
        return round(el / self.progress - el, 1)


class JobQueue:
    def __init__(self, max_history: int = 200):
        self.jobs: "OrderedDict[str, Job]" = OrderedDict()
        self._cv = threading.Condition()
        self._pending: list[tuple[Job, Callable]] = []
        self.max_history = max_history
        threading.Thread(target=self._worker, daemon=True, name="gpu-worker").start()

    def submit(self, kind: str, title: str, fn: Callable[[Callable[[float, str], None]], Any]) -> Job:
        job = Job(id=uuid.uuid4().hex[:10], kind=kind, title=title)
        with self._cv:
            self.jobs[job.id] = job
            while len(self.jobs) > self.max_history:
                oldest = next(iter(self.jobs))
                if self.jobs[oldest].status in ("queued", "running"):
                    break
                self.jobs.pop(oldest)
            self._pending.append((job, fn))
            self._cv.notify()
        return job

    def cancel(self, jid: str) -> bool:
        with self._cv:
            job = self.jobs.get(jid)
            if not job or job.status not in ("queued", "running"):
                return False
            job.cancel_requested = True
            if job.status == "queued":
                self._pending = [(j, f) for j, f in self._pending if j.id != jid]
                job.status, job.stage, job.finished = "cancelled", "Cancelled", time.time()
            return True

    def get(self, jid: str) -> Optional[Job]:
        return self.jobs.get(jid)

    def list(self) -> list[dict]:
        return [j.public() for j in reversed(self.jobs.values())]

    def clear_finished(self) -> None:
        with self._cv:
            for k in [k for k, j in self.jobs.items() if j.status in ("done", "error", "cancelled")]:
                self.jobs.pop(k)

    def _worker(self):
        while True:
            with self._cv:
                while not self._pending:
                    self._cv.wait()
                job, fn = self._pending.pop(0)
            job.status, job.started, job.stage = "running", time.time(), "Starting"

            def progress(frac: float, stage: str = "", preview: Optional[str] = None, _job=job):
                if _job.cancel_requested:
                    raise Cancelled()
                _job.progress = max(_job.progress, min(1.0, float(frac)))
                if stage:
                    _job.stage = stage
                if preview:
                    _job.preview = str(preview)

            try:
                job.result = fn(progress)
                job.status, job.progress, job.stage = "done", 1.0, "Finished"
            except Cancelled:
                job.status, job.stage = "cancelled", "Cancelled"
            except Exception as e:  # report, never kill the worker
                job.status, job.error, job.stage = "error", f"{type(e).__name__}: {e}", "Failed"
                traceback.print_exc()
            finally:
                job.finished = time.time()


queue = JobQueue()


def sub_progress(progress: Callable[[float, str], None], start: float, end: float):
    """Map a child task's 0..1 progress into [start, end] of the parent."""
    def p(frac: float, stage: str = "", preview: Optional[str] = None):
        progress(start + (end - start) * max(0.0, min(1.0, frac)), stage, preview)
    return p

from __future__ import annotations

import json
import time
from pathlib import Path

from agent_signals.config import DEFAULT_DEDUP_SECONDS, DEFAULT_STATE_DIR, DEFAULT_VOICE_STATE_DIR
from agent_signals.models import NotifyPayload


def ensure_dir(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path


class DeduplicationStore:
    def __init__(self, state_dir: Path = DEFAULT_STATE_DIR, window: int = DEFAULT_DEDUP_SECONDS):
        self._dir = ensure_dir(state_dir)
        self._window = window

    def is_duplicate(self, key: str) -> bool:
        key_file = self._dir / key
        now = int(time.time())
        if key_file.exists():
            try:
                last = int(key_file.read_text().strip())
            except (ValueError, OSError):
                last = 0
            if now - last < self._window:
                return True
        key_file.write_text(str(now))
        return False

    def save_context(self, key: str, payload: NotifyPayload) -> Path:
        context_file = self._dir / f"{key}.json"
        context_file.write_text(json.dumps(payload.to_dict()))
        return context_file


class VoiceState:
    def __init__(self, state_dir: Path = DEFAULT_VOICE_STATE_DIR):
        self._dir = ensure_dir(state_dir)
        self._enabled_file = self._dir / "enabled"
        self._last_key_file = self._dir / "last-spoken-key"
        self._pid_file = self._dir / "current-pid"
        self._text_file = self._dir / "current.txt"
        self._wav_file = self._dir / "current.wav"

    @property
    def enabled(self) -> bool:
        return self._enabled_file.exists()

    @enabled.setter
    def enabled(self, value: bool) -> None:
        if value:
            self._enabled_file.touch()
        else:
            self._enabled_file.unlink(missing_ok=True)

    @property
    def pid_file(self) -> Path:
        return self._pid_file

    @property
    def wav_file(self) -> Path:
        return self._wav_file

    @property
    def text_file(self) -> Path:
        return self._text_file

    def current_pid(self) -> int | None:
        if not self._pid_file.exists():
            return None
        try:
            return int(self._pid_file.read_text().strip())
        except (ValueError, OSError):
            return None

    def save_pid(self, pid: int) -> None:
        self._pid_file.write_text(str(pid))

    def clear_pid(self) -> None:
        self._pid_file.unlink(missing_ok=True)

    def is_duplicate_key(self, key: str) -> bool:
        if not key:
            return False
        if self._last_key_file.exists():
            try:
                return self._last_key_file.read_text().strip() == key
            except OSError:
                pass
        return False

    def save_last_key(self, key: str) -> None:
        if key:
            self._last_key_file.write_text(key)

    def is_playing(self) -> bool:
        pid = self.current_pid()
        if pid is None:
            return False
        try:
            import os
            os.kill(pid, 0)
            return True
        except OSError:
            return False

    def status_line(self) -> str:
        state = "on" if self.enabled else "off"
        if self.is_playing():
            return f"voice: {state}, playing"
        return f"voice: {state}"

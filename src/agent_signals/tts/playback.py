from __future__ import annotations

import os
import signal
import subprocess
import time
from pathlib import Path

from agent_signals.state import VoiceState


def _is_voice_pid(pid: int, wav_file: Path, kokoro_root: Path) -> bool:
    try:
        proc = subprocess.run(
            ["ps", "-p", str(pid), "-o", "command="],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
        )
    except Exception:
        return False
    cmd = proc.stdout.strip()
    if not cmd:
        return False
    return (
        str(wav_file) in cmd
        or "kokoro-v1.0.onnx" in cmd
        or "kokoro-speak" in cmd
    )


def _kill_tree(pid: int) -> None:
    try:
        children = subprocess.run(
            ["pgrep", "-P", str(pid)],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
        ).stdout.strip().split()
    except Exception:
        children = []
    for child in children:
        try:
            _kill_tree(int(child))
        except ValueError:
            pass
    try:
        os.kill(pid, signal.SIGTERM)
    except OSError:
        pass


def stop_playback(voice_state: VoiceState, kokoro_root: Path | None = None) -> None:
    from agent_signals.config import DEFAULT_KOKORO_ROOT
    kokoro_root = kokoro_root or DEFAULT_KOKORO_ROOT

    pid = voice_state.current_pid()
    if pid is not None and _is_voice_pid(pid, voice_state.wav_file, kokoro_root):
        _kill_tree(pid)

    wav_str = str(voice_state.wav_file)
    subprocess.run(
        ["pkill", "-TERM", "-f", wav_str],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )

    time.sleep(0.1)

    pid = voice_state.current_pid()
    if pid is not None and _is_voice_pid(pid, voice_state.wav_file, kokoro_root):
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass

    subprocess.run(
        ["pkill", "-KILL", "-f", wav_str],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )

    voice_state.clear_pid()

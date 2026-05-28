from __future__ import annotations

import subprocess
from pathlib import Path

from agent_signals.config import DEFAULT_SOUND


def play_sound(sound_file: Path = DEFAULT_SOUND) -> None:
    if not sound_file.exists():
        return
    subprocess.Popen(
        ["/usr/bin/afplay", str(sound_file)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

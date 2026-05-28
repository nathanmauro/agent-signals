from __future__ import annotations

import subprocess
from pathlib import Path

from agent_signals.config import DEFAULT_KOKORO_ROOT


def speak_kokoro(
    text: str,
    out_file: Path,
    voice: str = "af_bella",
    speed: str = "1.0",
    lang: str = "en-us",
    kokoro_root: Path = DEFAULT_KOKORO_ROOT,
) -> subprocess.Popen | None:
    python = kokoro_root / ".venv" / "bin" / "python"
    if not python.exists():
        return None

    script = f"""\
import sys
import soundfile as sf
from kokoro_onnx import Kokoro

root, out, voice, speed, lang = sys.argv[1:6]
text = sys.stdin.read()
kokoro = Kokoro(f"{{root}}/kokoro-v1.0.onnx", f"{{root}}/voices-v1.0.bin")
samples, sample_rate = kokoro.create(text, voice=voice, speed=float(speed), lang=lang)
sf.write(out, samples, sample_rate)
"""
    synth = subprocess.Popen(
        [
            str(python), "-c", script,
            str(kokoro_root), str(out_file), voice, speed, lang,
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    synth.communicate(input=text.encode())
    if synth.returncode != 0:
        return None

    player = subprocess.Popen(
        ["afplay", str(out_file)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return player

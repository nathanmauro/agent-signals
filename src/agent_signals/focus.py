from __future__ import annotations

import json
import subprocess
from pathlib import Path

from agent_signals.models import MuxContext, NotifyPayload
from agent_signals.mux.tmux import focus_tmux
from agent_signals.mux.zellij import focus_zellij


def activate_terminal() -> bool:
    result = subprocess.run(
        ["/usr/bin/osascript", "-e", 'tell application "Ghostty" to activate'],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if result.returncode == 0:
        return True
    subprocess.run(
        ["/usr/bin/open", "-a", "Ghostty"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return True


def focus_pane(context_file: str) -> bool:
    path = Path(context_file)
    if not path.exists():
        return False
    try:
        data = json.loads(path.read_text())
    except Exception:
        return False

    payload = NotifyPayload.from_dict(data)
    mux = payload.mux

    activate_terminal()

    if mux.type == "zellij":
        return focus_zellij(mux)
    elif mux.type == "tmux":
        return focus_tmux(mux)
    return False

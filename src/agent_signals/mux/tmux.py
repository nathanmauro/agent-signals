from __future__ import annotations

import os
import subprocess

from agent_signals.models import MuxContext, clean_text


def tmux_context() -> MuxContext:
    pane_id = os.environ.get("TMUX_PANE", "")
    if not pane_id:
        return MuxContext()
    fmt = "#{session_name}\t#{session_id}\t#{window_index}\t#{window_id}\t#{window_name}\t#{pane_id}\t#{pane_title}"
    try:
        proc = subprocess.run(
            ["tmux", "display-message", "-p", "-t", pane_id, fmt],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=2.0,
            check=False,
        )
    except Exception:
        return MuxContext()
    if proc.returncode != 0:
        return MuxContext()
    parts = proc.stdout.rstrip("\n").split("\t")
    if len(parts) < 7:
        return MuxContext()
    return MuxContext(
        type="tmux",
        session=clean_text(parts[0]),
        session_id=clean_text(parts[1]),
        window_index=clean_text(parts[2]),
        window_id=clean_text(parts[3]),
        window_name=clean_text(parts[4]),
        pane_id=clean_text(parts[5]),
        pane_title=clean_text(parts[6]),
    )


def focus_tmux(mux: MuxContext) -> bool:
    ok = True
    if mux.session:
        r = subprocess.run(
            ["tmux", "switch-client", "-t", mux.session],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if r.returncode != 0:
            ok = False
    if mux.window_id:
        r = subprocess.run(
            ["tmux", "select-window", "-t", mux.window_id],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if r.returncode != 0:
            ok = False
    if mux.pane_id:
        r = subprocess.run(
            ["tmux", "select-pane", "-t", mux.pane_id],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if r.returncode != 0:
            ok = False
    return ok

from __future__ import annotations

import os

from agent_signals.models import MuxContext
from agent_signals.mux.tmux import tmux_context
from agent_signals.mux.zellij import zellij_context


def detect_mux() -> MuxContext:
    if os.environ.get("ZELLIJ_PANE_ID"):
        ctx = zellij_context()
        if ctx.type:
            return ctx
    if os.environ.get("TMUX_PANE"):
        ctx = tmux_context()
        if ctx.type:
            return ctx
    return MuxContext()

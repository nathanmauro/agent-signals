from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

from agent_signals.models import MuxContext, clean_text


def _run_json(command: list[str]) -> object:
    try:
        proc = subprocess.run(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=2.0,
            check=False,
        )
    except Exception:
        return None
    if proc.returncode != 0 or not proc.stdout.strip():
        return None
    try:
        return json.loads(proc.stdout)
    except Exception:
        return None


def zellij_context() -> MuxContext:
    pane_id = os.environ.get("ZELLIJ_PANE_ID", "")
    session = os.environ.get("ZELLIJ_SESSION_NAME", "")
    if not pane_id:
        return MuxContext()
    pane_ref = pane_id if pane_id.startswith(("terminal_", "plugin_")) else f"terminal_{pane_id}"
    command = ["zellij"]
    if session:
        command.extend(["--session", session])
    command.extend(["action", "list-panes", "--json", "--all", "--state", "--tab", "--command"])
    panes = _run_json(command)
    found: dict = {}
    if isinstance(panes, list):
        bare_id = str(pane_id).removeprefix("terminal_").removeprefix("plugin_")
        for pane in panes:
            if str(pane.get("id")) == bare_id:
                found = pane
                break
    return MuxContext(
        type="zellij",
        session=session,
        pane_ref=pane_ref,
        pane_title=clean_text(found.get("title")) if found else "",
        tab_name=clean_text(found.get("tab_name")) if found else "",
        tab_id=str(found.get("tab_id", "")) if found else "",
        tab_position=str(found.get("tab_position", "")) if found else "",
        cwd=clean_text(found.get("pane_cwd")) if found else "",
        command=clean_text(found.get("pane_command")) if found else "",
    )


def focus_zellij(mux: MuxContext) -> bool:
    if not mux.session or not mux.pane_ref:
        return False
    ok = True
    if mux.tab_id:
        result = subprocess.run(
            ["zellij", "--session", mux.session, "action", "go-to-tab-by-id", mux.tab_id],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if result.returncode != 0:
            ok = False
    result = subprocess.run(
        ["zellij", "--session", mux.session, "action", "focus-pane-id", mux.pane_ref],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return ok and result.returncode == 0

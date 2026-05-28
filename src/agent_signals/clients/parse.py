from __future__ import annotations

import json
from pathlib import Path

from agent_signals.models import AgentEvent
from agent_signals.mux import detect_mux


def parse_hook_payload(label: str = "", raw: str = "") -> AgentEvent | None:
    data: dict = {}
    if raw.strip():
        try:
            data = json.loads(raw)
        except Exception:
            data = {}

    event_type = data.get("type") or data.get("hook_event_name") or ""
    if event_type and event_type not in {"agent-turn-complete", "Stop"}:
        return None

    transcript = str(data.get("transcript_path") or data.get("transcriptPath") or "")
    if label:
        client = label
    elif ".claude" in transcript:
        client = "Claude Code"
    else:
        client = "Codex"

    cwd = str(data.get("cwd") or "")
    mux = detect_mux()

    return AgentEvent(
        client=client,
        event_type=event_type,
        thread_id=str(data.get("thread-id") or data.get("session_id") or ""),
        turn_id=str(data.get("turn-id") or data.get("turn_id") or ""),
        cwd=cwd,
        transcript_path=transcript,
        mux=mux,
        raw=data,
    )

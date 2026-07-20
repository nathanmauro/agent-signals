from __future__ import annotations

from pathlib import Path

from agent_signals.transcripts.claude import (
    extract_previous_turn_from_claude,
    extract_text_from_claude,
)
from agent_signals.transcripts.codex import extract_text_from_codex


def _latest(pattern: str) -> Path | None:
    files = list(Path.home().glob(pattern))
    if not files:
        return None
    try:
        return max(files, key=lambda p: p.stat().st_mtime)
    except Exception:
        return None


def extract_previous_turn_response(transcript_path: str) -> tuple[str, str]:
    """Final assistant text of the last *completed* turn (Claude transcripts only).

    Unlike extract_last_response, this skips the in-flight turn, so it is safe
    to call from a tool running inside an active turn (e.g. the /say command).
    """
    path = Path(transcript_path).expanduser() if transcript_path else None
    if path and path.exists() and ".claude" in path.parts:
        return extract_previous_turn_from_claude(path)
    return extract_last_response(transcript_path)


def extract_last_response(transcript_path: str = "") -> tuple[str, str]:
    path = Path(transcript_path).expanduser() if transcript_path else None

    if path and path.exists():
        if ".claude" in path.parts:
            return extract_text_from_claude(path)
        return extract_text_from_codex(path)

    codex_path = _latest(".codex/sessions/**/*.jsonl")
    if codex_path:
        text, key = extract_text_from_codex(codex_path)
        if text:
            return text, key

    claude_path = _latest(".claude/projects/**/*.jsonl")
    if claude_path:
        text, key = extract_text_from_claude(claude_path)
        if text:
            return text, key

    return "", ""

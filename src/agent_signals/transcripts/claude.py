from __future__ import annotations

import json
from pathlib import Path


def _assistant_text(obj: dict) -> str:
    msg = obj.get("message")
    if not isinstance(msg, dict):
        return ""
    parts = []
    for item in msg.get("content", []):
        if isinstance(item, dict) and item.get("type") == "text":
            value = item.get("text", "")
            if isinstance(value, str):
                parts.append(value)
    return "\n".join(parts).strip()


def _user_prompt_text(obj: dict) -> str | None:
    """Return prompt text for a real user turn, None for tool results / meta noise."""
    if obj.get("type") != "user" or obj.get("isMeta"):
        return None
    msg = obj.get("message")
    if not isinstance(msg, dict):
        return None
    content = msg.get("content")
    if isinstance(content, str):
        return content if content.strip() else None
    if isinstance(content, list):
        texts = []
        for item in content:
            if not isinstance(item, dict):
                continue
            if item.get("type") == "tool_result":
                return None
            if item.get("type") == "text" and isinstance(item.get("text"), str):
                texts.append(item["text"])
        joined = "\n".join(texts)
        return joined if joined.strip() else None
    return None


def extract_previous_turn_from_claude(path: Path) -> tuple[str, str]:
    """Last completed turn's final assistant text — ignores the in-flight turn.

    Used by on-demand speak (/say): at invocation time the transcript already
    contains the new user message and the in-flight assistant tool calls, so the
    bottom-most assistant text is not the turn the user wants read. Turns whose
    prompt is itself a /say invocation are skipped so repeating /say does not
    read its own confirmation line.
    """
    key = str(path)
    try:
        lines = path.read_text(errors="ignore").splitlines()
    except Exception:
        return "", key

    objs = []
    for line in lines:
        try:
            obj = json.loads(line)
        except Exception:
            continue
        if isinstance(obj, dict):
            objs.append(obj)

    # Boundary: the most recent real user prompt starts the in-flight turn.
    boundary = len(objs)
    for i in range(len(objs) - 1, -1, -1):
        if _user_prompt_text(objs[i]) is not None:
            boundary = i
            break

    last_text: tuple[str, str] | None = None
    for i in range(boundary - 1, -1, -1):
        obj = objs[i]
        prompt = _user_prompt_text(obj)
        if prompt is not None:
            if "<command-name>/say" in prompt:
                last_text = None
                continue
            if last_text:
                return last_text
            continue
        if obj.get("type") != "assistant":
            continue
        text = _assistant_text(obj)
        if text and last_text is None:
            entry_key = obj.get("uuid") or obj.get("requestId") or f"{path}:{i}"
            last_text = (text, str(entry_key))
    return last_text if last_text else ("", key)


def extract_text_from_claude(path: Path) -> tuple[str, str]:
    key = str(path)
    try:
        lines = path.read_text(errors="ignore").splitlines()
    except Exception:
        return "", key
    for line in reversed(lines):
        try:
            obj = json.loads(line)
        except Exception:
            continue
        if obj.get("type") != "assistant":
            continue
        msg = obj.get("message")
        if not isinstance(msg, dict):
            continue
        parts = []
        for item in msg.get("content", []):
            if isinstance(item, dict) and item.get("type") == "text":
                value = item.get("text", "")
                if isinstance(value, str):
                    parts.append(value)
        text = "\n".join(parts).strip()
        key = obj.get("uuid") or obj.get("requestId") or f"{path}:{len(lines)}"
        return text, str(key)
    return "", key

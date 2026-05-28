from __future__ import annotations

import json
from pathlib import Path


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

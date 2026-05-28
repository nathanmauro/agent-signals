from __future__ import annotations

import json
from pathlib import Path


def extract_text_from_codex(path: Path) -> tuple[str, str]:
    key = str(path)
    try:
        lines = path.read_text(errors="ignore").splitlines()
    except Exception:
        return "", key
    for idx, line in enumerate(reversed(lines)):
        try:
            obj = json.loads(line)
        except Exception:
            continue
        if obj.get("type") != "response_item":
            continue
        payload = obj.get("payload")
        if not isinstance(payload, dict):
            continue
        if payload.get("type") != "message" or payload.get("role") != "assistant":
            continue
        parts = []
        for item in payload.get("content", []):
            if isinstance(item, dict) and item.get("type") in {"output_text", "text"}:
                value = item.get("text", "")
                if isinstance(value, str):
                    parts.append(value)
        text = "\n".join(parts).strip()
        key = f"{path}:{len(lines) - idx}"
        return text, key
    return "", key

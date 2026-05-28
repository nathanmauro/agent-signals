from __future__ import annotations

import re
from pathlib import Path
from urllib.parse import urlparse

from agent_signals.config import VoiceRule


def strip_memory_citation(text: str) -> str:
    return text.split("<oai-mem-citation>", 1)[0].strip()


def _domain_for_url(value: str) -> str:
    try:
        parsed = urlparse(value)
    except Exception:
        return "link"
    return parsed.netloc or "link"


def make_speakable(text: str, rule: VoiceRule) -> str:
    text = strip_memory_citation(text)
    had_code = "```" in text
    text = re.sub(r"```[\s\S]*?```", " ", text)
    text = re.sub(r"`([^`]+)`", r"\1", text)
    text = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r"\1", text)
    text = re.sub(
        r"https?://[^\s)]+",
        lambda m: _domain_for_url(m.group(0)),
        text,
    )
    text = re.sub(
        r"/Users/\w+/[^\s),;:]+",
        lambda m: Path(m.group(0)).name or "local path",
        text,
    )
    text = re.sub(r"(?m)^\s*[-*]\s+", "", text)
    text = re.sub(r"(?m)^\s*\d+\.\s+", "", text)
    text = re.sub(r"\s*\n\s*", " ", text)
    text = re.sub(r"[ \t]+", " ", text)
    text = text.strip()

    if had_code and not text:
        text = "Code details are on screen."

    if rule.prefix and not text.lower().startswith(rule.prefix.lower()):
        text = f"{rule.prefix}{text}"

    max_chars = rule.max_chars
    if max_chars > 0 and len(text) > max_chars:
        candidate = text[:max_chars]
        sentence_end = max(candidate.rfind("."), candidate.rfind("!"), candidate.rfind("?"))
        if sentence_end >= max(120, max_chars // 2):
            candidate = candidate[: sentence_end + 1]
        else:
            candidate = candidate.rstrip()
        text = f"{candidate} Full response is on screen."
    return text.strip()

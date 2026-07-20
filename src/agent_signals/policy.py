from __future__ import annotations

import json
import re
from pathlib import Path

from agent_signals.config import ChannelConfig, PolicyConfig, VoiceConfig, VoiceRule
from agent_signals.models import AgentEvent, Severity

_FALLBACK_RULES: dict = {
    "closing_window_chars": 300,
    "needs_input_patterns": [
        r"\?\s*$",
        r"(?i)\b(please confirm|which (one|option|approach)|do you want|would you like|want me to|shall i|need your (input|decision|approval|sign-?off)|waiting for your|your call|up to you|let me know)\b",
    ],
    "error_patterns": [
        r"(?i)\b(failed|failure|error|traceback|exception|permission denied|fatal)\b",
    ],
}
_RULES_PATHS = [
    Path.home() / ".config" / "agent-signals" / "severity-rules.json",
    Path(__file__).resolve().parents[2] / "config" / "severity-rules.json",
]


def _load_severity_rules() -> dict:
    for path in _RULES_PATHS:
        try:
            data = json.loads(path.read_text())
        except Exception:
            continue
        if isinstance(data, dict) and data.get("needs_input_patterns"):
            return {
                "closing_window_chars": int(data.get("closing_window_chars", 300)),
                "needs_input_patterns": list(data.get("needs_input_patterns", [])),
                "error_patterns": list(data.get("error_patterns", [])),
            }
    return _FALLBACK_RULES


def classify_severity(text: str) -> str:
    rules = _load_severity_rules()
    window = text.strip()[-rules["closing_window_chars"]:]
    for pattern in rules["error_patterns"]:
        try:
            if re.search(pattern, window):
                return Severity.ERROR
        except re.error:
            continue
    for pattern in rules["needs_input_patterns"]:
        try:
            if re.search(pattern, window):
                return Severity.NEEDS_INPUT
        except re.error:
            continue
    return Severity.NORMAL


def classify_voice_rule(text: str, config: VoiceConfig) -> VoiceRule:
    default = config.default_rule()
    for rule in config.rules:
        for pattern in rule.patterns:
            try:
                if re.search(pattern, text):
                    return rule
            except re.error:
                continue
    return default


def looks_like_json(text: str) -> bool:
    """True when ``text`` is a standalone JSON object or array — the whole
    message is machine output, not prose. Prose that merely embeds a snippet
    won't parse cleanly and is left alone."""
    stripped = text.strip()
    if not (stripped.startswith("{") or stripped.startswith("[")):
        return False
    try:
        value = json.loads(stripped)
    except Exception:
        return False
    return isinstance(value, (dict, list))


def last_assistant_message(event: AgentEvent) -> str:
    """The agent's final message for this turn. Codex delivers it inline on the
    hook payload (``last-assistant-message``); Claude Code leaves it in the
    transcript, which we read back."""
    raw = event.raw if isinstance(event.raw, dict) else {}
    inline = str(raw.get("last-assistant-message") or "").strip()
    if inline:
        return inline
    from agent_signals.transcripts import extract_last_response
    text, _ = extract_last_response(event.transcript_path)
    return text or ""


def is_suppressed_turn(event: AgentEvent) -> bool:
    """Suppress turn-completion notifications whose entire response is a JSON
    blob (e.g. Codex Desktop's internal title-generation turns). ``Notification``
    events — permission prompts and "waiting for input" — always surface."""
    if event.event_type == "Notification":
        return False
    return looks_like_json(last_assistant_message(event))


def channels_for_severity(severity: str, policy: PolicyConfig | None = None) -> ChannelConfig:
    if policy is None:
        policy = PolicyConfig()
    return policy.for_severity(severity)

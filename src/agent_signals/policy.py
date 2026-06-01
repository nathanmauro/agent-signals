from __future__ import annotations

import json
import re

from agent_signals.config import ChannelConfig, PolicyConfig, VoiceConfig, VoiceRule
from agent_signals.models import AgentEvent, Severity

ERROR_PATTERNS = [
    r"(?i)\b(blocked|failed|failure|error|traceback|exception|permission denied|unable to|could not|can't)\b",
]
NEEDS_INPUT_PATTERNS = [
    r"\?\s*$",
    r"(?i)\b(please confirm|which one|choose|send me|tell me|do you want)\b",
]


def classify_severity(text: str) -> str:
    for pattern in ERROR_PATTERNS:
        try:
            if re.search(pattern, text):
                return Severity.ERROR
        except re.error:
            continue
    for pattern in NEEDS_INPUT_PATTERNS:
        try:
            if re.search(pattern, text):
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

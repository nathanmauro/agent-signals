from __future__ import annotations

import re

from agent_signals.config import ChannelConfig, PolicyConfig, VoiceConfig, VoiceRule
from agent_signals.models import Severity

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


def channels_for_severity(severity: str, policy: PolicyConfig | None = None) -> ChannelConfig:
    if policy is None:
        policy = PolicyConfig()
    return policy.for_severity(severity)

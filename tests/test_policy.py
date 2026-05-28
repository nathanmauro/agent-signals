from pathlib import Path

from agent_signals.config import VoiceConfig
from agent_signals.policy import classify_severity, classify_voice_rule

FIXTURES = Path(__file__).parent / "fixtures"


def test_classify_normal():
    assert classify_severity("I've updated the file.") == "normal"


def test_classify_error():
    assert classify_severity("The build failed with exit code 1.") == "error"
    assert classify_severity("Permission denied accessing /etc/hosts") == "error"
    assert classify_severity("Traceback (most recent call last):") == "error"


def test_classify_needs_input():
    assert classify_severity("Which approach would you prefer?") == "needs_input"
    assert classify_severity("Please confirm you want to proceed.") == "needs_input"


def test_classify_voice_rule_default():
    config = VoiceConfig.load(Path(__file__).parent.parent / "config" / "voice-rules.json")
    rule = classify_voice_rule("The sky is blue today.", config)
    assert rule.name == "default"


def test_classify_voice_rule_needs_attention():
    config = VoiceConfig.load(Path(__file__).parent.parent / "config" / "voice-rules.json")
    rule = classify_voice_rule("The build failed.", config)
    assert rule.name == "needs_attention"
    assert rule.prefix == "Needs attention. "


def test_classify_voice_rule_code():
    config = VoiceConfig.load(Path(__file__).parent.parent / "config" / "voice-rules.json")
    rule = classify_voice_rule("Here's the change:\n```python\nprint('hi')\n```", config)
    assert rule.name == "code_or_patch"

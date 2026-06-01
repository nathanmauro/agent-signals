from pathlib import Path

from agent_signals.config import VoiceConfig
from agent_signals.models import AgentEvent
from agent_signals.policy import (
    classify_severity,
    classify_voice_rule,
    is_suppressed_turn,
    looks_like_json,
)

FIXTURES = Path(__file__).parent / "fixtures"


def _turn(message: str, event_type: str = "agent-turn-complete") -> AgentEvent:
    return AgentEvent(
        client="Codex Desktop",
        event_type=event_type,
        raw={"last-assistant-message": message},
    )


def test_looks_like_json_distinguishes_blob_from_prose():
    assert looks_like_json('{"title":"Create ACP branch"}')
    assert looks_like_json('["a", "b"]')
    assert not looks_like_json("Created the files. No tests run.")
    assert not looks_like_json("")
    # Starts with a brace but isn't valid JSON → treated as prose.
    assert not looks_like_json("{this is not json, just a note")


def test_suppresses_json_only_turn_completion():
    assert is_suppressed_turn(_turn('{"title":"Create Claude Code ACP branch"}'))


def test_keeps_prose_turn_completion():
    assert not is_suppressed_turn(_turn("Created the files you asked for."))


def test_keeps_notification_even_when_message_is_jsonish():
    assert not is_suppressed_turn(_turn('{"x":1}', event_type="Notification"))


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

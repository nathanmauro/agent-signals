import json
from pathlib import Path
from unittest.mock import patch

from agent_signals.clients.parse import parse_hook_payload
from agent_signals.models import MuxContext

FIXTURES = Path(__file__).parent / "fixtures"


@patch("agent_signals.mux.detect.detect_mux", return_value=MuxContext())
def test_parse_claude_stop(mock_mux):
    payload = (FIXTURES / "claude_stop.json").read_text()
    event = parse_hook_payload(label="Claude Code", raw=payload)
    assert event is not None
    assert event.client == "Claude Code"
    assert event.thread_id == "thread_abc123"
    assert event.turn_id == "turn_001"
    assert event.cwd_name == "test"


@patch("agent_signals.mux.detect.detect_mux", return_value=MuxContext())
def test_parse_codex_turn(mock_mux):
    payload = (FIXTURES / "codex_turn_ended.json").read_text()
    event = parse_hook_payload(raw=payload)
    assert event is not None
    assert event.client == "Codex"
    assert event.thread_id == "sess_xyz789"


@patch("agent_signals.mux.detect.detect_mux", return_value=MuxContext())
def test_parse_irrelevant_event(mock_mux):
    event = parse_hook_payload(raw='{"type": "tool_call"}')
    assert event is None


@patch("agent_signals.mux.detect.detect_mux", return_value=MuxContext())
def test_parse_empty_payload(mock_mux):
    event = parse_hook_payload(label="Claude Code", raw="")
    assert event is not None
    assert event.client == "Claude Code"

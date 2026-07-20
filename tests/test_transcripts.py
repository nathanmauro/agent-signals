from pathlib import Path

from agent_signals.transcripts.claude import (
    extract_previous_turn_from_claude,
    extract_text_from_claude,
)
from agent_signals.transcripts.codex import extract_text_from_codex

FIXTURES = Path(__file__).parent / "fixtures"


def test_extract_claude_transcript():
    text, key = extract_text_from_claude(FIXTURES / "claude_transcript.jsonl")
    assert "found and fixed the bug" in text
    assert key == "resp_001"


def test_extract_codex_transcript():
    text, key = extract_text_from_codex(FIXTURES / "codex_transcript.jsonl")
    assert "updated the configuration" in text


def test_extract_missing_file():
    text, key = extract_text_from_claude(FIXTURES / "nonexistent.jsonl")
    assert text == ""


def test_extract_empty_file(tmp_path: Path):
    empty = tmp_path / "empty.jsonl"
    empty.write_text("")
    text, key = extract_text_from_claude(empty)
    assert text == ""


def test_previous_turn_skips_inflight_turn():
    text, key = extract_previous_turn_from_claude(FIXTURES / "claude_transcript_inflight.jsonl")
    assert "found and fixed the bug" in text
    assert key == "resp_003"


def test_previous_turn_skips_prior_say_turns():
    text, key = extract_previous_turn_from_claude(FIXTURES / "claude_transcript_repeat_say.jsonl")
    assert "found and fixed the bug" in text
    assert key == "resp_001"


def test_previous_turn_empty_file(tmp_path: Path):
    empty = tmp_path / "empty.jsonl"
    empty.write_text("")
    text, key = extract_previous_turn_from_claude(empty)
    assert text == ""


def test_compact_summary_is_not_a_user_prompt():
    from agent_signals.transcripts.claude import _is_turn_boundary, _user_prompt_text

    compact = {
        "type": "user",
        "isCompactSummary": True,
        "message": {"content": "This session is being continued. Would you like me to continue?"},
    }
    assert _user_prompt_text(compact) is None
    assert _is_turn_boundary(compact)


def test_previous_turn_skips_compact_summary_prompt():
    from agent_signals.transcripts.claude import extract_previous_turn_from_claude

    path = FIXTURES / "claude_transcript_compact.jsonl"
    text, key = extract_previous_turn_from_claude(path)
    assert text == "Continuing after compaction. The refactor is done and tests pass."
    assert key == "resp_021"

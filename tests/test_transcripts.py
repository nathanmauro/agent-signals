from pathlib import Path

from agent_signals.transcripts.claude import extract_text_from_claude
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

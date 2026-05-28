from agent_signals.config import VoiceRule
from agent_signals.tts.speech_text import make_speakable, strip_memory_citation


def test_strip_memory_citation():
    text = "Hello world<oai-mem-citation>blah blah</oai-mem-citation>"
    assert strip_memory_citation(text) == "Hello world"


def test_strip_code_blocks():
    rule = VoiceRule()
    text = "Here's the fix:\n```python\ndef foo(): pass\n```\nThat should work."
    result = make_speakable(text, rule)
    assert "```" not in result
    assert "def foo" not in result
    assert "That should work" in result


def test_code_only_response():
    rule = VoiceRule()
    text = "```python\ndef foo(): pass\n```"
    result = make_speakable(text, rule)
    assert result == "Code details are on screen."


def test_strip_markdown_links():
    rule = VoiceRule()
    text = "Check [the docs](https://example.com/docs) for more info."
    result = make_speakable(text, rule)
    assert "the docs" in result
    assert "https://" not in result
    assert "[" not in result


def test_strip_file_paths():
    rule = VoiceRule()
    text = "I edited /Users/nathan/Developer/proj/agent-signals/cli.py"
    result = make_speakable(text, rule)
    assert "cli.py" in result
    assert "/Users/nathan" not in result


def test_prefix():
    rule = VoiceRule(prefix="Needs attention. ")
    text = "The build failed with exit code 1."
    result = make_speakable(text, rule)
    assert result.startswith("Needs attention. ")


def test_max_chars_truncation():
    rule = VoiceRule(max_chars=50)
    text = "This is a long response. " * 10
    result = make_speakable(text, rule)
    assert "Full response is on screen." in result


def test_sentence_boundary_truncation():
    rule = VoiceRule(max_chars=80)
    text = "First sentence here. Second sentence here. Third really long sentence that goes on."
    result = make_speakable(text, rule)
    assert result.endswith("Full response is on screen.")

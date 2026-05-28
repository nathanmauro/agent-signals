from pathlib import Path

from agent_signals.models import MuxContext, NotifyPayload
from agent_signals.state import DeduplicationStore, VoiceState


def test_dedup_first_is_not_duplicate(tmp_path: Path):
    store = DeduplicationStore(state_dir=tmp_path, window=20)
    assert not store.is_duplicate("key1")


def test_dedup_second_is_duplicate(tmp_path: Path):
    store = DeduplicationStore(state_dir=tmp_path, window=20)
    store.is_duplicate("key1")
    assert store.is_duplicate("key1")


def test_dedup_different_keys(tmp_path: Path):
    store = DeduplicationStore(state_dir=tmp_path, window=20)
    store.is_duplicate("key1")
    assert not store.is_duplicate("key2")


def test_save_context(tmp_path: Path):
    store = DeduplicationStore(state_dir=tmp_path)
    payload = NotifyPayload(title="test", key="abc", mux=MuxContext(type="zellij"))
    path = store.save_context("abc", payload)
    assert path.exists()
    import json
    data = json.loads(path.read_text())
    assert data["title"] == "test"
    assert data["mux"]["type"] == "zellij"


def test_voice_state_toggle(tmp_path: Path):
    vs = VoiceState(state_dir=tmp_path)
    assert not vs.enabled
    vs.enabled = True
    assert vs.enabled
    vs.enabled = False
    assert not vs.enabled


def test_voice_state_duplicate_key(tmp_path: Path):
    vs = VoiceState(state_dir=tmp_path)
    assert not vs.is_duplicate_key("k1")
    vs.save_last_key("k1")
    assert vs.is_duplicate_key("k1")
    assert not vs.is_duplicate_key("k2")


def test_voice_state_status(tmp_path: Path):
    vs = VoiceState(state_dir=tmp_path)
    assert vs.status_line() == "voice: off"
    vs.enabled = True
    assert vs.status_line() == "voice: on"

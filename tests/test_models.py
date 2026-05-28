from agent_signals.models import AgentEvent, MuxContext, NotifyPayload, clean_text


def test_clean_text_strips_ansi():
    assert clean_text("\x1b[31mred\x1b[0m text") == "red text"


def test_clean_text_strips_braille():
    assert clean_text("⠀ hello") == "hello"


def test_clean_text_collapses_whitespace():
    assert clean_text("  hello   world  ") == "hello world"


def test_mux_context_roundtrip():
    ctx = MuxContext(type="zellij", session="main", pane_ref="terminal_12", tab_name="dev")
    d = ctx.to_dict()
    assert d["type"] == "zellij"
    assert "window_id" not in d
    ctx2 = MuxContext.from_dict(d)
    assert ctx2.type == "zellij"
    assert ctx2.pane_ref == "terminal_12"


def test_agent_event_dedup_key():
    e1 = AgentEvent(client="Claude Code", thread_id="t1", turn_id="turn1")
    e2 = AgentEvent(client="Claude Code", thread_id="t1", turn_id="turn1")
    assert e1.dedup_key() == e2.dedup_key()

    e3 = AgentEvent(client="Claude Code", thread_id="t1", turn_id="turn2")
    assert e1.dedup_key() != e3.dedup_key()


def test_agent_event_cwd_name():
    e = AgentEvent(client="Claude Code", cwd="/Users/nathan/Developer/proj/agent-signals")
    assert e.cwd_name == "agent-signals"

    e2 = AgentEvent(client="Claude Code")
    assert e2.cwd_name == ""


def test_notify_payload_roundtrip():
    p = NotifyPayload(
        title="Claude Code responded",
        subtitle="main / dev",
        message="agent-signals is ready.",
        key="abc123",
        mux=MuxContext(type="zellij", session="main"),
    )
    d = p.to_dict()
    p2 = NotifyPayload.from_dict(d)
    assert p2.title == p.title
    assert p2.mux.type == "zellij"
    assert p2.mux.session == "main"

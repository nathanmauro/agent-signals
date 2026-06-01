from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path


def cmd_notify(args: argparse.Namespace) -> None:
    payload_str = args.payload
    if not payload_str and not sys.stdin.isatty():
        payload_str = sys.stdin.read()

    from agent_signals.clients.parse import parse_hook_payload
    event = parse_hook_payload(label=args.client or "", raw=payload_str or "")
    if event is None:
        return

    from agent_signals.policy import is_suppressed_turn
    if is_suppressed_turn(event):
        return

    from agent_signals.models import NotifyPayload
    title = f"{event.client} responded"
    subtitle = ""
    message = "Prompt response is ready."
    mux = event.mux

    if mux.type == "zellij":
        session = mux.session or "zellij"
        tab = mux.tab_name or f"tab {mux.tab_position}"
        pane_title = mux.pane_title or event.cwd_name or mux.pane_ref
        subtitle = f"{session} / {tab}"
        message = f"{pane_title} ({mux.pane_ref}) is ready. Click to focus."
    elif mux.type == "tmux":
        session = mux.session or "tmux"
        window = mux.window_name or mux.window_index or "window"
        pane_title = mux.pane_title or event.cwd_name or mux.pane_id
        subtitle = f"{session} / {window}"
        message = f"{pane_title} ({mux.pane_id}) is ready. Click to focus."
    elif event.cwd_name:
        message = f"{event.cwd_name}: response is ready."

    key = event.dedup_key()
    payload = NotifyPayload(
        title=title,
        subtitle=subtitle,
        message=message,
        key=key,
        mux=mux,
    )

    if args.dry_run:
        import json
        print(json.dumps(payload.to_dict()))
        return

    from agent_signals.state import DeduplicationStore
    store = DeduplicationStore()
    if store.is_duplicate(key):
        return
    context_file = store.save_context(key, payload)

    from agent_signals.policy import channels_for_severity, classify_severity
    from agent_signals.transcripts import extract_last_response
    text, _ = extract_last_response(event.transcript_path)
    severity = classify_severity(text) if text else "normal"
    channels = channels_for_severity(severity)

    from agent_signals.channels.dispatch import dispatch_notification
    dispatch_notification(payload, channels, context_file=context_file)


def cmd_focus(args: argparse.Namespace) -> None:
    from agent_signals.focus import focus_pane
    focus_pane(args.context)


def cmd_speak_last(args: argparse.Namespace) -> None:
    payload_str = ""
    if not sys.stdin.isatty():
        payload_str = sys.stdin.read()

    from agent_signals.state import VoiceState
    voice_state = VoiceState()

    if not args.dry_run and not voice_state.enabled:
        return

    transcript_path = ""
    if payload_str.strip():
        import json
        try:
            data = json.loads(payload_str)
            transcript_path = str(
                data.get("transcript_path")
                or data.get("transcriptPath")
                or data.get("session_path")
                or data.get("sessionPath")
                or ""
            )
        except Exception:
            pass

    from agent_signals.transcripts import extract_last_response
    text, key = extract_last_response(transcript_path)
    if not text:
        return

    from agent_signals.config import VoiceConfig
    rules_path = Path(os.environ.get(
        "AGENT_VOICE_RULES",
        str(Path(__file__).resolve().parent.parent.parent / "config" / "voice-rules.json"),
    ))
    if not rules_path.exists():
        rules_path = Path.home() / "Developer" / "config" / "voice" / "config" / "rules.json"
    config = VoiceConfig.load(rules_path if rules_path.exists() else None)

    from agent_signals.policy import classify_voice_rule
    from agent_signals.tts.speech_text import make_speakable, strip_memory_citation
    rule = classify_voice_rule(strip_memory_citation(text), config)
    speech_text = make_speakable(text, rule)
    if not speech_text:
        return

    if args.dry_run:
        import json
        print(json.dumps({
            "key": key,
            "text": speech_text,
            "voice": rule.voice,
            "speed": rule.speed,
            "lang": rule.lang,
            "rule": rule.name,
        }))
        return

    if voice_state.is_duplicate_key(key):
        return
    voice_state.save_last_key(key)

    from agent_signals.tts.playback import stop_playback
    stop_playback(voice_state)

    voice_state.text_file.write_text(speech_text)

    from agent_signals.tts.kokoro import speak_kokoro
    player = speak_kokoro(
        text=speech_text,
        out_file=voice_state.wav_file,
        voice=rule.voice,
        speed=rule.speed,
        lang=rule.lang,
    )
    if player:
        voice_state.save_pid(player.pid)


def cmd_voice(args: argparse.Namespace) -> None:
    from agent_signals.state import VoiceState
    voice_state = VoiceState()

    action = args.action
    if action == "on":
        voice_state.enabled = True
        print("voice: on")
    elif action == "off":
        voice_state.enabled = False
        from agent_signals.tts.playback import stop_playback
        stop_playback(voice_state)
        print("voice: off")
    elif action == "stop":
        from agent_signals.tts.playback import stop_playback
        stop_playback(voice_state)
        print("voice: stopped")
    else:
        print(voice_state.status_line())


def cmd_doctor(args: argparse.Namespace) -> None:
    import shutil
    from agent_signals.config import DEFAULT_KOKORO_ROOT, DEFAULT_STATE_DIR, DEFAULT_VOICE_STATE_DIR

    checks: list[tuple[str, bool, str]] = []

    notifier = shutil.which("terminal-notifier")
    checks.append(("terminal-notifier", notifier is not None, notifier or "not found"))

    zellij = shutil.which("zellij")
    checks.append(("zellij", zellij is not None, zellij or "not found"))

    kokoro_model = DEFAULT_KOKORO_ROOT / "kokoro-v1.0.onnx"
    checks.append(("kokoro model", kokoro_model.exists(), str(kokoro_model)))

    kokoro_python = DEFAULT_KOKORO_ROOT / ".venv" / "bin" / "python"
    checks.append(("kokoro venv", kokoro_python.exists(), str(kokoro_python)))

    checks.append(("state dir", DEFAULT_STATE_DIR.exists(), str(DEFAULT_STATE_DIR)))
    checks.append(("voice state dir", DEFAULT_VOICE_STATE_DIR.exists(), str(DEFAULT_VOICE_STATE_DIR)))

    from agent_signals.state import VoiceState
    vs = VoiceState()
    checks.append(("voice enabled", vs.enabled, str(vs.enabled)))

    zellij_pane = os.environ.get("ZELLIJ_PANE_ID", "")
    checks.append(("ZELLIJ_PANE_ID", bool(zellij_pane), zellij_pane or "(not set)"))

    tmux_pane = os.environ.get("TMUX_PANE", "")
    checks.append(("TMUX_PANE", bool(tmux_pane), tmux_pane or "(not set)"))

    for name, ok, detail in checks:
        status = "ok" if ok else "MISSING"
        print(f"  {status:>7}  {name}: {detail}")


def main() -> None:
    parser = argparse.ArgumentParser(prog="agent-signal", description="AI agent signal infrastructure")
    parser.add_argument("--dry-run", action="store_true", help="Print output without dispatching")
    sub = parser.add_subparsers(dest="command")

    p_notify = sub.add_parser("notify", help="Dispatch notification for agent stop event")
    p_notify.add_argument("--client", default="", help="Client label (e.g. 'Claude Code')")
    p_notify.add_argument("payload", nargs="?", default="", help="JSON hook payload")
    p_notify.set_defaults(func=cmd_notify)

    p_focus = sub.add_parser("focus", help="Activate terminal and focus mux pane")
    p_focus.add_argument("context", help="Path to context JSON file")
    p_focus.set_defaults(func=cmd_focus)

    p_speak = sub.add_parser("speak-last", help="Speak last assistant response via TTS")
    p_speak.set_defaults(func=cmd_speak_last)

    p_voice = sub.add_parser("voice", help="Control voice playback state")
    p_voice.add_argument("action", nargs="?", default="status", choices=["on", "off", "stop", "status"])
    p_voice.set_defaults(func=cmd_voice)

    p_doctor = sub.add_parser("doctor", help="Check system dependencies and state")
    p_doctor.set_defaults(func=cmd_doctor)

    args = parser.parse_args()
    if not hasattr(args, "func"):
        parser.print_help()
        sys.exit(2)
    args.func(args)


if __name__ == "__main__":
    main()

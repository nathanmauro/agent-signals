from __future__ import annotations

import shutil
from pathlib import Path

from agent_signals.config import ChannelConfig, DEFAULT_SOUND
from agent_signals.models import NotifyPayload


def _resolve_focus_command(context_file: Path) -> str | None:
    exe = shutil.which("agent-signal")
    if not exe:
        return None
    return f"{exe} focus {context_file}"


def dispatch_notification(
    payload: NotifyPayload,
    channels: ChannelConfig,
    context_file: Path | None = None,
    sound_file: Path = DEFAULT_SOUND,
) -> None:
    if channels.bell:
        from agent_signals.channels.visual_bell import send_bell
        send_bell()

    if channels.sound:
        from agent_signals.channels.sound import play_sound
        play_sound(sound_file)

    if channels.notify:
        from agent_signals.channels.macos import send_notification
        focus_cmd = _resolve_focus_command(context_file) if context_file else None
        send_notification(
            payload,
            ignore_dnd=channels.notify_ignore_dnd,
            focus_command=focus_cmd,
        )

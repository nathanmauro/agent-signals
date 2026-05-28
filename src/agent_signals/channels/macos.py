from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

from agent_signals.models import NotifyPayload


def send_notification(
    payload: NotifyPayload,
    ignore_dnd: bool = False,
    focus_command: str | None = None,
) -> None:
    notifier = shutil.which("terminal-notifier")
    if notifier:
        _send_terminal_notifier(notifier, payload, ignore_dnd, focus_command)
    else:
        _send_osascript(payload)


def _send_terminal_notifier(
    notifier: str,
    payload: NotifyPayload,
    ignore_dnd: bool,
    focus_command: str | None,
) -> None:
    args = [
        notifier,
        "-title", payload.title,
        "-subtitle", payload.subtitle,
        "-message", payload.message,
        "-group", payload.key,
        "-sender", "com.mitchellh.ghostty",
    ]
    if focus_command:
        args.extend(["-execute", focus_command])
    if ignore_dnd:
        args.append("-ignoreDnD")
    subprocess.Popen(
        args,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def _send_osascript(payload: NotifyPayload) -> None:
    subprocess.Popen(
        [
            "/usr/bin/osascript",
            "-e", "on run argv",
            "-e", 'display notification (item 2 of argv) with title (item 1 of argv) subtitle (item 3 of argv)',
            "-e", "end run",
            payload.title,
            payload.message,
            payload.subtitle,
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

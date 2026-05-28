from __future__ import annotations

import os
import subprocess


def send_bell() -> bool:
    pid = os.getpid()
    while pid and pid != 1:
        try:
            proc = subprocess.run(
                ["ps", "-o", "tty=", "-p", str(pid)],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                check=False,
            )
            tty = proc.stdout.strip()
        except Exception:
            tty = ""
        if tty and tty != "??":
            try:
                with open(f"/dev/{tty}", "w") as f:
                    f.write("\a")
                return True
            except OSError:
                pass
        try:
            proc = subprocess.run(
                ["ps", "-o", "ppid=", "-p", str(pid)],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                check=False,
            )
            pid = int(proc.stdout.strip())
        except (ValueError, Exception):
            break
    return False

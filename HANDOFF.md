# agent-signals — Handoff

Unified notification / voice / focus infrastructure for AI coding agents (Claude Code, Codex). When an agent finishes a turn, this fires a bell + sound + native macOS notification; clicking the notification focuses the originating terminal pane.

> **Not `agent-observatory`.** That sibling project (a Go process-monitor dashboard) had the Textual-TUI CPU runaway on 2026-05-27 that contributed to a WindowServer crash. **agent-signals has never used Textual** and is unrelated to that incident. Its daemon is a flat ~10 MB Rust process.

## Architecture (native rewrite, as of 2026-05-28)

Three processes, mostly Rust; Python retained only for TTS.

```
hook → agent-signal (Rust client)
          │  Unix socket: ~/.local/state/agent-signals/agent-signald.sock
          ▼
       agent-signald (Rust daemon, LaunchAgent, KeepAlive)
          │  dedup (20s) · context store · active-index (cap 32) · spool (cap 128) · 24h sweep
          │  persistent socket
          ▼
       AgentSignalsNotifier.app (Swift, LaunchAgent) — UNUserNotificationCenter
          │  on click → daemon → focus_context → activate Ghostty + zellij/tmux pane focus
```

- `speak-last` and `voice` shell out to the Python package (`python -m agent_signals.cli`). Everything else is native.
- Native notifications replaced `terminal-notifier` — this killed the `-sender`/`-execute`/`-ignoreDnD` conflicts and the per-notification process pile-up. `error` severity → `.timeSensitive` (DND override); `needs_input` and `normal` are routine.
- Resilience: daemon down → client spools + plays local cues itself. Notifier down → daemon spools, drains on reconnect.

## Source layout

- `native/rust/src/lib.rs` — the entire daemon + client (one file). Bins: `agent-signal`, `agent-signald`.
- `native/swift/AgentSignalsNotifier/main.swift` — notifier helper (socket client + UNUserNotificationCenter).
- `src/agent_signals/` — Python package: TTS/voice (Kokoro) + a parallel legacy CLI.
- `bin/` — hook wrappers (`agent-response-notify`, `agent-focus-pane`) → call `agent-signal`.
- `launchd/` — the two LaunchAgent plists. `scripts/install-native.sh` — build + install + load + doctor.

## Current status (verified 2026-05-28)

- ✅ Built, installed; both LaunchAgents running. Rust 7/7, Python 36/36 green.
- ✅ Daemon healthy, notifier connected, processing live traffic.
- ⚠️ **Notification authorization = DENIED.** The pipeline completes ("posted") and bell + sound fire, but macOS suppresses the visible banner. **This is the one thing between "running" and "you actually see a notification."**

## Plan / open items (priority order)

1. **Fix notification auth (last mile).** `agent-signal doctor` reports `denied`. Allow "AgentSignalsNotifier" in System Settings → Notifications, or `tccutil reset Notifications` + relaunch the app to re-trigger the prompt. Confirm with `agent-signal doctor` → `authorized`.
2. **Git remote.** Repo is local-only as of this commit. Decide public vs private, then `gh repo create` + push. (Standing action: "enable remote session access.")
3. **`dedup/` growth.** The `dedup/` dir is uncapped by count; the sweep only runs at daemon startup. Add a count cap and/or a periodic sweep so it can't accumulate between restarts. Harmless today (~10 bytes/file) but it's the one unbounded path.
4. **Legacy state cleanup.** ~42 old hash files linger in the state-dir root from the pre-reorg era. `agent-signal sweep --legacy` clears them.
5. **Teardown.** Confirm the old `~/Developer/config/voice/bin/` scripts are fully superseded by the symlinks into this repo.

## Commands

```bash
scripts/install-native.sh                 # build + install + load LaunchAgents + doctor
agent-signal doctor --verbose             # health: socket, notifier, auth
agent-signal sweep --legacy --dry-run     # preview state cleanup
cd native/rust && cargo test              # Rust suite (7 tests)
.venv/bin/python -m pytest -q             # Python TTS suite (36 tests)
```

## State / paths

- State: `~/.local/state/agent-signals/` — `agent-signald.sock`, `contexts/`, `dedup/`, `spool/`, `*.log`
- Bins: `~/.local/bin/agent-signal`, `~/.local/bin/agent-signald`
- App: `~/Applications/AgentSignalsNotifier.app`
- LaunchAgents: `~/Library/LaunchAgents/com.nathan.agent-signald.plist`, `com.nathan.AgentSignalsNotifier.plist`
- Voice state: `~/.local/state/agent-voice/`

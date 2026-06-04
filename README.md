# Agent Signals

Unified notification, voice, and focus infrastructure for AI coding agents (Claude Code, Codex) on macOS. When an agent finishes a turn — or stops to ask for input — Agent Signals fires a native macOS notification, plus a terminal bell and sound. Clicking the notification jumps you straight back to the exact terminal pane that produced it (Ghostty + zellij/tmux).

It exists to close the loop on long-running agent work: kick off a task, switch contexts, and get pulled back the moment the agent needs you — into the right pane, not a guessing game across a dozen splits.

## Features

- **Native macOS notifications** via `UNUserNotificationCenter` — no third-party CLI, no per-notification process pile-up.
- **Click-to-focus** — clicking a notification activates Ghostty and selects the originating zellij/tmux pane (and tab/window).
- **Pane-scoped grouping** — routine turn-complete notifications replace the latest card for that pane; needs-input notifications can stack distinctly inside the same session/window/pane group.
- **Severity-aware delivery** — `error` notifications are time-sensitive and break through Do Not Disturb; routine turn-completion and input prompts stay quiet.
- **Audible cues** — terminal bell plus sound alongside the visual notification.
- **Optional voice (TTS)** — speak the agent's last message aloud via local Kokoro text-to-speech.
- **Resilient by design** — the pipeline spools events and falls back to local cues when any process is down, then drains the backlog on reconnect.
- **Self-maintaining** — a long-lived daemon dedupes bursts, expires stale context, and garbage-collects its own state on a schedule.
- **Zero-cloud** — everything runs locally over a Unix socket; no network calls, no telemetry.

## Architecture

Agent Signals is three cooperating processes connected by a Unix socket:

```
┌──────────────┐      ┌────────────────────┐      ┌──────────────────────┐
│  hook fires  │─────▶│   agent-signal     │─────▶│   agent-signald      │
│ (Stop, etc.) │  cli │  (Rust client)     │ sock │   (Rust daemon)      │
└──────────────┘      └────────────────────┘      └──────────┬───────────┘
                                                             │ persistent socket
                                                             ▼
                                              ┌──────────────────────────┐
                                              │  AgentSignalsNotifier.app │
                                              │       (Swift)             │
                                              │  UNUserNotificationCenter │
                                              └──────────────────────────┘
```

**Flow.** An agent hook invokes `agent-signal notify`. The client writes one JSON line to the daemon's Unix socket at `~/.local/state/agent-signals/agent-signald.sock` and reads back an ack. The daemon (`agent-signald`) dedupes, saves the focus context, and forwards a post request to the Swift notifier, which posts the actual `UNUserNotificationCenter` notification. When you click it, the notifier calls back to the daemon, which looks up the saved context and focuses the right Ghostty window plus zellij/tmux pane.

**The daemon owns the durable state:**

- **Dedup** — identical events inside a 20s window collapse into one.
- **Context store** — focus targets persist with a 24h TTL.
- **Active index** — the most recent contexts (capped at 32) are kept hot for fast focus.
- **Spool** — up to 128 events are queued while the notifier is unreachable, and drained automatically when it reconnects.
- **Periodic sweep** — every 6h the daemon GCs expired contexts, stale dedup entries, and legacy state on a dedicated thread.

**Resilience.** If the daemon is down, the client spools the event locally and still plays a bell and sound so you get a cue. If the notifier is down, the daemon spools and drains on reconnect. No single process failure drops a signal silently.

**Severity mapping:**

| Severity | Interruption level | Behavior |
|---|---|---|
| `error` | `.timeSensitive` | Breaks through Do Not Disturb |
| `needs_input` | default | Routine notification |
| `normal` | default | Routine notification |

**Why native.** This pipeline replaced a `terminal-notifier` fan-out. That approach spawned a process per notification and forced brittle `-sender` / `-execute` / `-ignoreDnD` flag juggling. The native daemon plus Swift app eliminates the process pile-up and the flag conflicts, and gives precise control over interruption levels and click handling.

The Python package (`src/agent_signals`) is retained **only** for Kokoro TTS (the voice / speak-last features). Everything else is native Rust and Swift.

## Requirements

- **macOS** (uses `UNUserNotificationCenter`, AppKit, and LaunchAgents).
- **Rust toolchain** (stable, edition 2021) to build the client and daemon.
- **Python 3.11+** — *optional*, only for the Kokoro TTS voice features.
- Ghostty plus zellij or tmux for click-to-focus (notifications still work without them).

## Install

```bash
./scripts/install-native.sh
```

The installer:

1. Builds the Rust workspace in release mode (`agent-signal` + `agent-signald`).
2. Builds the Swift notifier app (`AgentSignalsNotifier.app`).
3. Installs the binaries into `~/.local/bin` and the app into `~/Applications`.
4. Installs the hook wrappers from `bin/` (`agent-response-notify`, `agent-focus-pane`, and the voice helpers).
5. Writes and loads the LaunchAgents for the daemon and notifier (both `KeepAlive`).
6. Runs a dry-run sweep and `agent-signal doctor --verbose` to report status.

It is idempotent — safe to re-run; existing files are overwritten in place.

### First run

macOS requires you to authorize notifications once. The first notification triggers an authorization prompt for `AgentSignalsNotifier`. Until it is granted, the pipeline still completes (the daemon reports `posted`, and the bell and sound fire) but macOS suppresses the visible banner. Grant it here:

> **System Settings → Notifications → AgentSignalsNotifier**

This is the only manual step. If the prompt never appeared, reset and relaunch:

```bash
tccutil reset Notifications
```

Verify the full pipeline with:

```bash
agent-signal doctor --verbose
```

`doctor` prints a remediation hint whenever notification authorization is `denied`.

## Hook wiring

Agent Signals is driven by your agent's hooks. For Claude Code, add the `Stop` and `Notification` hooks to `~/.claude/settings.json` so both turn-completion and input prompts route through `agent-response-notify`:

```json
{
  "hooks": {
    "Stop": [
      { "matcher": "", "hooks": [ { "type": "command", "command": "agent-response-notify" } ] }
    ],
    "Notification": [
      { "matcher": "", "hooks": [ { "type": "command", "command": "agent-response-notify" } ] }
    ]
  }
}
```

`agent-response-notify` reads the hook payload on stdin and calls `agent-signal notify`, which builds the notification (title, pane subtitle, severity) and hands it to the daemon.

## Usage

Once installed, signals flow automatically from your agent hooks. The `agent-signal` CLI is the manual entry point:

```bash
# Full health check: binaries, LaunchAgents, socket, notifier, notification auth
agent-signal doctor --verbose

# Fire a notification by hand from a hook-style JSON payload (handy for testing)
agent-signal notify --client "Claude Code" '{"type":"Stop","cwd":"/path/to/project"}'

# Preview a maintenance sweep, including pre-native cruft, without deleting anything
agent-signal sweep --legacy --dry-run

# Clear Agent Signals delivered notifications plus active/spooled local notification state
agent-signal clear

# Re-focus a saved pane context on demand
agent-signal focus <context-id>

# Toggle / inspect local Kokoro voice playback
agent-signal voice
```

`--dry-run` is available globally; run `agent-signal --help` for the full list. The `voice` and `speak-last` subcommands shell out to the Python TTS package.

## Uninstall

```bash
./scripts/uninstall-native.sh                # unload agents, remove bins/app/plists; keep state
./scripts/uninstall-native.sh --purge-state  # also wipe ~/.local/state/agent-signals
```

The uninstaller does not touch hook wiring in `~/.claude/settings.json` — remove that by hand.

## Paths

| Thing | Path |
|---|---|
| State dir | `~/.local/state/agent-signals/` |
| Daemon socket | `~/.local/state/agent-signals/agent-signald.sock` |
| Context store | `~/.local/state/agent-signals/contexts/` |
| Dedup entries | `~/.local/state/agent-signals/dedup/` |
| Spool (offline queue) | `~/.local/state/agent-signals/spool/` |
| Active index | `~/.local/state/agent-signals/active-notifications.json` |
| Rust binaries | `~/.local/bin/agent-signal`, `~/.local/bin/agent-signald` |
| Hook wrappers | `~/.local/bin/agent-response-notify`, `agent-focus-pane`, plus voice helpers |
| Swift app | `~/Applications/AgentSignalsNotifier.app` |
| LaunchAgents | `~/Library/LaunchAgents/com.nathan.agent-signald.plist`, `com.nathan.AgentSignalsNotifier.plist` |
| Voice state | `~/.local/state/agent-voice/` |

## Development

```bash
# Rust client + daemon
cd native/rust && cargo test

# Python TTS package (voice features only)
.venv/bin/python -m pytest -q
```

The Rust crate lives in `native/rust` — the daemon and client share a single `src/lib.rs`, with thin `src/bin/agent-signal.rs` and `src/bin/agent-signald.rs` entry points. The Swift notifier lives in `native/swift/AgentSignalsNotifier`. The Python TTS package lives in `src/agent_signals`.

## License

MIT.

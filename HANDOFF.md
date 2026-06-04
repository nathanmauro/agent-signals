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
          │  dedup (20s, age-GC'd + count cap) · context store (24h TTL, active-index cap 32)
          │  spool (cap 128) · periodic sweep every 6h on a dedicated thread
          │  persistent socket
          ▼
       AgentSignalsNotifier.app (Swift, LaunchAgent) — UNUserNotificationCenter
          │  on click → daemon → focus_context → activate Ghostty + zellij/tmux pane focus
```

- The notifier runs an **AppKit `.accessory` event loop** (`NSApplication`, no Dock icon, paired with `LSUIElement`) rather than a bare Foundation `RunLoop`. This is required: macOS activates the posting app on notification click, and only a real Cocoa event loop can answer that activation and deliver the `UNUserNotificationCenterDelegate` callback in-process. With a bare RunLoop, clicks produced "AgentSignalsNotifier.app is not responding" and the click→focus path never fired (fixed 2026-05-29).
- `speak-last` and `voice` shell out to the Python package (`python -m agent_signals.cli`). Everything else is native.
- Native notifications replaced `terminal-notifier` — this killed the `-sender`/`-execute`/`-ignoreDnD` conflicts and the per-notification process pile-up. `error` severity → `.timeSensitive` (DND override); `needs_input` and `normal` are routine.
- Resilience: daemon down → client spools + plays local cues itself. Notifier down → daemon spools, drains on reconnect.

## Source layout

- `native/rust/src/lib.rs` — the entire daemon + client (one file). Bins: `agent-signal`, `agent-signald`.
- `native/rust/tests/replay_real_events.rs` + `tests/fixtures/*.json` — real notification events captured from live daemon state, replayed through `parse_hook_payload` → `build_envelope` to pin severity / group_key / rendered payload. Includes the Codex `--client`-blob regression (see open item 3). `parse_hook_payload` and `build_envelope` are `pub` so the integration crate can call them.
- `native/swift/AgentSignalsNotifier/main.swift` — notifier helper (socket client + UNUserNotificationCenter).
- `src/agent_signals/` — Python package: TTS/voice (Kokoro) + a parallel legacy CLI.
- `bin/` — hook wrappers (`agent-response-notify`, `agent-focus-pane`) → call `agent-signal`. Installed to `~/.local/bin` by `install-native.sh`.
- `launchd/` — the two LaunchAgent plists.
- `scripts/install-native.sh` — build + install (bins, wrappers, app, plists) + load + doctor. `scripts/uninstall-native.sh` — unload + remove.
- `README.md` — install / usage / troubleshooting.

## Current status (2026-05-29)

The native rewrite is **deployed and live** on `in8-mac`. Daemon + client + notifier all built and installed; Rust + Python suites green; repo public and pushed.

- ✅ State growth is bounded on every path: dedup is age-GC'd **and** count-capped (`MAX_DEDUP_FILES`), contexts GC'd by 24h TTL + active-index cap 32, spool capped at 128. The sweep is **periodic** — it runs every 6h on a dedicated thread, not just at startup.
- ✅ `doctor` reports a remediation hint when notification auth is denied; on `in8-mac` it currently reports `authorized` + `notifier connected: true`.
- ✅ README, `scripts/uninstall-native.sh`, and the `install-native.sh` bin/ wrapper install are all in place.
- ✅ **Notification clicks work** (fixed 2026-05-29). The notifier was a bare Foundation `RunLoop` that could not answer the click-activation Apple Event, so every click produced "AgentSignalsNotifier.app is not responding" and LaunchServices spawned a dead duplicate instance. Switched to an AppKit `.accessory` event loop. This is what makes the click→focus path (the headline feature) actually fire — it had never worked on a real click before this.
- ✅ **Codex JSON-as-client regression is fixed** (2026-06-04). The wrapper now treats a JSON first argument as the payload and defaults the client to `Codex`; `parse_hook_payload` also has the same defensive path. Routine normal turn-complete notifications now reuse the pane-scoped notification id, so they replace the latest card instead of accumulating. `needs_input` notifications still keep distinct ids so prompts can stack by session/window/pane.

## Plan / open items

The original three open items are **done**: notification auth is `authorized`, install + Stop/Notification hooks are wired in `~/.claude/settings.json`, and the repo is public + pushed.

Remaining:

1. **Verify click→focus end-to-end.** Now that clicks deliver (post-AppKit fix), confirm a real banner click activates the originating Ghostty + zellij/tmux pane via `focus_context`. This path was unreachable before 2026-05-29, so it has effectively never been exercised on a live click.
2. **Persistent banners (user action).** Banner-vs-Alert is a System Settings choice, not code: **System Settings → Notifications → Agent Signals Notifier → "Alerts"** to make banners stay until dismissed. Code cannot force it — `.timeSensitive` only overrides DND, and `.critical` needs an Apple entitlement an ad-hoc local build can't have.

## Deploying changes — REQUIRED after any source change or PR merge

**Merging a PR does NOT make changes go live.** The runtime is compiled artifacts — `~/.local/bin/{agent-signal,agent-signald}`, `~/Applications/AgentSignalsNotifier.app`, and two launchd services — none of which a `git merge`/`git pull` touches. The repo source is not what runs.

After merging a PR (or any local edit to Rust/Swift/wrapper source), run:

```bash
scripts/install-native.sh
```

It rebuilds both release binaries, reinstalls them + hook wrappers, rebuilds + reinstalls the notifier app and plists, and `launchctl kickstart -k`'s both LaunchAgents so the running daemon/notifier exec the fresh binaries. Then it runs `doctor`.

- **CLI-path changes** (`agent-signal`, hook wrappers) go live as soon as the binary on disk is replaced — hooks `exec` it fresh per call, so no restart is strictly needed for those. The notify-suppression fix (PR #2) was a CLI-path change, which is why it went live from a manual rebuild without restarting anything.
- **Daemon/notifier changes** (`agent-signald`, the Swift app) require the service restart — the running process holds the old binary in memory until `kickstart -k` cycles it.
- Don't hand-copy individual binaries: `cargo build` produces both, and copying only `agent-signal` leaves `agent-signald` drifted (exactly what happened around PR #2 — installed daemon stayed at the May 29 build). `install-native.sh` reconciles everything in one shot.

## Commands

```bash
scripts/install-native.sh                 # build + install (bins, wrappers, app, plists) + load + doctor
scripts/uninstall-native.sh               # unload LaunchAgents + remove installed artifacts
agent-signal doctor --verbose             # health: socket, notifier, auth (+ auth remediation hint)
agent-signal sweep --legacy --dry-run     # preview state cleanup
agent-signal clear                        # clear delivered Agent Signals notifications + active/spooled state
cd native/rust && cargo test              # Rust suite
.venv/bin/python -m pytest -q             # Python TTS suite
```

See `README.md` for full install / usage / troubleshooting.

## State / paths

- State: `~/.local/state/agent-signals/` — `agent-signald.sock`, `contexts/`, `dedup/`, `spool/`, `*.log`
- Bins: `~/.local/bin/agent-signal`, `~/.local/bin/agent-signald`
- App: `~/Applications/AgentSignalsNotifier.app`
- LaunchAgents: `~/Library/LaunchAgents/com.nathan.agent-signald.plist`, `com.nathan.AgentSignalsNotifier.plist`
- Voice state: `~/.local/state/agent-voice/`

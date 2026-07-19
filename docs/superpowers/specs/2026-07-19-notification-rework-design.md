# Notification Rework — Design

**Date:** 2026-07-19
**Status:** Approved (design review with Nathan)
**Scope:** Native Rust/Swift notify path (the live one), plus one targeted fix to the Python `/say` path. Deploys via `scripts/install-native.sh`.

## Problem

Notifications fire far too often — enough that Nathan turned them off. Specific failures:

1. **False "needs input" after recap/compaction.** Two stacked causes:
   - The Claude Code `Notification` hook is trusted blindly: every event on it becomes a "{client} needs input" banner + bell + sound with no content inspection (`native/rust/src/lib.rs` `build_envelope`, ~:607). Claude Code fires this hook for idle-waiting as well as permission prompts, and compaction pauses are exactly the quiet stretch that trips the idle timer. Only a 20s dedup applies.
   - `Stop` events are classified against the bottom-most `type:"assistant"` transcript entry with no `isCompactSummary`/`isMeta`/sidechain filtering. Recap text characteristically contains "Would you like me to…", "let me know", trailing "?" — precisely what the needs-input regex matches. "Compact"/"recap" appear nowhere in the codebase; the failure mode is unhandled.
2. **The needs-input regex is the only gate for turn completions and matches sign-off boilerplate** ("let me know", "want me to", "your call", any trailing "?") anywhere in the message.
3. **The error regex matches benign prose** ("could not", "can't", "unable to", "blocked") and those alerts are `.timeSensitive` — they punch through Do Not Disturb.
4. **Every delivered notification fires banner + bell + sound** (hardcoded in `channels_for_severity`), needs-input cards stack per-event instead of replacing per-pane, there is no cooldown beyond the 20s same-key dedup, and no focus check — it notifies while you're looking at the pane that produced it.
5. **Bodies are uninformative and self-contradicting.** A needs-input card says "…is ready. Click to focus." The last assistant message is already extracted for classification but never shown.
6. **Three drifted copies of the severity patterns** exist: `native/rust/src/lib.rs`, `src/agent_signals/policy.py`, `config/voice-rules.json`.

## Goals

- Kill the recap/compaction false positive entirely.
- Cut cadence to the point where a notification means something: fewer events qualify, qualifying events ring fewer channels, repeats replace instead of stack, and nothing fires while Nathan is looking at the source pane.
- Make the body carry the actual ask.
- One source of truth for severity patterns; runtime-tunable config so cadence changes don't require a recompile.

## Non-goals

- Quiet hours / scheduling.
- Bringing the dormant Python notify path to feature parity (it stays dormant; it does share the unified severity rules).
- Linux/Windows support; non-Ghostty terminals.
- Changing the voice/TTS feature set (only the one compaction bug fix in `/say --previous`).

## Approved decisions

| Decision | Choice |
|---|---|
| Permission prompts | Banner only, per-pane card replacement — no bell/sound, no stacking |
| Focus gate | Suppress entirely (all channels) when Ghostty is frontmost AND the event's pane is the focused pane of the attached mux session; fail-open (notify) when focus can't be determined |
| Body content | The extracted closing ask, trimmed to ~140 chars; pane/session ref moves to the subtitle |

## Design

### 1. `Notification`-hook triage

`build_envelope` stops mapping every `Notification` event to full-strength `needs_input`. The hook's `message` text is classified into a `kind`:

- **`permission`** (message matches permission phrasing, e.g. "needs your permission", "wants permission to"): severity `needs_input`, **banner only**, notification id keyed per-pane so repeated prompts from the same pane replace one card. Body = the hook message verbatim (it already names the tool).
- **`idle`** (message matches "waiting for your input"): **dropped** when the transcript's last real boundary is a compact continuation or the turn is still in-flight (see §2); otherwise banner-only `needs_input`, per-pane replace. Rationale: the turn-end `Stop` already notified; idle is at best a reminder.
- **`unknown`**: banner-only `needs_input`, per-pane replace (fail-soft: still visible, never loud).

This is the direct fix for the false "wants input" banner.

### 2. Compaction-aware turn extraction (Rust port)

Port the turn-boundary logic from the in-flight Python `/say` work (`src/agent_signals/transcripts/claude.py`: `_user_prompt_text`, `extract_previous_turn_from_claude`) into the Rust transcript reader:

- Walk the transcript bottom-up. Skip entries with `isMeta`, `isCompactSummary`, or sidechain markers, and user entries whose content is tool_result-only.
- Identify the last *real* user-prompt boundary; the classified text is the assistant text of the completed turn after it.
- If the latest boundary is a compact continuation (`isCompactSummary: true` synthetic user message), the `Stop` does not notify at all.
- The severity regexes evaluate only the **closing window** of the extracted text (last 2 sentences / ~300 chars), not the whole message.

### 3. Severity rules: one shipped source

New `config/severity-rules.json`, installed by `install-native.sh`:

- `needs_input` patterns: curated list (question-forms, explicit decision asks), evaluated end-anchored per §2. Loosest conversational patterns pruned.
- `error` patterns: require error-shaped tokens ("error:", "failed", "traceback", "exception", "permission denied"). Bare "could not" / "can't" / "unable to" / "blocked" are removed. Only `error` severity may set `.timeSensitive` (DND-piercing).
- Closing-window size lives in the file.

Rust loads it at daemon/client start with a compiled-in fallback identical to the file. `src/agent_signals/policy.py` reads the same file. `config/voice-rules.json` is regenerated from it (or references it) so the three copies can't drift.

### 4. Message content

| Event | Title | Body | Subtitle |
|---|---|---|---|
| Turn-end `needs_input` | `{client} needs input` | Extracted closing ask, ≤140 chars (e.g. "Which auth approach do you want — JWT or sessions?") | `{session} / {tab-or-window} · {pane}` |
| `error` | `{client} hit an error` | Closing error line excerpt, ≤140 chars | same |
| Permission | `{client} needs permission` | Hook message verbatim | same |
| Idle (when it survives §1) | `{client} is waiting` | "Still waiting on you — click to focus." | same |

- `"…is ready. Click to focus."` and `"Prompt response is ready."` are retired from needs_input/error cards. When extraction yields nothing, fall back to a severity-consistent generic ("Needs your input. Click to focus.").
- `normal` severity remains fully dropped (unchanged from `is_actionable`).

### 5. Cadence layer

- **Focus gate** (client-side, before daemon send): Ghostty frontmost (NSWorkspace via the daemon/Swift helper, or `osascript` fallback) AND event pane == focused pane of the attached mux session (tmux: `display-message`; zellij: best-effort state query). Both true → suppress all channels. Undeterminable → notify (fail-open).
- **Per-pane cooldown:** same pane + same severity within 45s → post silently (card replaces, no bell/sound re-ring). Distinct from the existing 20s same-event dedup, which stays.
- **Config file** `~/.config/agent-signals/config.json` (compiled defaults; file optional): channels per severity, cooldown seconds, focus-gate on/off, idle policy (`drop` / `banner` / `full`).

### 6. Python `/say` compaction fix

`_user_prompt_text` additionally skips `isCompactSummary` entries so `/say --previous` after a compaction doesn't read pre-compact text. New fixture + test.

## Testing & verification

- **Rust unit tests** with new fixtures: recap-`Stop` payload + compaction transcript, idle-`Notification` payload, permission-prompt payload, sign-off-boilerplate turn (asserts no notify), genuine closing question (asserts needs_input + extracted body).
- **Python tests**: existing suite plus the `isCompactSummary` case.
- **End-to-end (verify through use):** replay real captured hook payloads through the installed `agent-response-notify` binary and assert the daemon's decision per event in its logs (posted / dropped / silent-replace); one live banner spot-check.
- **Deploy:** `scripts/install-native.sh` (merge alone changes nothing live).

## Sequencing

1. Land the in-flight `/say` diff as its own commit/PR (it is coherent and test-covered), including the §6 fix.
2. `notification-rework` branch: severity-rules file → Rust triage + compaction-aware extraction → message templates → cadence layer + config → Swift changes → tests/e2e → install + live verification.

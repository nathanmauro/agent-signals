# Agent Island — Design Spec

- **Date:** 2026-09-25
- **Status:** Draft for Nathan's review
- **Supersedes:** `2026-07-19-notification-rework-design.md` (agent-signals notification path). That rework is parked, not merged.
- **New home:** `~/Developer/proj/agent-island`. This spec moves there when the repo is bootstrapped.
- **Research evidence:** `~/.local/state/agent-island/research-2026-09-25/` is private and holds raw session text. Mask it before committing anything from it.

---

## 1. Intent

### 1.1 What Nathan asked for

- **Separation.** Agent notifications should be kept apart from all his other macOS notifications. Today they share Notification Center with Slack, Mail and the rest.
- **An island.** An iPhone-style Dynamic Island display that lives in the menu bar.
- **Adopt first.** If someone has already built this, take it and adapt it.
- **Herdr.** A first pass may be pinned to Herdr, his terminal workspace manager for agents. A full teardown of agent-signals is acceptable.

### 1.2 Decisions made while brainstorming (2026-09-25)

| Question | Decision |
|---|---|
| Island's job | **Live board + interrupts.** The pill is always visible and shows counts. Hovering opens the board. It pops out and chimes **only** when an agent needs Nathan or has errored. Finished turns show quietly: a count, no sound. |
| Answering approvals from the island | **Not in v1.** Nathan approves in the terminal. |
| Codex Desktop, which is not in Herdr | **Covered** by reading its session files. Codex Desktop's own banners are turned off. |
| Claude outside Herdr (tmux/salvo, remote-control, Desktop Code tab) | **Covered in v1** through Claude Code's session registry. |
| Where the pop-out appears | The pill lives on the **primary display**. The pop-out card appears on **the display under the pointer**. |
| Where the code lives | **New repo `~/Developer/proj/agent-island`.** agent-signals stays frozen as the home of `/say` TTS. |
| Base | **Moonglade fork** (MIT) for the display and window layer, plus **Bantay-TUI's** (MIT) Herdr request/response codec. Everything else is new. |

### 1.3 Why agent-signals failed

This is what the design must not repeat. Evidence is in the research bundle.

1. **It interrupted instead of giving him somewhere to look.**
   - Everything went through `UNUserNotificationCenter` into the shared Notification Center.
   - Banners vanished.
   - "Done" events were deliberately dropped.
   - Nothing cleared an alert when the agent moved on.
2. **Severity was guessed from the agent's prose.** Routine Codex turn completions shipped as `error` and broke through Do Not Disturb. The fix was written but never installed.
3. **The sound was separate from the card.** `afplay` ran in the hook process, so Nathan heard a ding with nothing on screen. That is what led him to switch it off on 07-27. Both LaunchAgents have been disabled since then.
4. **Click-to-focus never worked on a real click.** It knew about zellij and tmux, but not Herdr.
5. **The deploy surface drifted.** It had a daemon, a notifier app, wrappers in two places, and Python run from the repo through a path baked in at compile time. It was never tested live, and it kept no event log.

### 1.4 Success criteria

Each criterion is testable. See §12.

1. On the Samsung ultrawide (the main display, no notch), a pill sits at the top center **inside** the menu bar. It shows working, waiting, done and error counts, and it is readable without clicking.
2. Within 2 s of an agent becoming blocked on a permission or question, a card pops out on the display under the pointer. It names the agent and shows the question. Exactly one chime plays with it.
3. A finished turn raises the `done` count **silently**. Looking at that agent (in the island, in Herdr, or in Codex.app) clears it. Nothing needs clearing by hand.
4. One click lands on the exact Herdr pane in the right Ghostty window, or opens the exact Codex thread.
5. **Every sound has a visible card.** No agent sound comes from anything except the island.
6. A read-only query of the Notification Center database shows **zero** agent records after cutover.
7. The island survives display unplug/replug, sleep/wake and full-screen Spaces. It uses under 1 % CPU when idle and its file-descriptor count stays stable.
8. There is one app and one LaunchAgent. No hooks are installed, no agent config is written, and nothing is posted to Notification Center.

---

## 2. Architecture

```
  Herdr socket ─────────┐   (Claude panes in Herdr)
  ~/.config/herdr/       │
  herdr.sock             │
                         ▼
  Claude registry ──► [ SessionFeed × 3 ] ──► StateStore ──► InterruptPolicy ──► PeekController ─► card (pointer display)
  ~/.claude/sessions/    │   pure reducers      merge/sort      debounce/episodes     ChimePlayer ──► one chime
  *.json                 │                          │
                         │                          ├──► Pill + Board (primary display)
  Codex rollouts ────────┘                          └──► TransitionLog (JSONL)
  ~/.codex/sessions/                                   JumpPlanner ─► agent.focus + raise Ghostty | open codex:// | …
```

**Principles:**

- **Hook-free and read-only.** The island only reads Herdr's socket, Claude Code's registry and Codex's rollout files. It never writes agent configuration, installs hooks, registers Herdr plugins, or opens a socket of its own.
- **No Notification Center.** `UNUserNotificationCenter` is absent from the binary, and a static guard test enforces this (§12.1).
- **Sound and card come from one place.** `ChimePlayer` is called only by `PeekController`, when it shows a card.
- **The core is pure.** All state logic lives in the pure `IslandCore` target (reducers, policy, parsers), so it can be tested against recorded fixtures. The app target does I/O and UI only.
- **One process.** An LSUIElement app launched by one LaunchAgent (`com.nathan.agent-island`, `KeepAlive`).

---

## 3. Base and provenance

### 3.1 Moonglade

`ixjosemi/Moonglade`, MIT, commit `be0c5b4` (v0.4.0, 2026-08-15). This is a **one-time adoption, not a tracked fork**: about half of it is deleted, so nothing upstream will merge.

**Why Moonglade.** It is the only candidate whose display layer is verified on Nathan's hardware. A launch-free layout probe gave:
- **Samsung:** `presentation=pill`, 23 pt bar, 4 pt top gap, inside the 31 pt menu bar. The leftmost of the 17 status items is at x=1837, so a pill centered near x=1280 hides none of them.
- **Built-in display:** `presentation=notch`, notch width 185 pt.

It also has:
- a `.statusBar`, non-activating, borderless panel;
- `[.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]`;
- screen-change and Space observers;
- hover-to-expand;
- zero dependencies, a 9 s build, and 297 passing tests (a custom executable runner, `moonglade-tests`, not XCTest).

**Keep:**
- **Display and window layer:** `NotchPanel`, `NotchLayout`, `HangingNotchGeometry`, `ScreenSelection`, `NotchGlassBackground` (with its public `NSVisualEffectView` fallback).
- **Interaction:** `HoverInteraction`, `HoverSelection`, `PointerMovementGate`, `PointerSampleReducer`, `InlineActionsClickGate`, `SessionMenuLayout`.
- **Rows and text:** `StatusIndicatorStyle`, `SessionStatusSummary` (rewritten), `SessionNameOverrides`, `SessionTitleFormatter`, `SessionDurationFormatter`, `BrailleSpinner`, `PinnedSessionOrder`.
- **Utilities:** `SingleInstanceLock`, `GitWorkspaceInspector`, `BoundedProcessRunner`, `SecureFileReader`, `BoundedInput`, `CodexRolloutParser`.
- **About 100 of the tests:** layout, geometry, hover, pointer, click gates, formatters, name overrides, screen selection.

**Delete (about 6k LOC):**
- `Installation/` (`Installer`, `InstallationDoctor`, `ClaudeSettingsMerger`).
- The `moonglade` CLI target and the hook resources.
- `ClaudeHookProcessor`, `CodexNotifyProcessor`, `ConvoyRunsWatcher`.
- `CodexSessionsWatcher`. It binds each thread to a cwd-matched `codex` process, and the app-server's cwd is `/`, so it shows **zero** Codex Desktop threads. Its parser is kept.
- `ReaperService`, `ProcessScanner`, `ObservationScheduler`, `ObservationCadence`, `ProcessCommandCache`.
- `GhosttyTerminalQueryCache`, `GhosttySessionMatcher`, `TerminationService` (Kill).
- `StateRepository` (1,380 LOC), `StateChangeNotifier`, `ExternalFocusedWindowProvider`.
- `RippleEffect`, `Ripple.metal`, the prebuilt metallib and its CI hash check.
- Every pid and `processIdentity` path.
- `~/.moonglade` paths, the `com.moonglade.app` bundle id, and upstream `install.sh` / `install-app.sh`. **Never run those scripts.** They rewrite `~/.claude/settings.json` through the cockpit symlink.

### 3.2 Bantay-TUI

`8-BitRhyon/bantay-tui`, MIT, commit `e3e0517`.

**Port** the Herdr request builder, response and event parser, and one-shot client from `HerdrSocketClient.swift` / `HerdrSocketAdapter.swift` (about 160 LOC) as `HerdrCodec`. Fix it to send `target`, not `pane_id`, for `agent.*` methods, which protocol 22 requires. Also borrow the idea of a display-settle delay (0.35 s after wake or a screen change).

**Do not take:**
- `HerdrEventStream`. It leaks sockets (4 → 16 fds in 12 s, measured) and never subscribes to `pane.agent_status_changed`.
- `AgentEventManager`, any UI, the approval, hotkey and control-socket paths, the installers, or the shelf, tasks and usage features.

### 3.3 Licensing

The new repo keeps Moonglade's MIT `LICENSE` text and `NOTICE` (trademark statements for the bundled agent glyphs) and credits Bantay-TUI in `NOTICE`. Repo visibility is decided at bootstrap.

### 3.4 Toolchain

Swift 6.3.1 / Xcode 26.4.1 on macOS 26.5.1, with Swift 5 language mode (as upstream). Minimum macOS 14. Bundle id `com.nathan.agent-island`.

**Targets:**
- `IslandCore`: pure; no AppKit.
- `AgentIsland`: the app.
- `island-tests`: executable runner, as upstream.

---

## 4. Data model (`IslandCore`)

```swift
enum SessionSource { case herdr, claudeRegistry, codexDesktop }   // repurposes Moonglade's enum

enum DisplayState { case waiting, error, working, stale, doneUnseen, idle, starting }

struct RowID: Hashable { let source: SessionSource; let key: String }   // pane_id | pid+procStart | threadId

struct AgentRow: Equatable {
  let id: RowID
  let source: SessionSource
  var title: String          // session topic / thread name
  var subtitle: String       // Herdr workspace › tab | cwd basename
  var state: DisplayState
  var since: Date            // island-observed time of last state change
  var detail: Detail?        // question + options | error line | recap (lazy)
  var jump: JumpTarget
}

struct Detail: Equatable { var question: String; var options: [String]; var kind: Kind }  // Kind: question | permission | error | recap

enum JumpTarget {
  case herdrPane(paneID: String, windowTitlePrefix: String?)
  case codexThread(id: String)
  case claudeDesktop(sessionID: String)
  case terminal(tmuxTarget: String?)   // registry-only terminal sessions
}
```

`Summary(rows) -> [Segment]` produces the pill text in fixed order: `waiting`, `error`, `working` (stale rows count as working), `done`. Zero segments are hidden. `idle` and `starting` are never counted.

---

## 5. Sources

Every source implements one protocol:

```swift
protocol SessionFeed: AnyObject {
  var source: SessionSource { get }
  func start(_ publish: @escaping @MainActor ([AgentRow]) -> Void)  // publishes the full row set on every change (idempotent)
  func stop()
  func jump(_ row: AgentRow) async throws
}
```

`StateStore` keeps `[SessionSource: [AgentRow]]`, deduplicates across sources (§5.2), merges and sorts using Moonglade's `precedes`, and publishes only when something changes.

### 5.1 Herdr feed (Claude panes in Herdr)

**Socket:** `$HERDR_SOCKET_PATH`, or `~/.config/herdr/herdr.sock`. NDJSON, protocol 22. Verified against `herdr-schema.json`.

**Transport rules:**
- Use one Unix-socket connection per request. The server closes it after replying.
- Never write on a subscribed connection. Any write closes the stream.
- Normalize event names with `name.replacingOccurrences(of: ".", with: "_")`. Status events arrive dotted (`pane.agent_status_changed`); lifecycle events arrive underscored (`pane_created`).
- An `invalid_request` with "unknown variant" means **method unsupported**. Parse-level errors come back with `id:""`.
- Cancelling a stream **must close its fd**. Covered by a regression test.

**Methods:**

| Method | Params | Use |
|---|---|---|
| `ping` | – | Require `protocol == 22`, else disable the feed (see §10). Log `version`. Health check every 30 s. |
| `session.snapshot` | – | Bootstrap, reconcile every 12 s, and refresh focus context when a blocked episode starts. |
| `events.subscribe` | global stream **G** | Lifecycle and focus: `pane.created`, `pane.closed`, `pane.exited`, `pane.agent_detected`, `pane.updated`, `pane.moved`, `pane.focused`, `tab.focused`, `workspace.focused`, `workspace.created`, `workspace.closed`, `workspace.renamed`, `tab.created`, `tab.closed`, `tab.renamed`. |
| `events.subscribe` | `[{"type":"pane.agent_status_changed","pane_id":P}]` | **One connection per pane.** Subscriptions are rejected atomically, so one stale pane id must not take down the others. About 20 fds. |
| `pane.process_info` | `{target}` | Pane foreground pids, used to deduplicate against the Claude registry (§5.2). Cached per pane and refreshed on `pane.agent_detected` / `pane.updated`. |
| `agent.read` | `{target, source:"detection", strip_ansi:true}` | Peek text. Detection-only reads are passive. |
| `agent.focus` | `{target: pane_id}` | Jump. It also marks the pane seen. |

**Never called:**
- `agent.explain`: its matched region is about 85 bytes, too little for text.
- `agent.read` with `recent` or `recent_unwrapped`: these scroll idle agents and are refused on busy ones.
- Mutations: `send_keys`, `agent.prompt`, `client.window_title.*`, `plugin.link`.

**Bootstrap:**
1. `ping`.
2. Open G and buffer its events until the subscription is acknowledged.
3. Take snapshot #1 and open a status stream S_p for **every** pane, shells included.
4. Once every stream is acknowledged, take snapshot #2 and apply the buffered events.
5. On `pane_created`, open S_p. On `pane_closed`, close S_p.
6. On EOF or a ping failure: close every stream fd, then reconnect with backoff 0.5 / 1 / 2 / 5 s (capped at 5 s) and repeat the full bootstrap.
7. The 12 s reconcile also covers Herdr issue #3124, where a subscription stays acknowledged but silently stops. If a pane's status in the snapshot differs from what its stream last reported, apply the snapshot's status and reopen that stream.

**Status mapping:**

| Herdr `agent_status` | Island `DisplayState` |
|---|---|
| `working` | `working`. Becomes `stale` when neither status nor `state_change_seq` has changed for more than 30 min. Stale rows are dimmed and show their age ("working 52m"); they never alert. |
| `blocked` | `waiting`, once it has held for 1 s (see §7.4). |
| `done` | `doneUnseen` |
| `idle` | `idle`, unless the overlay below applies |
| `unknown` | `starting`: muted, never counted |

**Error.** Herdr has no error state, so v1 keeps the rule narrow. A pane counts as errored on `pane_exited` while its agent was `working` or `blocked`, or on `pane.agent_detected{released:true}` while it was `working`. **Exception:** ignore the exit if `pane_closed` arrives within 1 s, or if that pane was focused with Ghostty frontmost, because Nathan closed it himself. There are no screen-text error heuristics in v1.

**Seen state.** Herdr's server-side `done` is the primary seen signal. Observed live on 0.9.0:
- Viewing a pane in the TUI clears `done` for the whole server and emits `→ idle`.
- `agent.focus` from the island does the same.
- Reads do not clear it.

**Island overlay: "finished while away".** A pane that goes `working → idle` without passing through `done`, while Ghostty is **not** frontmost, is marked `doneUnseen` by the island. This covers a turn that finishes in the focused pane while Nathan is in another app. It clears on:
- a row click;
- the pane's next `working`;
- Ghostty becoming frontmost while that pane is focused (per the latest snapshot).

**Accepted caveat:** any other client or tool that focuses a pane also clears Herdr's `done`.

**Peek text** (`DetectionTextParser`, pure):
- **Blocked.** Call `agent.read detection` (about 65 lines) and parse bottom-up. The question lies between the tab-chip line or horizontal rule and the first option matching `^\s*(❯\s*)?\d+\.\s`. Show up to 4 option labels. If parsing fails, show the pane title and "needs you".
- **Done recap.** Take the `※ recap:` line, falling back to the last `⏺` paragraph. Fetch it lazily on hover, never on the event path.

**Titles:**
- title: `terminal_title_stripped`;
- subtitle: `workspace label › tab label`;
- Ghostty window match: `window_title` (default `ui.window_title` = `"<host>: <workspace>"`).

### 5.2 Claude registry feed (Claude outside Herdr)

**Files.** `~/.claude/sessions/<pid>.json` only. **Never open `*.key` files** in that directory; they are messaging-socket secrets. A static guard test checks that the feed's path filter matches `*.json` only.

**Watching.** FSEvents on the directory, plus a 2 s liveness sweep. A row is live when `kill(pid, 0) == 0` **and** the process start time matches `procStart`, which guards against pid reuse.

**Fields used:** `pid`, `sessionId`, `cwd`, `procStart`, `kind`, `entrypoint`, `name`, `status`, `waitingFor`, `statusUpdatedAt`, `tmux`.

**Filters:**
- Include only `kind == "interactive"`.
- Exclude headless entrypoints: `sdk-cli`, `sdk-ts`, and any other `sdk-*`.

**Deduplication.** Drop a registry row when its pid is a foreground process of any Herdr pane (from `pane.process_info`). **Herdr is authoritative for Herdr panes.**

**Mapping:**

| `status` | Island state |
|---|---|
| `busy` | `working` (stale rule as in §5.1, keyed on `statusUpdatedAt`) |
| `waiting` | `waiting`. `Detail.kind` is `question` for `waitingFor` "input needed" or "dialog open", and `permission` for "permission prompt". The text is `waitingFor` plus the session name. |
| `idle` after `busy` | `doneUnseen`. The island owns this seen flag; it clears on a click, the next `busy`, or 12 h. |
| `idle` | `idle` |
| `shell` | `idle` |

**Jump:**
- `entrypoint == "claude-desktop"` and Claude.app running: open `claude://code/continue?session=<sessionId>` using the running Claude.app's bundle URL. Fall back to `claude://code/needs-input`. The accepted id format is unverified (§15).
- `tmux` field present: activate Ghostty, then `tmux switch-client -t <target>`. The field's shape is verified during implementation.
- Otherwise, activate Ghostty.

**Soak diagnostic (log-only).** For Herdr panes whose pid appears in the registry, write the registry `status` next to Herdr's status in the transition log (§9). The data decides whether v1.1 should use the registry as a tie-breaker for Herdr detection flicker and stuck "working".

**Not covered:** agent-team teammates in tmux (`--agent-id …`) do not appear in the registry. Their lead pane already shows as working.

### 5.3 Codex Desktop feed

**Files.** `$CODEX_SESSIONS_DIR`, or `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. Codex Desktop and `codex_work_desktop` produced 201 of 212 recent rollouts. The Codex TUI produced none.

**Watching.** Recursive FSEvents on the sessions directory, plus a 2 s stat poll over files modified in the last 24 h. Read incrementally from byte offsets.

**Cold start** (files reach **200 MB**; 131 MB is written in a day):
1. Always read **line 1** (`session_meta`, about 20 KB) separately, capped at 1 MB. The thread id also comes from the filename UUID.
2. Scan **backwards** from the end in 256 KB chunks until the last `task_started`, `task_complete` or `turn_aborted` is found. The last `task_started` can be 0.4–5.6 MB from the end.
3. From then on, read forward incrementally.

**Filters.** Drop the session when `session_meta.payload.source` is an object with a `subagent` key, or when `thread_source ∈ {guardian_review, subagent}`. Guardian turns outnumber real ones about 4:1. `source` is sometimes a string and sometimes an object, so it needs a custom decoder. Keep `user`, `agent_created_thread`, `voice_chat`, `automation` and `realtime_voice`. `codex_exec` and chrome-extension originators are hidden by default; one setting shows `exec`.

**State:**
- `task_started(turn_id)` → `working`.
- `task_complete(turn_id)` → `doneUnseen`, with `last_agent_message` as the recap and row detail.
- `turn_aborted` → `idle`, with no chime (all 7 observed were `reason: interrupted`).
- **Waiting (exact):** a `function_call` named `request_user_input` (the blocking Plan-mode tool) with no matching `function_call_output` for the same `call_id`. The question comes from the call's arguments. It clears when the output arrives. There were 65 such calls through 08-16, with waits of up to 12,068 s.
- **Waiting (heuristic):** the last completed turn contains a `request_user_input_async` call and no later `task_started`. It clears on the next `task_started`, a click, a Codex activation, or after 12 h.
- `error` / `stream_error` event messages → `error`. These event names are **inferred** (none observed) and covered only by fixtures.
- If `com.openai.codex` is not running, `working` threads become `stale`.
- **No approval signal exists in this setup:** `approval_policy = "never"`, and there were 0 approval events in 534 rollouts over 30 days.

**Titles.** Read from `~/.codex/session_index.jsonl` (`{id, thread_name}`), falling back to the cwd basename.

**Seen store.** `~/Library/Application Support/AgentIsland/codex-seen.json` holds `{threadId: lastSeenTurnId}` and is written atomically. On first launch, every completed turn is recorded as seen, so the board does not flood. A thread is marked seen on any of:
- a row click;
- a later `task_started`;
- `NSWorkspace.didActivateApplication` for `com.openai.codex` after the completion;
- 12 h passing.

**Jump.** `NSWorkspace.open(URL("codex://threads/<id>"), withApplicationAt: <running com.openai.codex bundleURL>, …)`. ChatGPT.app also claims `codex:`, so the running Codex app's bundle URL must be passed. That `<id>` is the rollout's `session_meta.id` is **inferred** and must be verified live (§12.5).

---

## 6. Display and windowing

- **Pill: primary display.** A new `ScreenSelection.primary` returns the display whose menu bar is at (0,0), i.e. `CGMainDisplayID`, not `NSScreen.main`. On the Samsung this is a 23 pt pill inside the 31 pt menu bar. With the lid only, it hugs the notch.
- **Peek card: the display under the pointer.** It uses the same hanging geometry. On a non-primary display it hangs from that display's top center: in its notch on the built-in display, or as a pill on a non-notch screen. The card is a second panel instance. The pill never moves.
- **Option:** `allDisplays`, which already exists upstream, mirrors the pill on every display. Pointer-follow and focused-window modes for the **pill** are removed.
- **Full-screen Spaces:** shown (`.fullScreenAuxiliary`) so interrupts stay visible. A "hide collapsed pill in full-screen" option is deferred until the live check shows it gets in the way.
- **Click-through.** The panel spans 800×394 pt. Without extra handling, about 363 pt of it would cover Ghostty's tab and title bars. `ignoresMouseEvents = !interactiveRegion.contains(pointer)` is driven by an **always-on global and local mouse-move monitor**. `interactiveRegion` is a pure function of the layout state: the collapsed pill rect, the expanded board rect, or the card rect.
- **Hover is driven by the monitor.** Moonglade's hover currently relies on an `NSTrackingArea`, which stops delivering events while `ignoresMouseEvents` is true. The monitor becomes the primary hover input: location, entered and exited, re-evaluated whenever the region changes. The tracking area is secondary.
- **Settle.** After wake or a screen-parameter change, wait 0.35 s, then re-anchor.

---

## 7. Interaction

### 7.1 Pill

`3 working · 1 waiting · 2 done`, using Nathan's word "waiting" for blocked. When present, an error segment is shown first, in red. With no agents the pill shows a dim glyph. When a feed is offline, a small warning glyph appears; hovering it names the feed.

### 7.2 Board (on hover or click)

- Groups, in order: **Waiting**, **Error**, **Working** (with age; dimmed when stale), **Done**, then a collapsed "N idle".
- Compact rows, about 28 pt high. The board's height is capped at about 60 % of the visible screen height.
- Each row shows a status light, title, subtitle, a source glyph (Herdr / Claude / Codex), the time since its last change, and the git branch (cached).
- Rows can be renamed (name overrides kept from upstream). There is no Kill action.

### 7.3 Peek and chime (waiting and error only)

- **Card content:** one row plus the question and up to 4 options, or the error line.
- **Timing:** it stays up 8 s, and stays open while hovered.
- **Several pending:** it shows the most severe first (error, then waiting), then the newest, with "+N more".
- **Chime:** system sound "Glass" at volume 0.35, played only by `PeekController` while it shows a card.
- **Settings:** mute, display mode, and show-exec-threads. **Peek-for-done is deliberately not a setting.**

### 7.4 `InterruptPolicy` (pure)

`decide(prev, next, focus, now) -> (peeks: [PeekEvent], chime: Bool)`

| Rule | Value |
|---|---|
| A blocked state must hold before it counts | 1 s. This absorbs the observed 0.3 s blocked→done→blocked flicker. |
| Blocked episode | One peek and one chime per row per episode. A new episode needs 10 s out of blocked, or a change in the question text. |
| Global gap between chimes | 3 s; the peek still shows. |
| Quiet period after launch or reconnect | 10 s. Rows already waiting show in the count only. |
| Suppressed while looking | **Herdr:** Ghostty is frontmost **and** a fresh snapshot says the pane is focused. **Codex:** Codex.app is frontmost. **Claude Desktop:** Claude.app is frontmost. Suppression affects the peek and chime only; counts still update. |
| Done, working, idle, starting | Never peek, never chime. |

### 7.5 Jump (row or card click)

`JumpPlanner.plan(JumpTarget) -> [JumpAction]`, pure:

- **Herdr:**
  1. `agent.focus{target}`.
  2. `GhosttyRaiser`: AppleScript `focus` on the Ghostty terminal whose window name starts with the Herdr window title prefix, then `activate`. This needs the Automation permission.
  3. Fallback: `NSRunningApplication(bundleIdentifier: "com.mitchellh.ghostty")?.activate()`.
- **Codex:** open `codex://threads/<id>` with the running Codex bundle URL.
- **Claude Desktop / terminal:** as in §5.2.

A click also marks the row seen (Herdr through `agent.focus`; Codex and registry through their seen stores).

---

## 8. Components

These are units with one purpose each. About 8–8.5k Swift LOC in total: roughly 6.5k kept and 2k new.

**`IslandCore`** (pure; tests run against fixtures)

| Unit | Responsibility |
|---|---|
| `Model` | `DisplayState`, `AgentRow`, `RowID`, `JumpTarget`, `Summary`. |
| `HerdrCodec` | Build requests; parse responses and events; normalize names; map unknown-variant errors. Ported from Bantay (credited). |
| `HerdrReducer` | `apply(HerdrInput, now) -> [AgentRow]`: status mapping, stale aging, error rule, finished-while-away overlay. |
| `DetectionTextParser` | `parseBlocked(String) -> Prompt?` and `parseRecap(String) -> String?`. |
| `ClaudeRegistryReducer` | `apply([RegistryEntry], herdrPids: Set<Int32>, now) -> [AgentRow]`: filters, deduplication, mapping, own seen flag. |
| `CodexReducer` | `ingest(lines, file) -> [ThreadState]`: custom `source` decoding, filters, `turn_id` pairing, open `request_user_input`, `_async` heuristic, error. Extends the kept `CodexRolloutParser`. |
| `CodexSeenStore` | `{threadId: lastSeenTurnId}` with a baseline on first run, 12 h expiry, and atomic writes. |
| `InterruptPolicy` | §7.4. |
| `JumpPlanner` | §7.5. |
| Kept Moonglade core | §3.1. `ScreenSelection` gains `.primary`, loses `.pointer` and `.focusedWindow`, and gains a pointer-display resolver for the card. |

**`AgentIsland`** (I/O and UI)

| Unit | Responsibility |
|---|---|
| `HerdrClient` | `request<T>(method, params)` over a one-shot connection. `subscribe([Subscription]) -> AsyncThrowingStream<HerdrEvent, Error>`, one connection per call, closing its fd on cancel. |
| `HerdrFeed` | Bootstrap, per-pane streams, 12 s reconcile, 30 s ping, backoff, cached `process_info`, lazy detection reads. |
| `ClaudeRegistryFeed` | FSEvents plus liveness sweep; `*.json` only. |
| `CodexDesktopFeed` | FSEvents plus stat poll, offsets, first-line and backward-scan cold start, titles, liveness and activation observers. |
| `StateStore` | `init(feeds:)`. Merge, deduplicate, sort, publish on change, call `InterruptPolicy`, expose `focus(rowID)`. |
| `PeekController` | Card panel on the pointer display, queue, 8 s auto-collapse, stays open on hover, updates the interactive region. The only caller of `ChimePlayer`. |
| `ChimePlayer` | One `NSSound`, volume, mute, and `playedCount` for tests. |
| `GhosttyRaiser` | AppleScript focus by title prefix, then activate; `NSRunningApplication` fallback. |
| `TransitionLog` | §9. |
| `StateDump` | When `AGENT_ISLAND_STATE_DUMP=<path>` is set, writes rows, summary, peek queue, `chime.playedCount` and planned jump actions as JSON on every change. Used by end-to-end tests. |
| Kept Moonglade app | `NotchPanel` (adds primary mode, monitor-driven click-through and hover, settle delay), `NotchWidgetView` (bar labels, grouped compact rows, no Kill), `NotchGlassBackground`, `SettingsView` (mute, display mode, show exec). |
| Packaging | `scripts/build-app.sh` (ad-hoc signing, or `SIGN_IDENTITY` if set), `scripts/install.sh` (copies to `~/Applications` and loads the LaunchAgent), `scripts/uninstall.sh`. **No hook step of any kind.** |

**Test and debug environment variables:**
- `HERDR_SOCKET_PATH`, `CODEX_SESSIONS_DIR`, `CLAUDE_SESSIONS_DIR`: point feeds at fakes or fixtures.
- `AGENT_ISLAND_STATE_DUMP`.
- `AGENT_ISLAND_JUMP_DRY_RUN=1`: record jump actions without running them.

---

## 9. Observability

- **Transition log:** `~/.local/state/agent-island/transitions.jsonl`. One line per row state change, peek, chime and suppression:
  - `ts`, `source`, `rowID`, `from`, `to`, `rule` (which policy rule fired or suppressed);
  - question text, truncated to 200 characters;
  - for Herdr panes, the registry `status` shadow (§5.2).
- **Rotation:** at 10 MB, keeping 3 files.
- This answers "what made that sound?" and "why didn't I see X?" without guessing, which closes failure item 5 in §1.3.
- **Feed health events:** connect, disconnect, protocol mismatch and backoff, all in the same log.

---

## 10. Failure modes

| Failure | Behavior |
|---|---|
| Herdr socket missing or refused | The feed goes offline: its rows are kept but dimmed, with a warning glyph on the pill. Reconnect with backoff. The launch guard applies on reconnect. |
| Herdr `protocol ≠ 22` or an unsupported method | The feed is disabled, with a warning glyph and a log line. The island keeps running on its other feeds. |
| Silent dead subscription (#3124) | The 12 s reconcile repairs state and reopens the stream. |
| Codex sessions dir missing or unreadable | The feed stays idle; the warning glyph shows only after Codex.app has been seen running. |
| Registry dir missing | The feed stays idle, with no warning. |
| Island crash | LaunchAgent `KeepAlive` restarts it. The launch guard prevents a chime storm, and the Codex baseline prevents a flood. |
| Display change or wake | Wait 0.35 s, then re-anchor on the current primary display. |

---

## 11. Security and privacy

- **Read-only on agent state.** It writes only its own files: the seen store, the log, preferences and the LaunchAgent plist.
- It never reads `~/.claude/sessions/*.key`, and makes no network calls or telemetry.
- Question text appears on screen in the card. There is no redaction mode in v1; it would be added only if screen-sharing makes it necessary.
- The local log holds truncated question text and stays under `~/.local/state`.

---

## 12. Verification

### 12.1 Unit and fixture tests (`island-tests`, run on every build)

**Herdr:**
- Event name normalization, both dotted and underscored.
- Bootstrap order.
- A per-pane stream opens on `pane_created` and closes on `pane_closed`.
- An atomic rejection affects only that pane's stream.
- **fd leak:** 100 reconnect cycles against the fake socket leave open fds at ≤ panes + 2.
- Reconcile repairs a dropped event and reopens the silent stream.
- Blocked for less than 1 s gives no peek. The recorded 0.3 s flicker gives one peek and one chime.
- Episodes, the 3 s gap and the launch guard.
- `done` followed by a view goes to `idle`.
- The finished-while-away overlay: set when Ghostty is not frontmost; cleared on focus.
- The `unknown` status is never counted. A pane becomes stale after 30 min with an unchanged sequence number.
- The user-close exception to the error rule.

**Fixture replay.** Masked copies of `sub2-events.log` and `poll.log` are replayed through an in-process fake Herdr Unix-socket server. The test asserts the pill-summary timeline.

**Detection parser.** Masked `read-blocked-detection.txt` and `read-blocked2-det.txt` must yield the question and its options. The done fixture must yield the recap. Garbage input falls back to "needs you".

**Claude registry:**
- The liveness check with a `procStart` mismatch.
- Filters for `kind` and `sdk-*`.
- Deduplication against Herdr pids.
- The `waitingFor` → `Detail.kind` mapping.
- busy → idle gives `doneUnseen`.
- `*.key` files are never opened: a fixture directory contains a `.key` file, and a file-access spy asserts it is never read.

**Codex:**
- `source` decodes as a string and as an object.
- guardian, subagent and exec threads are filtered.
- `turn_id` pairing.
- An open `request_user_input` gives `waiting`, and its output clears it.
- The `_async` heuristic.
- `turn_aborted` gives `idle`.
- **Cold start on a synthetic file larger than 200 MB with a turn in progress:** `session_meta` is found, the in-progress turn is `working`, and it completes in under 300 ms.
- Titles, the seen-store baseline, and marking seen on activation.

**Display and jump:**
- `.primary` returns the CG main display wherever the pointer is.
- Samsung geometry: 23 pt pill with a 4 pt gap inside the 31 pt bar. Notch geometry is unchanged.
- The card resolves to the pointer's display.
- `interactiveRegion` is correct at the pill edges.
- Herdr jump actions are `[agent.focus, raiseGhostty, fallback]`. The Codex jump uses the running bundle URL.

**Static guards.** The build fails on any match in `Sources/`:
- `UNUserNotificationCenter`, `NSUserNotification`, `display notification`, `terminal-notifier`
- `.claude/settings`, `.codex/hooks`, `.codex/config`, `herdr/config`
- `send_keys`, `agent.prompt`, `plugin.link`

### 12.2 Live contract test

Opt in with `HERDR_CONTRACT=1`. It is read-only; rerun it after every Herdr update.
- `ping` reports protocol 22.
- `session.snapshot` decodes.
- G plus one per-pane subscription is acknowledged.
- `agent.read detection` on a pane returns text.
- `pane.process_info` returns pids.
- An unknown method maps to "unsupported".

### 12.3 Scripted end-to-end run (in CI or locally, before any live check)

Launch the **built app** against the fake Herdr server, fixture registry and rollout directories, with `AGENT_ISLAND_STATE_DUMP` and `AGENT_ISLAND_JUMP_DRY_RUN=1` set. Then:

1. Drive the Herdr events, registry changes and rollout appends for blocked, done, working, pane-exit, user-close and Codex question.
2. Assert from the state dump: pill text, peek queue, `chime.playedCount`, and planned jump actions.
3. Drive hover, expand and collapse through synthetic mouse events while `ignoresMouseEvents` toggles, and assert the board state.
4. Idle CPU averages under 1 % (`ps -o %cpu`). `lsof -p` fds stay stable over 10 min.
5. The Notification Center audit shows zero records from `com.nathan.agent-island`.

### 12.4 Notification Center audit (the repeatable acceptance test for "separated")

`scripts/nc-agent-audit.py`, moved in from the research bundle. It:
- copies `~/Library/Group Containers/group.com.apple.usernoted/db2/{db,db-wal,db-shm}` to a temp dir and opens the copy with `mode=ro`;
- selects records with `coalesce(delivered_date, request_date) >= --since`;
- counts records from `com.openai.codex`, `com.mitchellh.ghostty`, `com.anthropic.claudefordesktop`, `com.nathan.agentsignalsnotifier` and `com.nathan.agent-island`, plus terminal-notifier and Script Editor records with agent words in them;
- exits 1 if any are found.

The database only keeps undismissed records, so during the trial period it runs every 10 min with `--log ~/.local/state/agent-island/nc-audit.jsonl` and unions the results by uuid.

### 12.5 Live acceptance (Nathan, about 30 min, after the Herdr server is on 0.9.1)

1. **Samsung.** The pill is inside the menu bar at the top center and covers no menu items. A click on a Ghostty tab or title bar just under the pill works, both collapsed and expanded, as long as it is outside the card. Hovering expands the board, and moving away collapses it.
2. **Lid only.** The pill hugs the notch.
3. **Background question** in an unviewed Herdr pane. Within 2 s there is one card, on the pointer's display, with the question, and exactly one chime.
4. **Row click.** It lands on the right pane in the right Ghostty window.
5. **Question while watching** that pane with Ghostty frontmost. No card and no chime; only the count changes.
6. **Background finish.** "1 done" appears silently. Viewing the pane in the Herdr TUI drops the count within 12 s.
7. **Finish while away.** Start a turn in the **focused** pane, switch to another app, and wait for it to finish. "1 done" appears.
8. **Codex Desktop finish.** The done count rises silently. Clicking the row opens **that exact thread**, which verifies the `codex://threads/<id>` id.
9. **Codex Plan-mode question.** One card and one chime.
10. **Claude outside Herdr** (salvo/tmux session). A question produces one card and one chime, and the row jumps. If a Desktop Code-tab session is available, check that it appears (this verifies §15 item 3).
11. **Display changes.** Unplug and replug the Samsung, sleep and wake, and enter a full-screen Space. The pill re-anchors on the primary display.
12. **24 h trial run.** The transition log shows no false chimes. CPU and fds stay stable. `grep herdr::sound` in `herdr-client.log` shows no new entries. The Notification Center audit is clean for the stage in effect.

---

## 13. Cutover and agent-signals teardown

**Rules:**
- The island runs **alongside** before anything is silenced. The one exception is Herdr's own sounds (Stage 1a).
- Every step is a file move or a one-line config change that can be reverted.
- **`/say` must keep working.** `/say` and the cockpit Stop hook need `~/.local/bin/agent-signal` (`speak-last`, `voice`) and `kokoro-speak`. TTS shells into the agent-signals checkout through a path baked in at compile time (`lib.rs:2227-2243`), so **the repo must stay at `~/Developer/proj/agent-signals`**. **Do not run `scripts/uninstall-native.sh` as written:** it deletes `agent-signal` and `kokoro-speak`.

**Stage 0 — before the island ships**
- Record a baseline with `nc-agent-audit.py --since <now>`.
- Preserve the uncommitted `notification-rework` work (`lib.rs`, the untracked `app_config.rs`) as a WIP commit on that branch, **only after confirming that no other agent session is working in that checkout**. Park the branch; do not merge it.
- Capture a Black Box decision: "agent-signals notifications retired in favor of agent-island; repo keeps voice/TTS".

**Stage 1a — at the island's first launch.** These are the only switches that don't wait for the side-by-side run. Herdr's sounds are exactly the sound-without-card failure, and live checks 3, 5 and 6 can't be judged while they play.

| Surface | Change | Revert |
|---|---|---|
| Herdr sound (**required**) | `[ui.sound] enabled = false` in `~/dotfiles/home/.config/herdr/config.toml`. The default is on, and today it plays a "Done" sound in background workspaces with no card. That breaks success criterion 5. | Remove the line. |
| Herdr toasts | `[ui.toast] delivery = "off"`. The current `"terminal"` value is an **uncommitted edit by another agent**: coordinate before editing. | `"terminal"` |

**Stage 1b — after live checks 1–11 pass**

| Surface | Change | Revert |
|---|---|---|
| Codex Desktop turn banners | Settings → Notifications → turn completions **Off**, which sets `[desktop] notifications-turn-mode = "off"` (currently `"always"`, `config.toml:1329`). Confirm it still reads `off` after Codex restarts. | `"always"` or `"unfocused"` |
| Claude Desktop "finished" | Settings, or `preferences.notificationLevels.idle = "off"` (or `"badge"`). | Delete the key. |

**Stage 2 — after the 24 h trial run shows the island caught Codex and Claude questions**
- Codex Desktop: `notifications-questions-enabled = false` and `notifications-permissions-enabled = false`. Permission banners never fire under `approval_policy = never`.
- Claude Desktop: permission and question → `"badge"` or `"off"`.
- **Acceptance:** zero agent records in the Notification Center audit for 24 h.

**Stage 3 — retire agent-signals' notification parts**

Done by the agent, with Nathan's go-ahead:
1. Add `scripts/retire-notifications.sh` to agent-signals. It must be idempotent and support `--restore`. It moves the following into `~/.local/state/agent-signals/retired-<date>/`:
   - both LaunchAgent plists (after a no-op `launchctl bootout`);
   - `~/Applications/AgentSignalsNotifier.app`;
   - `~/.local/bin/{agent-signald, agent-response-notify, agent-focus-pane, agent-signal.agent-signals-disabled-20260601-real, agent-response-notify.agent-signals-disabled-20260601}`;
   - the `~/Developer/config/voice/bin/{agent-response-notify, agent-focus-pane}` symlinks, if present.
2. **Keep:** `agent-signal`, `agent-speak-last`, `agent-voice-stop`, `voice-toggle`, `kokoro-speak`, the repo `.venv`, `src/agent_signals`, and `config/voice-rules.json`.
3. Make `install-native.sh` **voice-only**: it builds `agent-signal` and the voice wrappers, and no daemon, notifier or plists. Make `--keep-voice` the default for `uninstall-native.sh`. Add a README pointer: "notifications moved to agent-island; this repo provides voice". Then run `scripts/install-native.sh`, because merging alone does not deploy.
4. Update consumers of the frozen `~/.local/state/agent-signals` active-notifications: `terminal-tmux/tmux-sessionizer` and `tmux-session-face` badges. Either drop them or point them at `herdr agent list --json`.
5. Update the docs that mention the old services: `dotfiles/docs/live-services.md` and the VS Code log-tail task.
6. **Later, in a separate PR** after a TTS regression test: delete the notify, focus, clear and sweep subcommands and the daemon code, the `agent-signald` bin, `native/swift`, `launchd/`, and Python `channels/`. Or move TTS out entirely.

**Check `/say` after every stage:**
- `agent-signal speak-last --dry-run --force --previous "$(ls -t ~/.claude/projects/*/*.jsonl | head -1)"` prints JSON.
- `agent-signal voice status` prints a status line.
- `/say` is audible (Nathan).

**Leave alone unless the audit shows records from them:**
- Codex `notify` (the SkyComputerUseClient → `agent-notify --codex` chain).
- Claude Code `preferredNotifChannel: "auto"`. It is a cockpit symlink, and editing it dirties cockpit.
- `com.nathan.agent-pet`: its LaunchAgent is RunAtLoad but it is not running. It is a separate desktop surface and is out of scope.
- `agent-observatory`: running. It already indexes Codex rollouts and is prior art if the Codex parser needs cross-checking.

---

## 14. Estimate and execution

**Agent time.** Claude subagents implement from the plan:
- **Sonnet** for well-specified tasks;
- **Opus** for the Herdr client and feed, `InterruptPolicy`, and click-through and hover.

Each task is verified by a different model than the one that implemented it. Codex was out of usage on 2026-09-25. Once it has capacity again, Codex can take over any task that hasn't started, with Claude verifying.

| # | Item | Hours |
|---|---|---|
| 1 | Bootstrap the repo from Moonglade `be0c5b4`, strip the data plane, rename the bundle id and paths, delete sounds, installer and metal, bring the kept tests to green | 2 |
| 2 | Feed seam: `SessionFeed`, repurposed `SessionSource`, `StateStore` merge and dedup, `focus(rowID)`, remove the file re-read and Kill | 2–2.5 |
| 3 | Herdr feed: ported codec with the `target` fix, fd-safe client, bootstrap, reconcile, ping, backoff, `process_info`, reducer, error rule, away overlay, fake-socket fixtures | 5–6 |
| 4 | Detection text parser with masked fixtures | 1 |
| 5 | Claude registry feed: liveness, filters, dedup, mapping, seen flag, jump variants, `.key` guard | 1.5–2 |
| 6 | Codex Desktop feed: decoder, filters, pairing, open `request_user_input`, `_async`, first-line and backward-scan cold start, FSEvents and poll, titles, seen store, observers | 4–4.5 |
| 7 | `DisplayState`, summary, pill labels, grouped compact rows, stale aging, feed-offline glyph | 2 |
| 8 | `InterruptPolicy`, `PeekController` (card on the pointer display), `ChimePlayer` | 3.5–4.5 |
| 9 | Jump: `agent.focus`, `GhosttyRaiser`, Codex and Claude URLs, tmux switch | 1.5–2 |
| 10 | Display: `.primary`, monitor-driven click-through **and hover**, settle | 3–4 |
| 11 | Transition log, `StateDump`, packaging (build, install, uninstall, LaunchAgent), README | 1.5 |
| 12 | Verification: static guards, contract test, scripted end-to-end run, CPU and fd sampling, Notification Center audit script | 2–3 |
| 13 | agent-signals Stage 3: retire script, voice-only install, uninstall guard, README, reinstall, `/say` check | 1 |

**Total: about 30–37 agent-hours.**

**Critical path:** 1 → 2 → 3 → 7/8 → 12. After item 2 lands, items 3, 5, 6 and 10 can run in separate worktrees, giving about 1.5–2 days elapsed.

**Nathan's hands** (about 1–1.25 h, plus the 24 h trial run):
1. **Upgrade the Herdr server from 0.9.0 to 0.9.1.** The installed binary is 0.9.1, but the running server reports `server_binary_stale`. Use either `herdr update --handoff` (experimental) or a planned restart, which ends the live agents. It is needed so that `agent.focus` moves the attached TUI and TUI navigation emits focus events.
2. **Optional:** create a self-signed code-signing certificate in Keychain Access (about 10 min), so the Ghostty Automation grant survives rebuilds.
3. **First launch:** approve the Ghostty Automation prompt on the first jump. A locally built app carries no quarantine flag, so no Gatekeeper prompt is expected. If one appears, use Privacy & Security → Open Anyway.
4. Live acceptance checks 1–11 (about 30 min).
5. Stage 1a, 1b and 2 switches (about 10 min in total), and a go-ahead for Stage 3.

---

## 15. Risks and items to verify during implementation

**Risks:**

1. **Herdr and Claude status comes from screen detection,** using remotely updated manifests (e.g. `claude.toml 2026.09.11.1`). Expect flicker, stuck "working" (one pane sat at working for more than 50 min) and brief `unknown` states. The debounce, episode rules and aging reduce this but do not remove it. The transition log, including the registry shadow, measures it during the trial run.
2. **The Herdr server upgrade is a prerequisite** for jumping and seen semantics. The seen behavior was measured on 0.9.0; the 0.9.1 docs describe seen tracking per explicit focus and per TUI client. Live checks 6 and 7 re-verify this after the upgrade, and the away overlay is the fallback.
3. **Codex has no approval signal** in this setup. The `_async` heuristic cannot tell when a question was dismissed. This is why Stage 2 waits for the trial run.
4. **Undocumented formats:** Codex rollout JSONL, `session_index.jsonl`, the `~/.claude/sessions` registry, and `codex://` / `claude://` deep links. These are mitigated by contract tests and pinned fixtures, and the feeds degrade to offline rather than crash.
5. **Click-through depends on WindowServer behavior plus the `ignoresMouseEvents` toggle.** The private `CABackdropLayer` glass may break in a macOS update; the public fallback exists.
6. **Turning off Codex turn banners removes the only redundant completion signal.** The Notification Center audit and the state dump during the trial run are the backstop.
7. **The fork diverges from upstream completely.** About 8k LOC becomes Nathan's to maintain.

**Verify during implementation** (not decisions):

1. That the id in `codex://threads/<id>` is the rollout `session_meta.id` (live check 8).
2. The `claude://code/continue?session=<id>` id format.
3. Whether Desktop Code-tab sessions register in `~/.claude/sessions`.
4. The shape of the registry `tmux` field for `switch-client`.
5. Herdr 0.9.1 seen semantics (live checks 6 and 7).
6. Whether the Codex `error` and `stream_error` event names exist.
7. Whether monitor-driven hover works with `ignoresMouseEvents` toggling (end-to-end step 3 and live check 1).

---

## 16. Out of scope for v1 (candidates for later)

- **Answering approvals or questions from the island.** If wanted, route it through Herdr `agent.send_keys` with a re-check of state just before sending. No blocking hooks.
- **The Claude registry as status authority or tie-breaker** for Herdr panes. Decided by the trial-run shadow data.
- **Full message text for "done"** (transcript tailing; `transcript.rs` and the captured payloads in agent-signals are the reference).
- Remote Herdr machines (`--machine` / `--remote`).
- Mirroring macOS Focus modes, a hide-in-full-screen setting, and a redaction mode.
- Auto-update and notarization.
- Moving TTS out of agent-signals.

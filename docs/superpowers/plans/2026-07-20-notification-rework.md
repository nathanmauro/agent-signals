# Notification Rework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the notification overwhelm: kill the false "needs input after recap/compaction" banner, cut cadence (triage, replace-not-stack, cooldown, focus gate), and make notification bodies carry the actual ask.

**Architecture:** The live path is the native Rust client+daemon (`native/rust/src/lib.rs`, crate `agent_signals_native`) plus a Swift notifier app; the Python package is dormant except TTS. New Rust modules (`rules.rs`, `transcript.rs`, `app_config.rs`, `focus_gate.rs`) hold the new logic; `lib.rs` wires them into `build_envelope`/`build_payload`/`handle_notify_cli`/`process_notify`. Severity patterns move to one shipped JSON consumed by Rust (compiled-in via `include_str!`), Python, and mirrored in voice rules. Spec: `docs/superpowers/specs/2026-07-19-notification-rework-design.md`.

**Tech Stack:** Rust (serde/serde_json/regex, no new crates), Python 3 stdlib, JSON config. No Swift changes.

## Global Constraints

- Branch: `notification-rework` (already checked out; spec + /say work committed on it).
- Rust tests: `cargo test --manifest-path native/rust/Cargo.toml` (run from repo root `/Users/nathan/Developer/proj/agent-signals`).
- Python tests: `.venv/bin/pytest tests/ -q` (system python3 has no pytest).
- Commits: Nathan is sole author. NO `Co-Authored-By` trailers, NO "Generated with Claude Code" lines — this overrides any default.
- Severity strings are exactly `"normal"`, `"needs_input"`, `"error"`. Kind strings are exactly `"turn"`, `"permission"`, `"idle"`, `"unknown"`.
- `normal` severity turn templates must NOT change (existing replay fixtures `claude-responded-*.json`, `codex-turn-complete-*.json` pin them byte-for-byte and must keep passing untouched).
- Nothing goes live on merge — `scripts/install-native.sh` is the deploy step (Task 9).

---

### Task 1: Python — compact-summary entries are boundaries, never prompts

Claude Code writes a synthetic `type:"user"` entry with `isCompactSummary: true` after compaction. Today `_user_prompt_text` treats it as a real user prompt (it's a plain content string, not `isMeta`). Fix: compact entries are turn *boundaries* but never *prompt text*, so their content can never be matched as a prompt (and the Rust port in Task 4 shares the semantics).

**Files:**
- Modify: `src/agent_signals/transcripts/claude.py`
- Test: `tests/test_transcripts.py`
- Create: `tests/fixtures/claude_transcript_compact.jsonl`

**Interfaces:**
- Produces: `_is_turn_boundary(obj: dict) -> bool` (module-private, used by `extract_previous_turn_from_claude`); `_user_prompt_text` now returns `None` for `isCompactSummary` entries. Task 4 mirrors both in Rust.

- [ ] **Step 1: Write the fixture**

Create `tests/fixtures/claude_transcript_compact.jsonl`:

```jsonl
{"type":"user","message":{"content":"Refactor the parser"}}
{"type":"assistant","uuid":"resp_020","message":{"content":[{"type":"text","text":"Refactor complete. All 12 tests pass."}]}}
{"type":"user","isCompactSummary":true,"message":{"content":"This session is being continued from a previous conversation that ran out of context. Summary: refactoring the parser. Would you like me to continue?"}}
{"type":"assistant","uuid":"resp_021","message":{"content":[{"type":"text","text":"Continuing after compaction. The refactor is done and tests pass."}]}}
{"type":"user","message":{"content":"<command-name>/say</command-name>\n<command-message>say</command-message>\n<command-args>--previous</command-args>"}}
{"type":"assistant","uuid":"resp_022","message":{"content":[{"type":"text","text":"Reading the last turn aloud."}]}}
```

- [ ] **Step 2: Write the failing tests**

Append to `tests/test_transcripts.py`:

```python
def test_compact_summary_is_not_a_user_prompt():
    from agent_signals.transcripts.claude import _is_turn_boundary, _user_prompt_text

    compact = {
        "type": "user",
        "isCompactSummary": True,
        "message": {"content": "This session is being continued. Would you like me to continue?"},
    }
    assert _user_prompt_text(compact) is None
    assert _is_turn_boundary(compact)


def test_previous_turn_skips_compact_summary_prompt():
    from agent_signals.transcripts.claude import extract_previous_turn_from_claude

    path = FIXTURES / "claude_transcript_compact.jsonl"
    text, key = extract_previous_turn_from_claude(path)
    assert text == "Continuing after compaction. The refactor is done and tests pass."
    assert key == "resp_021"
```

(`FIXTURES` already exists at the top of the file. If it doesn't, add `FIXTURES = Path(__file__).parent / "fixtures"` with `from pathlib import Path`.)

- [ ] **Step 3: Run tests to verify failure**

Run: `.venv/bin/pytest tests/test_transcripts.py -q`
Expected: FAIL — `_is_turn_boundary` does not exist (ImportError), and/or `_user_prompt_text(compact)` returns the content string, not None.

- [ ] **Step 4: Implement**

In `src/agent_signals/transcripts/claude.py`, change the first guard of `_user_prompt_text` from:

```python
    if obj.get("type") != "user" or obj.get("isMeta"):
        return None
```

to:

```python
    if obj.get("type") != "user" or obj.get("isMeta") or obj.get("isCompactSummary"):
        return None
```

Add below `_user_prompt_text`:

```python
def _is_turn_boundary(obj: dict) -> bool:
    """A compact summary starts a new logical stretch even though it is not a
    real prompt; a real user prompt always does."""
    if obj.get("type") == "user" and obj.get("isCompactSummary"):
        return True
    return _user_prompt_text(obj) is not None
```

In `extract_previous_turn_from_claude`, change the boundary loop from:

```python
    boundary = len(objs)
    for i in range(len(objs) - 1, -1, -1):
        if _user_prompt_text(objs[i]) is not None:
            boundary = i
            break
```

to:

```python
    boundary = len(objs)
    for i in range(len(objs) - 1, -1, -1):
        if _is_turn_boundary(objs[i]):
            boundary = i
            break
```

(The backward walk below the boundary already handles compact entries correctly: `_user_prompt_text` now returns `None` for them, so the walk passes through instead of treating their content as a prompt.)

- [ ] **Step 5: Run the full Python suite**

Run: `.venv/bin/pytest tests/ -q`
Expected: all pass (43 existing + 2 new).

- [ ] **Step 6: Commit**

```bash
git add src/agent_signals/transcripts/claude.py tests/test_transcripts.py tests/fixtures/claude_transcript_compact.jsonl
git commit -m "transcripts: treat compact summaries as boundaries, not prompts"
```

---

### Task 2: One severity-rules source — `config/severity-rules.json` + Python loader

Three drifted pattern copies exist (`lib.rs:854/863`, `policy.py:9-15`, `config/voice-rules.json`). Create the canonical file; make Python read it with a closing-window; sync voice rules. (Rust consumes it in Task 3.)

**Files:**
- Create: `config/severity-rules.json`
- Modify: `src/agent_signals/policy.py`
- Modify: `config/voice-rules.json:12-24` (the `needs_attention` and `needs_input` pattern lists)
- Test: `tests/test_policy.py`

**Interfaces:**
- Produces: `config/severity-rules.json` with keys `version` (int), `closing_window_chars` (int), `needs_input_patterns` (list[str]), `error_patterns` (list[str]). Task 3's Rust loader reads this exact shape via `include_str!`.
- Python `classify_severity(text: str) -> str` keeps its signature; patterns now come from the file (fallback to built-ins on read error) and match only the closing window.

- [ ] **Step 1: Create the canonical rules file**

Create `config/severity-rules.json`:

```json
{
  "version": 1,
  "closing_window_chars": 300,
  "needs_input_patterns": [
    "\\?\\s*$",
    "(?i)\\b(please confirm|which (one|option|approach)|do you want|would you like|want me to|shall i|need your (input|decision|approval|sign-?off)|waiting for your|your call|up to you|let me know)\\b"
  ],
  "error_patterns": [
    "(?i)\\b(failed|failure|error|traceback|exception|permission denied|fatal)\\b"
  ]
}
```

Deliberately pruned vs today: error loses `blocked`, `unable to`, `could not`, `can't` (benign prose that was punching through DND); needs_input loses `send me`, `tell me`, bare `choose` (sign-off boilerplate). Both lists now apply only to the last `closing_window_chars` characters.

- [ ] **Step 2: Write the failing tests**

Append to `tests/test_policy.py`:

```python
def test_severity_rules_file_is_the_source():
    from agent_signals.policy import _load_severity_rules

    rules = _load_severity_rules()
    assert rules["closing_window_chars"] == 300
    assert any("please confirm" in p for p in rules["needs_input_patterns"])


def test_benign_prose_is_not_an_error():
    # "could not" / "can't" / "unable to" / "blocked" are pruned from error.
    assert classify_severity("The old approach could not scale, so I rewrote it.") == "normal"
    assert classify_severity("I can't say this is elegant, but it works and tests pass.") == "normal"


def test_needs_input_must_be_in_the_closing_window():
    body = "Do you want the long version? I went ahead with the short one.\n" + ("Details of the change. " * 40)
    assert classify_severity(body) == "normal"


def test_closing_question_still_classifies():
    body = ("Context paragraph. " * 40) + "\nWhich approach do you want — JWT or sessions?"
    assert classify_severity(body) == "needs_input"
```

- [ ] **Step 3: Run tests to verify failure**

Run: `.venv/bin/pytest tests/test_policy.py -q`
Expected: FAIL — `_load_severity_rules` missing; "could not scale" classifies as `error`; the early-question body classifies as `needs_input`.

- [ ] **Step 4: Implement the Python loader**

In `src/agent_signals/policy.py`, replace the two module-level pattern constants (lines 9-15) with:

```python
_FALLBACK_RULES: dict = {
    "closing_window_chars": 300,
    "needs_input_patterns": [
        r"\?\s*$",
        r"(?i)\b(please confirm|which (one|option|approach)|do you want|would you like|want me to|shall i|need your (input|decision|approval|sign-?off)|waiting for your|your call|up to you|let me know)\b",
    ],
    "error_patterns": [
        r"(?i)\b(failed|failure|error|traceback|exception|permission denied|fatal)\b",
    ],
}
_RULES_PATHS = [
    Path.home() / ".config" / "agent-signals" / "severity-rules.json",
    Path(__file__).resolve().parents[2] / "config" / "severity-rules.json",
]


def _load_severity_rules() -> dict:
    for path in _RULES_PATHS:
        try:
            data = json.loads(path.read_text())
        except Exception:
            continue
        if isinstance(data, dict) and data.get("needs_input_patterns"):
            return {
                "closing_window_chars": int(data.get("closing_window_chars", 300)),
                "needs_input_patterns": list(data.get("needs_input_patterns", [])),
                "error_patterns": list(data.get("error_patterns", [])),
            }
    return _FALLBACK_RULES
```

Add `from pathlib import Path` to the imports. Replace the body of `classify_severity` with:

```python
def classify_severity(text: str) -> str:
    rules = _load_severity_rules()
    window = text.strip()[-rules["closing_window_chars"]:]
    for pattern in rules["error_patterns"]:
        try:
            if re.search(pattern, window):
                return Severity.ERROR
        except re.error:
            continue
    for pattern in rules["needs_input_patterns"]:
        try:
            if re.search(pattern, window):
                return Severity.NEEDS_INPUT
        except re.error:
            continue
    return Severity.NORMAL
```

Note: `parents[2]` from `src/agent_signals/policy.py` is the repo root (`[0]`=`agent_signals`, `[1]`=`src`, `[2]`=repo).

- [ ] **Step 5: Sync `config/voice-rules.json`**

In `config/voice-rules.json`, replace the `needs_attention` rule's `patterns` array with:

```json
      "patterns": [
        "(?i)\\b(failed|failure|error|traceback|exception|permission denied|fatal)\\b"
      ],
```

and the `needs_input` rule's `patterns` array with:

```json
      "patterns": [
        "\\?\\s*$",
        "(?i)\\b(please confirm|which (one|option|approach)|do you want|would you like|want me to|shall i|need your (input|decision|approval|sign-?off)|waiting for your|your call|up to you|let me know)\\b"
      ],
```

- [ ] **Step 6: Run the full Python suite**

Run: `.venv/bin/pytest tests/ -q`
Expected: all pass. (Existing classify tests use messages whose triggers survive: "build failed" → `failed`; "Which approach would you prefer?" → `\?\s*$`; "The build failed." voice rule → `failed`.)

- [ ] **Step 7: Commit**

```bash
git add config/severity-rules.json config/voice-rules.json src/agent_signals/policy.py tests/test_policy.py
git commit -m "policy: unify severity patterns in severity-rules.json with closing-window matching"
```

---

### Task 3: Rust `rules.rs` — load severity rules, closing-window classify

**Files:**
- Create: `native/rust/src/rules.rs`
- Modify: `native/rust/src/lib.rs` (add `pub mod rules;` near the top; replace `classify_severity` body at lib.rs:850-870 with a delegation)
- Test: unit tests inside `rules.rs`

**Interfaces:**
- Produces: `pub struct SeverityRules` with `pub fn compiled_default() -> &'static SeverityRules` (lazy static via `OnceLock`), `pub fn load() -> SeverityRules` (user override `~/.config/agent-signals/severity-rules.json`, else compiled default), `pub fn classify(&self, text: &str) -> String`. Tasks 5–6 call `rules.classify(...)`; `handle_notify_cli` calls `SeverityRules::load()`.

- [ ] **Step 1: Write `native/rust/src/rules.rs` with failing tests included**

```rust
use regex::Regex;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

const COMPILED_RULES: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/severity-rules.json"));

#[derive(Debug, Deserialize)]
struct RawRules {
    #[serde(default = "default_window")]
    closing_window_chars: usize,
    #[serde(default)]
    needs_input_patterns: Vec<String>,
    #[serde(default)]
    error_patterns: Vec<String>,
}

fn default_window() -> usize {
    300
}

#[derive(Debug)]
pub struct SeverityRules {
    closing_window_chars: usize,
    needs_input: Vec<Regex>,
    error: Vec<Regex>,
}

impl SeverityRules {
    fn from_json(text: &str) -> Option<Self> {
        let raw: RawRules = serde_json::from_str(text).ok()?;
        if raw.needs_input_patterns.is_empty() {
            return None;
        }
        let compile = |patterns: &[String]| {
            patterns
                .iter()
                .filter_map(|p| Regex::new(p).ok())
                .collect::<Vec<_>>()
        };
        Some(Self {
            closing_window_chars: raw.closing_window_chars,
            needs_input: compile(&raw.needs_input_patterns),
            error: compile(&raw.error_patterns),
        })
    }

    pub fn compiled_default() -> &'static SeverityRules {
        static RULES: OnceLock<SeverityRules> = OnceLock::new();
        RULES.get_or_init(|| {
            SeverityRules::from_json(COMPILED_RULES)
                .expect("compiled-in config/severity-rules.json must be valid")
        })
    }

    /// User override at ~/.config/agent-signals/severity-rules.json, else the
    /// compiled-in copy of config/severity-rules.json.
    pub fn load() -> SeverityRules {
        let user_path = user_rules_path();
        if let Ok(text) = fs::read_to_string(&user_path) {
            if let Some(rules) = SeverityRules::from_json(&text) {
                return rules;
            }
        }
        SeverityRules::from_json(COMPILED_RULES)
            .expect("compiled-in config/severity-rules.json must be valid")
    }

    /// Only the closing window of a message decides severity: a question
    /// buried mid-message is context, one at the end is a handoff.
    pub fn classify(&self, text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return "normal".to_string();
        }
        let window = closing_window(trimmed, self.closing_window_chars);
        if self.error.iter().any(|re| re.is_match(window)) {
            return "error".to_string();
        }
        if self.needs_input.iter().any(|re| re.is_match(window)) {
            return "needs_input".to_string();
        }
        "normal".to_string()
    }
}

fn user_rules_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".config/agent-signals/severity-rules.json")
}

fn closing_window(text: &str, chars: usize) -> &str {
    let count = text.chars().count();
    if count <= chars {
        return text;
    }
    let skip = count - chars;
    match text.char_indices().nth(skip) {
        Some((idx, _)) => &text[idx..],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_question_is_needs_input() {
        let rules = SeverityRules::compiled_default();
        let body = format!(
            "{}\nWhich approach do you want — JWT or sessions?",
            "Context paragraph. ".repeat(40)
        );
        assert_eq!(rules.classify(&body), "needs_input");
    }

    #[test]
    fn early_question_is_normal() {
        let rules = SeverityRules::compiled_default();
        let body = format!(
            "Do you want the long version? I went ahead with the short one.\n{}",
            "Details of the change. ".repeat(40)
        );
        assert_eq!(rules.classify(&body), "normal");
    }

    #[test]
    fn benign_prose_is_not_error() {
        let rules = SeverityRules::compiled_default();
        assert_eq!(
            rules.classify("The old approach could not scale, so I rewrote it."),
            "normal"
        );
        assert_eq!(
            rules.classify("I can't say it is elegant, but it works and tests pass."),
            "normal"
        );
    }

    #[test]
    fn real_failure_is_error() {
        let rules = SeverityRules::compiled_default();
        assert_eq!(
            rules.classify("The build failed: linker error, could not resolve symbol."),
            "error"
        );
    }

    #[test]
    fn signoff_boilerplate_without_closing_ask_is_normal() {
        let rules = SeverityRules::compiled_default();
        // "tell me" / "send me" / bare "choose" are pruned.
        assert_eq!(
            rules.classify("Done. Tell me about any issues via the tracker."),
            "normal"
        );
    }

    #[test]
    fn closing_window_respects_char_boundaries() {
        // Multibyte chars near the cut must not panic.
        let text = format!("{}é continue?", "x".repeat(400));
        let rules = SeverityRules::compiled_default();
        assert_eq!(rules.classify(&text), "needs_input");
    }
}
```

- [ ] **Step 2: Register the module and delegate `classify_severity`**

In `native/rust/src/lib.rs`, add after the `use` block (below line 17):

```rust
pub mod rules;

use rules::SeverityRules;
```

Replace the whole `fn classify_severity` (lib.rs:850-870) with:

```rust
fn classify_severity(text: &str) -> String {
    SeverityRules::compiled_default().classify(text)
}
```

- [ ] **Step 3: Run the Rust tests**

Run: `cargo test --manifest-path native/rust/Cargo.toml`
Expected: PASS — new `rules::tests` pass; all existing lib tests and `replay_real_events` keep passing (the pruned patterns only shrink matches; every fixture's triggering phrase survives: `failed_turn_is_actionable` matches `failed`, the four `turn_that_asks_a_question_is_actionable` messages match `\?\s*$` / `need your decision` / `let me know`).

- [ ] **Step 4: Commit**

```bash
git add native/rust/src/rules.rs native/rust/src/lib.rs
git commit -m "native: load severity rules from severity-rules.json with closing-window classify"
```

---### Task 4: Rust `transcript.rs` — compaction-aware turn reading

Port Task 1's boundary semantics to Rust for the notify path: find the last turn boundary (real prompt or compact summary), extract the completed turn's final assistant text (skipping meta/sidechain entries), and flag compact-boundary and in-flight states.

**Files:**
- Create: `native/rust/src/transcript.rs`
- Modify: `native/rust/src/lib.rs` (add `pub mod transcript;`)
- Test: unit tests inside `transcript.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct ClaudeTurn {
      pub text: String,              // final assistant text after the last boundary ("" if none)
      pub boundary_is_compact: bool, // last boundary is an isCompactSummary entry
      pub in_flight: bool,           // transcript does not end with assistant text
  }
  pub fn read_claude_turn(transcript_text: &str) -> ClaudeTurn
  ```
  Task 5 calls `read_claude_turn` from `classify_event`.

- [ ] **Step 1: Write `native/rust/src/transcript.rs`**

```rust
use serde_json::Value;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ClaudeTurn {
    pub text: String,
    pub boundary_is_compact: bool,
    pub in_flight: bool,
}

/// Read the current turn out of a Claude Code transcript at Stop/Notification
/// time. Entries flagged `isMeta`, `isCompactSummary`, or `isSidechain` are
/// never treated as prompts or assistant output; a compact summary still marks
/// a boundary so a Stop right after compaction can be recognized and dropped.
pub fn read_claude_turn(transcript_text: &str) -> ClaudeTurn {
    let objs: Vec<Value> = transcript_text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|obj| obj.is_object())
        .collect();

    let boundary = objs.iter().rposition(is_turn_boundary);
    let boundary_is_compact = boundary
        .map(|idx| is_compact_summary(&objs[idx]))
        .unwrap_or(false);
    let after = boundary.map(|idx| idx + 1).unwrap_or(0);

    let mut text = String::new();
    for obj in objs[after..].iter().rev() {
        if is_skippable(obj) {
            continue;
        }
        if obj.get("type").and_then(Value::as_str) == Some("assistant") {
            let t = assistant_text(obj);
            if !t.is_empty() {
                text = t;
                break;
            }
        }
    }

    let in_flight = objs
        .iter()
        .rev()
        .find(|obj| !is_skippable(obj))
        .map(|obj| {
            !(obj.get("type").and_then(Value::as_str) == Some("assistant")
                && !assistant_text(obj).is_empty())
        })
        .unwrap_or(true);

    ClaudeTurn {
        text,
        boundary_is_compact,
        in_flight,
    }
}

fn flag(obj: &Value, key: &str) -> bool {
    obj.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn is_compact_summary(obj: &Value) -> bool {
    obj.get("type").and_then(Value::as_str) == Some("user") && flag(obj, "isCompactSummary")
}

fn is_skippable(obj: &Value) -> bool {
    flag(obj, "isMeta") || flag(obj, "isSidechain") || flag(obj, "isCompactSummary")
}

fn is_turn_boundary(obj: &Value) -> bool {
    is_compact_summary(obj) || user_prompt_text(obj).is_some()
}

/// Rust twin of Python `_user_prompt_text`: Some(text) for a real user turn,
/// None for tool results, meta noise, and compact summaries.
fn user_prompt_text(obj: &Value) -> Option<String> {
    if obj.get("type").and_then(Value::as_str) != Some("user")
        || flag(obj, "isMeta")
        || flag(obj, "isCompactSummary")
    {
        return None;
    }
    let content = obj.pointer("/message/content")?;
    if let Some(text) = content.as_str() {
        return if text.trim().is_empty() {
            None
        } else {
            Some(text.to_string())
        };
    }
    let items = content.as_array()?;
    let mut texts = Vec::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("tool_result") => return None,
            Some("text") => {
                if let Some(t) = item.get("text").and_then(Value::as_str) {
                    texts.push(t);
                }
            }
            _ => {}
        }
    }
    let joined = texts.join("\n");
    if joined.trim().is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn assistant_text(obj: &Value) -> String {
    let Some(content) = obj.pointer("/message/content").and_then(Value::as_array) else {
        return String::new();
    };
    content
        .iter()
        .filter_map(|item| {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                item.get("text").and_then(Value::as_str)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPLETED: &str = r#"{"type":"user","message":{"content":"Fix the bug"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Fixed the null check. Which test suite should I run?"}]}}"#;

    const RECAP_AFTER_COMPACT: &str = r#"{"type":"user","message":{"content":"Fix the bug"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Working on it."}]}}
{"type":"user","isCompactSummary":true,"message":{"content":"This session is being continued from a previous conversation. Summary: fixing the bug. Would you like me to continue?"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Here is a recap of where we are. Would you like me to proceed?"}]}}"#;

    const IN_FLIGHT: &str = r#"{"type":"user","message":{"content":"Fix the bug"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Looking now."}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#;

    #[test]
    fn completed_turn_extracts_final_text() {
        let turn = read_claude_turn(COMPLETED);
        assert_eq!(
            turn.text,
            "Fixed the null check. Which test suite should I run?"
        );
        assert!(!turn.boundary_is_compact);
        assert!(!turn.in_flight);
    }

    #[test]
    fn recap_after_compact_flags_compact_boundary() {
        let turn = read_claude_turn(RECAP_AFTER_COMPACT);
        assert!(turn.boundary_is_compact);
        assert_eq!(
            turn.text,
            "Here is a recap of where we are. Would you like me to proceed?"
        );
    }

    #[test]
    fn tool_activity_at_tail_is_in_flight() {
        let turn = read_claude_turn(IN_FLIGHT);
        assert!(turn.in_flight);
    }

    #[test]
    fn sidechain_and_meta_entries_are_ignored() {
        let text = r#"{"type":"user","message":{"content":"Go"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Real answer."}]}}
{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"Subagent chatter?"}]}}
{"type":"user","isMeta":true,"message":{"content":"meta note?"}}"#;
        let turn = read_claude_turn(text);
        assert_eq!(turn.text, "Real answer.");
        assert!(!turn.in_flight);
    }

    #[test]
    fn empty_transcript_is_in_flight_with_no_text() {
        let turn = read_claude_turn("");
        assert_eq!(turn.text, "");
        assert!(turn.in_flight);
    }
}
```

- [ ] **Step 2: Register the module**

In `native/rust/src/lib.rs`, extend the module block from Task 3:

```rust
pub mod rules;
pub mod transcript;
```

- [ ] **Step 3: Run the Rust tests**

Run: `cargo test --manifest-path native/rust/Cargo.toml`
Expected: PASS (5 new `transcript::tests`, everything else unchanged).

- [ ] **Step 4: Commit**

```bash
git add native/rust/src/transcript.rs native/rust/src/lib.rs
git commit -m "native: compaction-aware Claude transcript turn reader"
```

---

### Task 5: Triage + classification — `classify_event`, event kinds, compact drop

The heart of the fix. `Notification` events stop being blind-trusted `needs_input`: they get a kind (`permission` / `idle` / `unknown`), and idle events are dropped when the transcript shows a compact boundary or an in-flight turn. `Stop` events whose boundary is a compact summary are dropped. `NotifyEnvelope` gains `kind`.

**Files:**
- Modify: `native/rust/src/lib.rs` — `NotifyEnvelope` (line ~229), `build_envelope` (line ~604), new `classify_event`; existing unit tests in the `tests` module
- Test: unit tests in `lib.rs` `tests` module

**Interfaces:**
- Consumes: `SeverityRules::classify` (Task 3), `transcript::read_claude_turn` (Task 4).
- Produces:
  ```rust
  pub struct Classification { pub severity: String, pub kind: String, pub excerpt: String }
  fn classify_event(event: &AgentEvent, rules: &SeverityRules) -> Classification
  pub fn build_envelope_with(event: AgentEvent, rules: &SeverityRules) -> NotifyEnvelope
  // build_envelope(event) delegates to build_envelope_with(event, SeverityRules::compiled_default())
  ```
  `NotifyEnvelope` gains `#[serde(default = "default_kind")] pub kind: String` (`default_kind()` returns `"turn"` so old spooled envelopes deserialize). Task 6 consumes `Classification` in `build_payload`; Task 7 consumes `kind` for channels/cooldown.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `native/rust/src/lib.rs` (uses `tempfile::tempdir`, already a dev-dependency). The transcript path must contain `.claude` so the Claude branch of transcript reading triggers:

```rust
    fn write_claude_transcript(dir: &std::path::Path, name: &str, jsonl: &str) -> String {
        let claude_dir = dir.join(".claude/projects/example");
        fs::create_dir_all(&claude_dir).unwrap();
        let path = claude_dir.join(name);
        fs::write(&path, jsonl).unwrap();
        path.to_string_lossy().into_owned()
    }

    const RECAP_TRANSCRIPT: &str = r#"{"type":"user","message":{"content":"Fix the bug"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Working on it."}]}}
{"type":"user","isCompactSummary":true,"message":{"content":"This session is being continued from a previous conversation. Summary: fixing the bug."}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Recap: fixed the null check. Would you like me to continue?"}]}}"#;

    const QUESTION_TRANSCRIPT: &str = r#"{"type":"user","message":{"content":"Add auth"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Added the scaffolding. Which auth approach do you want — JWT or sessions?"}]}}"#;

    fn stop_event(transcript_path: &str) -> AgentEvent {
        let raw = format!(
            r#"{{"type":"Stop","hook_event_name":"Stop","transcript_path":{},"cwd":"/tmp/project","thread-id":"t1","turn-id":"u1"}}"#,
            serde_json::to_string(transcript_path).unwrap()
        );
        let mut event = parse_hook_payload("Claude Code", &raw).unwrap();
        event.mux = MuxContext::default();
        event
    }

    fn notification_event(message: &str, transcript_path: &str) -> AgentEvent {
        let raw = format!(
            r#"{{"hook_event_name":"Notification","session_id":"s1","cwd":"/tmp/project","message":{},"transcript_path":{}}}"#,
            serde_json::to_string(message).unwrap(),
            serde_json::to_string(transcript_path).unwrap()
        );
        let mut event = parse_hook_payload("Claude Code", &raw).unwrap();
        event.mux = MuxContext::default();
        event
    }

    #[test]
    fn stop_at_compact_boundary_is_dropped() {
        // The recap text is question-shaped — exactly the old false positive —
        // but the compact boundary must drop it wholesale.
        let temp = tempdir().unwrap();
        let path = write_claude_transcript(temp.path(), "recap.jsonl", RECAP_TRANSCRIPT);
        let envelope = build_envelope(stop_event(&path));
        assert_eq!(envelope.severity, "normal");
        assert_eq!(envelope.kind, "turn");
        assert!(!is_actionable(&envelope));
    }

    #[test]
    fn stop_with_closing_question_is_needs_input_turn() {
        let temp = tempdir().unwrap();
        let path = write_claude_transcript(temp.path(), "question.jsonl", QUESTION_TRANSCRIPT);
        let envelope = build_envelope(stop_event(&path));
        assert_eq!(envelope.severity, "needs_input");
        assert_eq!(envelope.kind, "turn");
        assert!(is_actionable(&envelope));
    }

    #[test]
    fn permission_notification_is_permission_kind() {
        let event = notification_event("Claude needs your permission to use Bash", "");
        let envelope = build_envelope(event);
        assert_eq!(envelope.severity, "needs_input");
        assert_eq!(envelope.kind, "permission");
        assert!(is_actionable(&envelope));
    }

    #[test]
    fn idle_notification_after_compact_is_dropped() {
        // The false "Claude wants input right after a recap": idle Notification
        // while the transcript's last boundary is a compact summary.
        let temp = tempdir().unwrap();
        let path = write_claude_transcript(temp.path(), "recap2.jsonl", RECAP_TRANSCRIPT);
        let event = notification_event("Claude is waiting for your input", &path);
        let envelope = build_envelope(event);
        assert_eq!(envelope.severity, "normal");
        assert_eq!(envelope.kind, "idle");
        assert!(!is_actionable(&envelope));
    }

    #[test]
    fn idle_notification_mid_turn_is_dropped() {
        let temp = tempdir().unwrap();
        let in_flight = r#"{"type":"user","message":{"content":"Fix"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#;
        let path = write_claude_transcript(temp.path(), "inflight.jsonl", in_flight);
        let event = notification_event("Claude is waiting for your input", &path);
        let envelope = build_envelope(event);
        assert_eq!(envelope.severity, "normal");
        assert!(!is_actionable(&envelope));
    }

    #[test]
    fn idle_notification_without_transcript_survives_as_idle() {
        // Fail-soft: cannot inspect the transcript → still show a banner.
        let event = notification_event("Claude is waiting for your input", "");
        let envelope = build_envelope(event);
        assert_eq!(envelope.severity, "needs_input");
        assert_eq!(envelope.kind, "idle");
    }

    #[test]
    fn unknown_notification_survives_as_unknown_kind() {
        let event = notification_event("Some new hook message", "");
        let envelope = build_envelope(event);
        assert_eq!(envelope.severity, "needs_input");
        assert_eq!(envelope.kind, "unknown");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path native/rust/Cargo.toml`
Expected: FAIL — `envelope.kind` does not exist (compile error).

- [ ] **Step 3: Implement**

In `native/rust/src/lib.rs`:

(a) `NotifyEnvelope` (line ~229) gains a field after `severity`:

```rust
    #[serde(default = "default_kind")]
    pub kind: String,
```

and near it:

```rust
fn default_kind() -> String {
    "turn".to_string()
}
```

(b) Add the classification types and function (place above `build_envelope`):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub severity: String,
    pub kind: String,
    /// Closing excerpt of the assistant text, for the notification body.
    pub excerpt: String,
}

/// Triage for `Notification` hook events. Claude Code sends this hook for
/// permission prompts AND idle "waiting for your input" reminders — only the
/// former is unconditionally urgent. Idle reminders are dropped when the
/// transcript shows the turn is not actually waiting on the user: a compact
/// summary boundary (recap in progress) or an in-flight tool-using turn.
fn classify_notification(event: &AgentEvent, rules: &SeverityRules) -> Classification {
    let message = event
        .raw
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let lowered = message.to_lowercase();
    if lowered.contains("permission") {
        return Classification {
            severity: "needs_input".to_string(),
            kind: "permission".to_string(),
            excerpt: String::new(),
        };
    }
    if lowered.contains("waiting for your input") {
        if event.transcript_path.contains(".claude") {
            if let Ok(text) = fs::read_to_string(&event.transcript_path) {
                let turn = transcript::read_claude_turn(&text);
                if turn.boundary_is_compact || turn.in_flight {
                    return Classification {
                        severity: "normal".to_string(),
                        kind: "idle".to_string(),
                        excerpt: String::new(),
                    };
                }
            }
        }
        return Classification {
            severity: "needs_input".to_string(),
            kind: "idle".to_string(),
            excerpt: String::new(),
        };
    }
    let _ = rules;
    Classification {
        severity: "needs_input".to_string(),
        kind: "unknown".to_string(),
        excerpt: String::new(),
    }
}

fn classify_event(event: &AgentEvent, rules: &SeverityRules) -> Classification {
    if event.event_type == "Notification" {
        return classify_notification(event, rules);
    }
    // Turn completion. Claude Code: read the transcript compaction-aware;
    // Codex: the final text arrives inline on the payload.
    let inline = event
        .raw
        .get("last-assistant-message")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let text = if !inline.is_empty() {
        inline.to_string()
    } else if event.transcript_path.contains(".claude") {
        match fs::read_to_string(&event.transcript_path) {
            Ok(content) => {
                let turn = transcript::read_claude_turn(&content);
                if turn.boundary_is_compact {
                    // A Stop whose turn starts at a compact summary is the
                    // recap false-positive; never notify on it.
                    return Classification {
                        severity: "normal".to_string(),
                        kind: "turn".to_string(),
                        excerpt: String::new(),
                    };
                }
                turn.text
            }
            Err(_) => String::new(),
        }
    } else {
        extract_last_response(&event.transcript_path)
    };
    let severity = rules.classify(&text);
    let excerpt = if severity == "normal" {
        String::new()
    } else {
        closing_excerpt(&text, 140)
    };
    Classification {
        severity,
        kind: "turn".to_string(),
        excerpt,
    }
}

/// The last non-empty line of the message, trimmed to `max_chars` — the
/// closing ask/error a banner can actually show.
fn closing_excerpt(text: &str, max_chars: usize) -> String {
    let last_line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    let cleaned = clean_text(last_line);
    if cleaned.chars().count() <= max_chars {
        return cleaned;
    }
    let mut out = cleaned.chars().take(max_chars - 1).collect::<String>();
    out.push('…');
    out
}
```

(c) Replace `build_envelope` (line ~604) with:

```rust
pub fn build_envelope(event: AgentEvent) -> NotifyEnvelope {
    build_envelope_with(event, SeverityRules::compiled_default())
}

pub fn build_envelope_with(event: AgentEvent, rules: &SeverityRules) -> NotifyEnvelope {
    let key = event.dedup_key();
    let class = classify_event(&event, rules);
    let payload = build_payload(&event, &key);
    let channels = channels_for_severity(&class.severity);
    let group_key = notification_group_key(&event, &payload);
    let notification_id = notification_request_id(&group_key, &key, &class.severity);
    NotifyEnvelope {
        event,
        payload,
        severity: class.severity,
        kind: class.kind,
        channels,
        group_key,
        notification_id,
        created_at: now_secs(),
    }
}
```

(`build_payload`, `channels_for_severity`, and `notification_request_id` are reworked in Tasks 6–7; here they keep their current signatures so this task compiles and tests severity/kind only. `classify_severity` from Task 3 is now unused — delete the `fn classify_severity` wrapper and `last_assistant_message` if the compiler reports them dead; `is_suppressed_turn` still uses `last_assistant_message`, so it stays.)

In `handle_notify_cli` (line ~443), replace `let envelope = build_envelope(event);` with:

```rust
    let rules = SeverityRules::load();
    let envelope = build_envelope_with(event, &rules);
```

- [ ] **Step 4: Run the Rust tests**

Run: `cargo test --manifest-path native/rust/Cargo.toml`
Expected: ALL PASS — the 7 new tests, existing lib tests, and `replay_real_events`. (The real idle fixture `claude-notification-zellij` has no readable transcript, so it classifies `needs_input` exactly as before, and `build_payload` is untouched until Task 6, so its rendered bytes are unchanged here.)

- [ ] **Step 5: Commit**

```bash
git add native/rust/src/lib.rs
git commit -m "native: triage Notification events and drop compact-boundary turns"
```

---

### Task 6: Message content + replace-not-stack cards + fixture corpus

Bodies carry the extracted ask; titles are severity/kind-true; every card replaces per (pane, kind, severity) instead of stacking; the replay harness learns to carry transcripts so the recap scenarios are pinned with real payload shapes.

**Files:**
- Modify: `native/rust/src/lib.rs` — `build_payload` (line ~626), `notification_request_id` (line ~714), `build_envelope_with` (Task 5's version), unit tests
- Modify: `native/rust/tests/replay_real_events.rs` — `transcript_jsonl` support
- Modify: `native/rust/tests/fixtures/claude-notification-zellij.json`, `native/rust/tests/fixtures/claude-notification-permission.json` (expect blocks)
- Create: `native/rust/tests/fixtures/claude-stop-recap-compact.json`, `native/rust/tests/fixtures/claude-stop-question.json`, `native/rust/tests/fixtures/claude-notification-idle-compact.json`

**Interfaces:**
- Consumes: `Classification` (Task 5), and Task 5's test helpers in the lib.rs `tests` module: `write_claude_transcript(dir, name, jsonl) -> String`, `stop_event(transcript_path) -> AgentEvent`, `notification_event(message, transcript_path) -> AgentEvent`, `QUESTION_TRANSCRIPT`.
- Produces: `build_payload(event: &AgentEvent, key: &str, class: &Classification) -> NotifyPayload`; `notification_request_id(group_key: &str, kind: &str, severity: &str) -> String`. Message templates (exact strings):

| kind/severity | title | body | subtitle (zellij / tmux / cwd) |
|---|---|---|---|
| turn/normal | `{client} responded` | UNCHANGED old templates | UNCHANGED |
| turn/needs_input | `{client} needs input` | excerpt, else `Needs your input. Click to focus.` | `{session} / {tab} · {pane_ref}` / `{session} / {window} · {pane_id}` / `{cwd_name}` |
| turn/error | `{client} hit an error` | excerpt, else `Something failed. Click to focus.` | same as needs_input |
| permission | `{client} needs permission` | hook `message` verbatim, else `Permission needed. Click to focus.` | same |
| idle | `{client} is waiting` | `Still waiting on you — click to focus.` | same |
| unknown | `{client} needs input` | hook `message` verbatim, else `Waiting for your input. Click to focus.` | same |

- [ ] **Step 1: Rewrite the two existing stacking tests as replacement tests**

In the lib.rs `tests` module, replace `same_tmux_pane_needs_input_notifications_stack_distinct_cards` with:

```rust
    #[test]
    fn same_tmux_pane_needs_input_notifications_replace_one_card() {
        let event = AgentEvent {
            client: "Claude Code".to_string(),
            thread_id: "thread".to_string(),
            turn_id: "turn-1".to_string(),
            cwd: "/tmp/project".to_string(),
            mux: MuxContext {
                mux_type: "tmux".to_string(),
                session: "default".to_string(),
                window_id: "@7".to_string(),
                window_name: "opencode".to_string(),
                pane_id: "%1039".to_string(),
                ..MuxContext::default()
            },
            event_type: "Notification".to_string(),
            transcript_path: String::new(),
            raw: json!({"message": "Claude needs your permission to use Bash"}),
        };
        let mut next_event = event.clone();
        next_event.turn_id = "turn-2".to_string();

        let first = build_envelope(event);
        let second = build_envelope(next_event);

        assert_eq!(first.severity, "needs_input");
        assert_eq!(first.kind, "permission");
        assert_eq!(first.group_key, second.group_key);
        // Repeated prompts from the same pane replace one card, never stack.
        assert_eq!(first.notification_id, second.notification_id);
    }
```

and replace `codex_inline_error_message_keeps_distinct_notification_id` with:

```rust
    #[test]
    fn codex_inline_error_message_replaces_per_pane_card() {
        let event = AgentEvent {
            client: "Codex".to_string(),
            thread_id: "thread".to_string(),
            turn_id: "turn-1".to_string(),
            cwd: "/tmp/project".to_string(),
            mux: MuxContext {
                mux_type: "tmux".to_string(),
                session: "default".to_string(),
                window_id: "@7".to_string(),
                pane_id: "%1039".to_string(),
                ..MuxContext::default()
            },
            event_type: "agent-turn-complete".to_string(),
            transcript_path: String::new(),
            raw: json!({"last-assistant-message": "The command failed"}),
        };
        let mut next_event = event.clone();
        next_event.turn_id = "turn-2".to_string();

        let first = build_envelope(event);
        let second = build_envelope(next_event);

        assert_eq!(first.severity, "error");
        assert_eq!(first.group_key, second.group_key);
        assert_eq!(first.notification_id, second.notification_id);
    }
```

Add new template tests:

```rust
    #[test]
    fn needs_input_body_carries_the_closing_ask() {
        let temp = tempdir().unwrap();
        let path = write_claude_transcript(temp.path(), "ask.jsonl", QUESTION_TRANSCRIPT);
        let mut event = stop_event(&path);
        event.mux = MuxContext {
            mux_type: "zellij".to_string(),
            session: "default".to_string(),
            pane_ref: "terminal_19".to_string(),
            pane_title: "agent-signals".to_string(),
            tab_name: "signals".to_string(),
            ..MuxContext::default()
        };
        let envelope = build_envelope(event);
        assert_eq!(envelope.payload.title, "Claude Code needs input");
        assert_eq!(
            envelope.payload.message,
            "Added the scaffolding. Which auth approach do you want — JWT or sessions?"
        );
        assert_eq!(envelope.payload.subtitle, "default / signals · terminal_19");
    }

    #[test]
    fn error_turn_title_and_fallback_body() {
        let raw = r#"{
          "type": "agent-turn-complete",
          "thread-id": "t1",
          "turn-id": "u1",
          "cwd": "/tmp/project",
          "last-assistant-message": "The build failed: linker error, could not resolve symbol."
        }"#;
        let mut event = parse_hook_payload("Codex", raw).unwrap();
        event.mux = MuxContext::default();
        let envelope = build_envelope(event);
        assert_eq!(envelope.payload.title, "Codex hit an error");
        assert_eq!(
            envelope.payload.message,
            "The build failed: linker error, could not resolve symbol."
        );
        assert_eq!(envelope.payload.subtitle, "project");
    }

    #[test]
    fn idle_notification_renders_waiting_card() {
        let event = notification_event("Claude is waiting for your input", "");
        let envelope = build_envelope(event);
        assert_eq!(envelope.payload.title, "Claude Code is waiting");
        assert_eq!(
            envelope.payload.message,
            "Still waiting on you — click to focus."
        );
    }

    #[test]
    fn long_excerpt_is_trimmed_with_ellipsis() {
        let long_ask = format!("{} — which one?", "option ".repeat(40));
        let raw = format!(
            r#"{{"type":"agent-turn-complete","thread-id":"t","turn-id":"u","cwd":"/tmp/p","last-assistant-message":{}}}"#,
            serde_json::to_string(&long_ask).unwrap()
        );
        let mut event = parse_hook_payload("Codex", &raw).unwrap();
        event.mux = MuxContext::default();
        let envelope = build_envelope(event);
        assert_eq!(envelope.payload.message.chars().count(), 140);
        assert!(envelope.payload.message.ends_with('…'));
    }
```

Also update the existing `notification_event_uses_hook_message` test: its message is a permission prompt, so change the title assertion from `.contains("needs input")` to:

```rust
        assert_eq!(envelope.payload.title, "Claude Code needs permission");
```

(keep the message-verbatim assertion).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path native/rust/Cargo.toml`
Expected: FAIL — new tests don't compile against the old `build_payload` signature or fail on old template strings/ids.

- [ ] **Step 3: Implement templates and ids**

In `native/rust/src/lib.rs`:

(a) Replace `notification_request_id` (line ~714) with:

```rust
/// One card per (pane, kind, severity): a newer event of the same kind from
/// the same pane replaces the delivered card instead of stacking a new one.
fn notification_request_id(group_key: &str, kind: &str, severity: &str) -> String {
    sha256_hex(&format!("{group_key}:{kind}:{severity}"))
        .chars()
        .take(32)
        .collect()
}
```

(b) Replace `build_payload` (line ~626) with:

```rust
fn build_payload(event: &AgentEvent, key: &str, class: &Classification) -> NotifyPayload {
    let mux = event.mux.clone();

    // Location strings shared by every template.
    let (location, pane_label, ready_message) = match mux.mux_type.as_str() {
        "zellij" => {
            let session = if mux.session.is_empty() {
                "zellij"
            } else {
                &mux.session
            };
            let tab = if !mux.tab_name.is_empty() {
                mux.tab_name.clone()
            } else if !mux.tab_position.is_empty() {
                format!("tab {}", mux.tab_position)
            } else {
                "tab".to_string()
            };
            let pane_title = first_nonempty(&[&mux.pane_title, &event.cwd_name(), &mux.pane_ref]);
            (
                format!("{session} / {tab}"),
                mux.pane_ref.clone(),
                format!("{pane_title} ({}) is ready. Click to focus.", mux.pane_ref),
            )
        }
        "tmux" => {
            let session = if mux.session.is_empty() {
                "tmux"
            } else {
                &mux.session
            };
            let window = first_nonempty(&[&mux.window_name, &mux.window_index, "window"]);
            let pane_title = first_nonempty(&[&mux.pane_title, &event.cwd_name(), &mux.pane_id]);
            (
                format!("{session} / {window}"),
                mux.pane_id.clone(),
                format!("{pane_title} ({}) is ready. Click to focus.", mux.pane_id),
            )
        }
        _ if !event.cwd_name().is_empty() => (
            String::new(),
            String::new(),
            format!("{}: response is ready.", event.cwd_name()),
        ),
        _ => (String::new(), String::new(), "Prompt response is ready.".to_string()),
    };

    // Routine completions keep the legacy render exactly (they are dropped by
    // is_actionable anyway; --dry-run and the replay corpus pin these bytes).
    if class.kind == "turn" && class.severity == "normal" {
        return NotifyPayload {
            title: format!("{} responded", event.client),
            subtitle: location,
            message: ready_message,
            key: key.to_string(),
            mux,
        };
    }

    let subtitle = if !location.is_empty() && !pane_label.is_empty() {
        format!("{location} · {pane_label}")
    } else if !location.is_empty() {
        location
    } else {
        event.cwd_name()
    };

    let hook_message = event
        .raw
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    let (title, message) = match class.kind.as_str() {
        "permission" => (
            format!("{} needs permission", event.client),
            if hook_message.is_empty() {
                "Permission needed. Click to focus.".to_string()
            } else {
                hook_message
            },
        ),
        "idle" => (
            format!("{} is waiting", event.client),
            "Still waiting on you — click to focus.".to_string(),
        ),
        "unknown" => (
            format!("{} needs input", event.client),
            if hook_message.is_empty() {
                "Waiting for your input. Click to focus.".to_string()
            } else {
                hook_message
            },
        ),
        // "turn" with needs_input / error severity.
        _ => {
            let title = if class.severity == "error" {
                format!("{} hit an error", event.client)
            } else {
                format!("{} needs input", event.client)
            };
            let message = if !class.excerpt.is_empty() {
                class.excerpt.clone()
            } else if class.severity == "error" {
                "Something failed. Click to focus.".to_string()
            } else {
                "Needs your input. Click to focus.".to_string()
            };
            (title, message)
        }
    };

    NotifyPayload {
        title,
        subtitle,
        message,
        key: key.to_string(),
        mux,
    }
}
```

(c) In `build_envelope_with`, update the two call sites:

```rust
    let payload = build_payload(&event, &key, &class);
    ...
    let notification_id = notification_request_id(&group_key, &class.kind, &class.severity);
```

(remove the now-unused `event_key` parameter usage; `key` is still used by `build_payload` and dedup).

- [ ] **Step 4: Extend the replay harness with transcripts**

In `native/rust/tests/replay_real_events.rs`:

(a) Add to `struct Vector`:

```rust
    #[serde(default)]
    transcript_jsonl: Option<Vec<Value>>,
```

(b) At the top of the per-vector loop in `replays_every_real_event_fixture`, before `let raw = ...`, insert:

```rust
        // A fixture may carry its transcript inline; write it under a `.claude`
        // path so the Claude transcript branch triggers, and point the payload
        // at it.
        let _transcript_dir; // keep tempdir alive for the vector's lifetime
        let mut v = v;
        if let Some(lines) = v.transcript_jsonl.take() {
            let dir = tempfile::tempdir().unwrap();
            let claude_dir = dir.path().join(".claude/projects/replay");
            fs::create_dir_all(&claude_dir).unwrap();
            let path = claude_dir.join("transcript.jsonl");
            let text = lines
                .iter()
                .map(|l| serde_json::to_string(l).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(&path, text).unwrap();
            v.raw["transcript_path"] = Value::String(path.to_string_lossy().into_owned());
            _transcript_dir = Some(dir);
        } else {
            _transcript_dir = None;
        }
```

Note: fixtures with `transcript_jsonl` must set `expect.cwd` etc. normally but omit `transcript_path` from `raw` (the harness injects it), and the harness-injected path is not asserted. Add `tempfile` to `[dev-dependencies]` usage in this test via `use` — it is already a dev-dependency in `Cargo.toml`.

(c) Update the corpus floor: `vectors.len() >= 8` → `vectors.len() >= 11`.

- [ ] **Step 5: Add the three new fixtures**

Create `native/rust/tests/fixtures/claude-stop-recap-compact.json`:

```json
{
  "name": "claude-stop-recap-compact",
  "source": "synthesized from the recap false-positive: Stop firing on a post-compaction recap turn",
  "note": "The recap text is question-shaped ('Would you like me to continue?') — the old classifier turned this into needs_input. The compact boundary must drop it.",
  "label": "Claude Code",
  "raw": {
    "type": "Stop",
    "hook_event_name": "Stop",
    "cwd": "/Users/nathan/Developer/proj/agent-signals",
    "thread-id": "thread_recap",
    "turn-id": "turn_recap"
  },
  "transcript_jsonl": [
    {"type": "user", "message": {"content": "Fix the parser bug"}},
    {"type": "assistant", "message": {"content": [{"type": "text", "text": "Working on it."}]}},
    {"type": "user", "isCompactSummary": true, "message": {"content": "This session is being continued from a previous conversation that ran out of context. Summary: fixing the parser bug."}},
    {"type": "assistant", "message": {"content": [{"type": "text", "text": "Recap: the parser bug is fixed and tests pass. Would you like me to continue?"}]}}
  ],
  "inject_mux": null,
  "expect": {
    "parsed": true,
    "client": "Claude Code",
    "event_type": "Stop",
    "thread_id": "thread_recap",
    "turn_id": "turn_recap",
    "cwd": "/Users/nathan/Developer/proj/agent-signals",
    "severity": "normal",
    "title": "Claude Code responded",
    "subtitle": "",
    "message": "agent-signals: response is ready.",
    "group_key": "agent-signals:Claude Code:cwd:/Users/nathan/Developer/proj/agent-signals"
  }
}
```

Create `native/rust/tests/fixtures/claude-stop-question.json`:

```json
{
  "name": "claude-stop-question",
  "source": "synthesized: Stop on a genuine closing question",
  "note": "The one turn-completion shape that must still notify, with the ask in the body.",
  "label": "Claude Code",
  "raw": {
    "type": "Stop",
    "hook_event_name": "Stop",
    "cwd": "/Users/nathan/Developer/proj/agent-signals",
    "thread-id": "thread_q",
    "turn-id": "turn_q"
  },
  "transcript_jsonl": [
    {"type": "user", "message": {"content": "Add auth"}},
    {"type": "assistant", "message": {"content": [{"type": "text", "text": "Added the scaffolding. Which auth approach do you want — JWT or sessions?"}]}}
  ],
  "inject_mux": null,
  "expect": {
    "parsed": true,
    "client": "Claude Code",
    "event_type": "Stop",
    "thread_id": "thread_q",
    "turn_id": "turn_q",
    "cwd": "/Users/nathan/Developer/proj/agent-signals",
    "severity": "needs_input",
    "title": "Claude Code needs input",
    "subtitle": "agent-signals",
    "message": "Added the scaffolding. Which auth approach do you want — JWT or sessions?",
    "group_key": "agent-signals:Claude Code:cwd:/Users/nathan/Developer/proj/agent-signals"
  }
}
```

Create `native/rust/tests/fixtures/claude-notification-idle-compact.json`:

```json
{
  "name": "claude-notification-idle-compact",
  "source": "synthesized from the reported false positive: idle Notification during a compaction/recap pause",
  "note": "'Claude is waiting for your input' while the transcript's last boundary is a compact summary → dropped wholesale.",
  "label": "Claude Code",
  "raw": {
    "hook_event_name": "Notification",
    "session_id": "sess_idle_compact",
    "cwd": "/Users/nathan/Developer/proj/agent-signals",
    "message": "Claude is waiting for your input"
  },
  "transcript_jsonl": [
    {"type": "user", "message": {"content": "Fix the parser bug"}},
    {"type": "assistant", "message": {"content": [{"type": "text", "text": "Working on it."}]}},
    {"type": "user", "isCompactSummary": true, "message": {"content": "This session is being continued from a previous conversation that ran out of context. Summary: fixing the parser bug."}},
    {"type": "assistant", "message": {"content": [{"type": "text", "text": "Recap: the parser bug is fixed and tests pass. Would you like me to continue?"}]}}
  ],
  "inject_mux": null,
  "expect": {
    "parsed": true,
    "client": "Claude Code",
    "event_type": "Notification",
    "thread_id": "sess_idle_compact",
    "turn_id": "",
    "cwd": "/Users/nathan/Developer/proj/agent-signals",
    "severity": "normal",
    "title": "Claude Code is waiting",
    "subtitle": "agent-signals",
    "message": "Still waiting on you — click to focus.",
    "group_key": "agent-signals:Claude Code:cwd:/Users/nathan/Developer/proj/agent-signals"
  }
}
```

- [ ] **Step 6: Update the two existing Notification fixtures' expect blocks**

`native/rust/tests/fixtures/claude-notification-zellij.json` — a real idle event with no transcript (fail-soft banner). Update the `expect` fields:

```json
    "severity": "needs_input",
    "title": "Claude Code is waiting",
    "subtitle": "default / sba-agentic · terminal_19",
    "message": "Still waiting on you — click to focus.",
```

(keep every other field as-is).

`native/rust/tests/fixtures/claude-notification-permission.json` — read the fixture; its hook `message` contains "permission", so update `expect.title` to `"Claude Code needs permission"` and append <code> · {pane_ref-or-pane_id}</code> to `expect.subtitle` per its `inject_mux` (zellij → `pane_ref`, tmux → `pane_id`, no mux → subtitle becomes the cwd basename). `expect.message` stays the verbatim hook message. If the actual rendered values differ, the test failure output prints them — align the expect block with the template table above, never by weakening the assertions.

- [ ] **Step 7: Run the Rust tests**

Run: `cargo test --manifest-path native/rust/Cargo.toml`
Expected: ALL PASS, including `replay_real_events` with 11+ fixtures. If a fixture mismatch appears, fix the fixture's expect block only when the rendered output matches the template table; otherwise fix the code.

- [ ] **Step 8: Commit**

```bash
git add native/rust/src/lib.rs native/rust/tests/replay_real_events.rs native/rust/tests/fixtures/
git commit -m "native: informative severity-true notification bodies; cards replace per pane"
```

---

### Task 7: `app_config.rs` — user config, per-kind channels, daemon cooldown

**Files:**
- Create: `native/rust/src/app_config.rs`
- Modify: `native/rust/src/lib.rs` — `Paths` (line ~28), `channels_for_severity` (line ~872) → `channels_for`, `process_notify` (line ~1392), `handle_connection` Notify arm (line ~1259), `drain_spool` (line ~1556), `sweep_expired` (line ~1614)
- Test: unit tests in `app_config.rs` and lib.rs

**Interfaces:**
- Consumes: `ChannelConfig` (lib.rs:210), `Classification.kind` (Task 5).
- Produces:
  ```rust
  pub struct AppConfig {
      pub focus_gate: bool,          // default true (Task 8 consumes)
      pub cooldown_seconds: u64,     // default 45
      pub channels: HashMap<String, ChannelConfig>, // keys: turn_needs_input, turn_error, permission, idle, unknown
  }
  impl AppConfig {
      pub fn compiled_default() -> &'static AppConfig;
      pub fn load() -> AppConfig;    // ~/.config/agent-signals/config.json, defaults on missing/invalid
      pub fn channels_for(&self, kind: &str, severity: &str) -> ChannelConfig;
  }
  ```
  Defaults: `turn_needs_input` = bell+sound+banner; `turn_error` = bell+sound+banner+ignore_dnd; `permission`/`idle`/`unknown` = banner only. `process_notify` gains a `cooldown_seconds: u64` parameter and returns silenced channels (bell=false, sound=false) when the same (group_key, kind) posted within the window.

- [ ] **Step 1: Write `native/rust/src/app_config.rs`**

```rust
use crate::ChannelConfig;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

const BANNER_ONLY: ChannelConfig = ChannelConfig {
    bell: false,
    sound: false,
    notify: true,
    notify_ignore_dnd: false,
};
const FULL: ChannelConfig = ChannelConfig {
    bell: true,
    sound: true,
    notify: true,
    notify_ignore_dnd: false,
};
const FULL_DND: ChannelConfig = ChannelConfig {
    bell: true,
    sound: true,
    notify: true,
    notify_ignore_dnd: true,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub focus_gate: bool,
    pub cooldown_seconds: u64,
    pub channels: HashMap<String, ChannelConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut channels = HashMap::new();
        channels.insert("turn_needs_input".to_string(), FULL);
        channels.insert("turn_error".to_string(), FULL_DND);
        channels.insert("permission".to_string(), BANNER_ONLY);
        channels.insert("idle".to_string(), BANNER_ONLY);
        channels.insert("unknown".to_string(), BANNER_ONLY);
        Self {
            focus_gate: true,
            cooldown_seconds: 45,
            channels,
        }
    }
}

impl AppConfig {
    pub fn compiled_default() -> &'static AppConfig {
        static CONFIG: OnceLock<AppConfig> = OnceLock::new();
        CONFIG.get_or_init(AppConfig::default)
    }

    /// ~/.config/agent-signals/config.json; every field optional, unknown
    /// fields ignored, invalid file → defaults.
    pub fn load() -> AppConfig {
        let path = user_config_path();
        let Ok(text) = fs::read_to_string(&path) else {
            return AppConfig::default();
        };
        match serde_json::from_str::<AppConfig>(&text) {
            Ok(mut cfg) => {
                // Missing channel keys fall back to defaults per key.
                let defaults = AppConfig::default().channels;
                for (key, value) in defaults {
                    cfg.channels.entry(key).or_insert(value);
                }
                cfg
            }
            Err(_) => AppConfig::default(),
        }
    }

    pub fn channels_for(&self, kind: &str, severity: &str) -> ChannelConfig {
        let key = match kind {
            "turn" | "" => {
                if severity == "error" {
                    "turn_error"
                } else {
                    "turn_needs_input"
                }
            }
            other => other,
        };
        self.channels
            .get(key)
            .copied()
            .unwrap_or_else(|| AppConfig::default().channels[key])
    }
}

fn user_config_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".config/agent-signals/config.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_approved_cadence() {
        let cfg = AppConfig::default();
        let permission = cfg.channels_for("permission", "needs_input");
        assert!(permission.notify && !permission.bell && !permission.sound);
        let idle = cfg.channels_for("idle", "needs_input");
        assert!(idle.notify && !idle.bell && !idle.sound);
        let turn = cfg.channels_for("turn", "needs_input");
        assert!(turn.notify && turn.bell && turn.sound && !turn.notify_ignore_dnd);
        let error = cfg.channels_for("turn", "error");
        assert!(error.notify_ignore_dnd);
        assert_eq!(cfg.cooldown_seconds, 45);
        assert!(cfg.focus_gate);
    }

    #[test]
    fn partial_config_json_overrides_only_named_fields() {
        let cfg: AppConfig =
            serde_json::from_str(r#"{"cooldown_seconds": 120}"#).unwrap();
        assert_eq!(cfg.cooldown_seconds, 120);
        assert!(cfg.focus_gate);
    }

    #[test]
    fn legacy_empty_kind_maps_to_turn() {
        let cfg = AppConfig::default();
        assert_eq!(
            cfg.channels_for("", "needs_input"),
            cfg.channels_for("turn", "needs_input")
        );
    }
}
```

Note: `#[serde(default)]` on the struct requires `Default`; the manual `Default` impl above provides it, and `partial_config_json_overrides_only_named_fields` relies on it (channels map deserializes empty → filled by `load`'s merge; the direct `from_str` in the test leaves `channels` at the `#[serde(default)]` struct-level default, which is the full map — both paths safe). `ChannelConfig` already derives `Deserialize` + `Copy`.

- [ ] **Step 2: Wire config into lib.rs**

(a) Register: `pub mod app_config;` beside the other mods, and `use app_config::AppConfig;`.

(b) Delete `fn channels_for_severity` (line ~872). In `build_envelope_with`, replace `let channels = channels_for_severity(&class.severity);` with:

```rust
    let channels = AppConfig::compiled_default().channels_for(&class.kind, &class.severity);
```

(`build_envelope` and tests thus use deterministic defaults; the CLI path applies the user's config next.)

(c) In `handle_notify_cli`, after `let rules = SeverityRules::load();` add `let cfg = AppConfig::load();` and build the envelope with user channels:

```rust
    let rules = SeverityRules::load();
    let cfg = AppConfig::load();
    let mut envelope = build_envelope_with(event, &rules);
    envelope.channels = cfg.channels_for(&envelope.kind, &envelope.severity);
```

(d) `Paths` gains a cooldown dir. Add field `cooldown_dir: PathBuf` to `struct Paths` (line ~28), set it in `from_state_dir`:

```rust
            cooldown_dir: state_dir.join("cooldown"),
```

and create it in `ensure()`:

```rust
        fs::create_dir_all(&self.cooldown_dir)?;
```

In `sweep_expired`, after the dedup lines (line ~1618-1619), add:

```rust
    dedup_removed += remove_old_files(&paths.cooldown_dir, cutoff, dry_run)?;
```

(e) Cooldown. Add near `is_duplicate` (line ~1458):

```rust
/// Same pane + same kind inside the window → still post (the card replaces),
/// but silence bell and sound so a chatty pane cannot re-ring every turn.
fn in_cooldown(paths: &Paths, group_key: &str, kind: &str, window: u64) -> io::Result<bool> {
    if window == 0 {
        return Ok(false);
    }
    fs::create_dir_all(&paths.cooldown_dir)?;
    let file = paths
        .cooldown_dir
        .join(sha256_hex(&format!("{group_key}:{kind}")));
    let now = now_secs();
    let mut hit = false;
    if let Ok(text) = fs::read_to_string(&file) {
        if let Ok(last) = text.trim().parse::<u64>() {
            if now.saturating_sub(last) < window {
                hit = true;
            }
        }
    }
    fs::write(file, now.to_string())?;
    Ok(hit)
}
```

(f) `process_notify` gains the window parameter: signature becomes

```rust
fn process_notify(
    paths: &Paths,
    notifier: &Arc<Mutex<Option<UnixStream>>>,
    envelope: &NotifyEnvelope,
    cooldown_seconds: u64,
) -> io::Result<NotifyOutcome> {
```

After the `is_duplicate` early-return block, add:

```rust
    let silent = in_cooldown(paths, &envelope.group_key, &envelope.kind, cooldown_seconds)?;
    let delivered_channels = if silent {
        ChannelConfig {
            bell: false,
            sound: false,
            ..envelope.channels
        }
    } else {
        envelope.channels
    };
```

and replace every `channels: Some(envelope.channels)` in the posted/spooled outcomes below that point with `channels: Some(delivered_channels)` (the early `notify`-disabled and notifier-unavailable outcomes above the dedup check keep `envelope.channels`).

Update the two callers: `handle_connection`'s Notify arm →

```rust
            let cooldown = AppConfig::load().cooldown_seconds;
            let outcome = process_notify(&paths, &notifier, &envelope, cooldown)?;
```

and `drain_spool` →

```rust
        let outcome = process_notify(paths, notifier, &envelope, 0)?;
```

(spool drain replays queued cards; the cooldown was already stamped when they were accepted, and re-silencing a drain would drop cues the user never got).

- [ ] **Step 3: Write the failing daemon cooldown test**

Add to lib.rs `tests`:

```rust
    #[test]
    fn cooldown_silences_repeat_posts_from_same_pane() {
        let temp = tempdir().unwrap();
        let paths = Paths::from_state_dir(temp.path().to_path_buf());
        paths.ensure().unwrap();

        assert!(!in_cooldown(&paths, "group-a", "turn", 45).unwrap());
        assert!(in_cooldown(&paths, "group-a", "turn", 45).unwrap());
        // Different kind or group is independent.
        assert!(!in_cooldown(&paths, "group-a", "permission", 45).unwrap());
        assert!(!in_cooldown(&paths, "group-b", "turn", 45).unwrap());
        // Window 0 disables.
        assert!(!in_cooldown(&paths, "group-a", "turn", 0).unwrap());
    }
```

- [ ] **Step 4: Run the Rust tests**

Run: `cargo test --manifest-path native/rust/Cargo.toml`
Expected: ALL PASS. The pre-existing test `routine_notify_cli_does_not_contact_daemon_or_spool` and replay corpus stay green (channels for `normal` never reach the daemon).

- [ ] **Step 5: Commit**

```bash
git add native/rust/src/app_config.rs native/rust/src/lib.rs
git commit -m "native: user config with per-kind channels and daemon-side cooldown"
```

---

### Task 8: Focus gate — suppress when Nathan is looking at the pane

**Files:**
- Create: `native/rust/src/focus_gate.rs`
- Modify: `native/rust/src/lib.rs` — `handle_notify_cli` (line ~430)
- Test: unit tests in `focus_gate.rs`

**Interfaces:**
- Consumes: `MuxContext` (lib.rs:103), `run_output_with_timeout` (lib.rs:1019 — make it `pub(crate)`), `AppConfig.focus_gate` (Task 7).
- Produces: `pub fn source_is_focused(mux: &MuxContext) -> bool` (true = suppress) and the pure decision core `pub fn decide(frontmost_app: Option<&str>, pane_active: Option<bool>) -> bool`.

- [ ] **Step 1: Write `native/rust/src/focus_gate.rs`**

```rust
use crate::{run_output_with_timeout, MuxContext};
use serde_json::Value;
use std::process::Command;
use std::time::Duration;

/// True only when we can positively confirm the user is looking at the source
/// pane: Ghostty is the frontmost app AND the event's pane is the focused pane
/// of its multiplexer session. Any probe failure returns false — fail open,
/// the notification fires.
pub fn source_is_focused(mux: &MuxContext) -> bool {
    let pane_active = match mux.mux_type.as_str() {
        "tmux" => tmux_pane_active(&mux.pane_id),
        "zellij" => zellij_pane_active(mux),
        _ => None,
    };
    if pane_active != Some(true) {
        return false; // cheap check first; also fail-open on unknown
    }
    decide(frontmost_app_name().as_deref(), pane_active)
}

pub fn decide(frontmost_app: Option<&str>, pane_active: Option<bool>) -> bool {
    matches!(frontmost_app, Some(name) if name.contains("Ghostty")) && pane_active == Some(true)
}

fn frontmost_app_name() -> Option<String> {
    // lsappinfo avoids the System Events TCC prompt osascript would trigger.
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "lsappinfo info -only name \"$(lsappinfo front)\""]);
    let output = run_output_with_timeout(command, Duration::from_secs(2)).ok()?;
    // Output shape: "LSDisplayName"="Ghostty" (key name varies by macOS).
    let value = output.rsplit('=').next()?.trim().trim_matches('"');
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn tmux_pane_active(pane_id: &str) -> Option<bool> {
    if pane_id.is_empty() {
        return None;
    }
    let mut command = Command::new("tmux");
    command.args([
        "display-message",
        "-p",
        "-t",
        pane_id,
        "#{session_attached}\t#{window_active}\t#{pane_active}",
    ]);
    let output = run_output_with_timeout(command, Duration::from_secs(2)).ok()?;
    let parts: Vec<&str> = output.trim().split('\t').collect();
    if parts.len() != 3 {
        return None;
    }
    let attached = parts[0].parse::<u32>().ok()? > 0;
    Some(attached && parts[1] == "1" && parts[2] == "1")
}

fn zellij_pane_active(mux: &MuxContext) -> Option<bool> {
    if mux.session.is_empty() || mux.pane_ref.is_empty() {
        return None;
    }
    let mut command = Command::new("zellij");
    command.args([
        "--session",
        &mux.session,
        "action",
        "list-panes",
        "--json",
        "--all",
        "--state",
        "--tab",
    ]);
    let output = run_output_with_timeout(command, Duration::from_secs(2)).ok()?;
    let panes: Vec<Value> = serde_json::from_str(&output).ok()?;
    let bare_id = mux
        .pane_ref
        .trim_start_matches("terminal_")
        .trim_start_matches("plugin_");
    let pane = panes.iter().find(|p| {
        p.get("id")
            .map(|id| {
                id.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| id.to_string())
            })
            .unwrap_or_default()
            == bare_id
    })?;
    pane_focus_field(pane)
}

/// zellij's list-panes JSON has carried different focus field names across
/// versions; accept the known spellings, and return None (fail-open) when the
/// installed version exposes none of them.
fn pane_focus_field(pane: &Value) -> Option<bool> {
    for key in ["is_focused", "focused", "is_active", "active"] {
        if let Some(v) = pane.get(key).and_then(Value::as_bool) {
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn suppresses_only_when_ghostty_front_and_pane_active() {
        assert!(decide(Some("Ghostty"), Some(true)));
        assert!(!decide(Some("Safari"), Some(true)));
        assert!(!decide(Some("Ghostty"), Some(false)));
        assert!(!decide(Some("Ghostty"), None));
        assert!(!decide(None, Some(true)));
    }

    #[test]
    fn zellij_focus_field_tolerates_version_spellings() {
        assert_eq!(pane_focus_field(&json!({"is_focused": true})), Some(true));
        assert_eq!(pane_focus_field(&json!({"focused": false})), Some(false));
        assert_eq!(pane_focus_field(&json!({"title": "x"})), None);
    }

    #[test]
    fn no_mux_never_suppresses() {
        assert!(!source_is_focused(&MuxContext::default()));
    }
}
```

- [ ] **Step 2: Wire into lib.rs**

(a) `pub mod focus_gate;` beside the other mods; change `fn run_output_with_timeout` (line ~1019) to `pub(crate) fn run_output_with_timeout`.

(b) In `handle_notify_cli`, after the `if !actionable { return Ok(0); }` block, add:

```rust
    // Focus gate: if Nathan is already looking at the source pane, no channel
    // fires at all. Fail-open — any undeterminable probe notifies as before.
    if cfg.focus_gate && focus_gate::source_is_focused(&envelope.event.mux) {
        return Ok(0);
    }
```

(This sits after the `dry_run` block, so `--dry-run` keeps reporting pure classification.)

- [ ] **Step 3: Run the Rust tests**

Run: `cargo test --manifest-path native/rust/Cargo.toml`
Expected: ALL PASS (`no_mux_never_suppresses` exercises the fail-open path without probing; the tmux/zellij/lsappinfo probes are exercised live in Task 9).

- [ ] **Step 4: Commit**

```bash
git add native/rust/src/focus_gate.rs native/rust/src/lib.rs
git commit -m "native: suppress notifications when the source pane is focused"
```

---

### Task 9: Docs, deploy, live verification

**Files:**
- Modify: `README.md` (the notification-behavior section around "Routine turn-complete events are intentionally silent"), `HANDOFF.md`
- No code changes.

- [ ] **Step 1: Update README**

In `README.md`, find the section describing notification behavior (search for "Routine turn-complete"). Rewrite it to document, in this order: (1) the four event kinds (`turn`, `permission`, `idle`, `unknown`) and when each notifies; (2) compaction-awareness — Stops and idle reminders during a compact/recap never notify; (3) default channels per kind (turn needs_input/error = banner+bell+sound, error additionally pierces DND; permission/idle/unknown = banner only, cards replace per pane); (4) the 45s per-pane cooldown and the focus gate; (5) the two user config files with a complete example of each:

````markdown
### Tuning

Severity patterns live in `config/severity-rules.json` (compiled into the
binary; user override at `~/.config/agent-signals/severity-rules.json`).
Cadence lives in `~/.config/agent-signals/config.json` (all fields optional):

```json
{
  "focus_gate": true,
  "cooldown_seconds": 45,
  "channels": {
    "turn_needs_input": {"bell": true, "sound": true, "notify": true, "notify_ignore_dnd": false},
    "turn_error":       {"bell": true, "sound": true, "notify": true, "notify_ignore_dnd": true},
    "permission":       {"bell": false, "sound": false, "notify": true, "notify_ignore_dnd": false},
    "idle":             {"bell": false, "sound": false, "notify": true, "notify_ignore_dnd": false},
    "unknown":          {"bell": false, "sound": false, "notify": true, "notify_ignore_dnd": false}
  }
}
```
````

- [ ] **Step 2: Run both full test suites one last time**

Run: `cargo test --manifest-path native/rust/Cargo.toml && .venv/bin/pytest tests/ -q`
Expected: ALL PASS.

- [ ] **Step 3: Deploy**

Run: `scripts/install-native.sh`
Expected: cargo release build, binaries + wrappers installed, LaunchAgents kick-started, `doctor` output ends with notifier connected + authorization ok.

- [ ] **Step 4: Live verification through use (scripted replay against the installed binary)**

Replay the false-positive and true-positive scenarios through the *installed* binary. `--dry-run` prints the envelope verdict without posting:

```bash
cd /Users/nathan/Developer/proj/agent-signals
mkdir -p /tmp/agsig-e2e/.claude/projects/e2e
cat > /tmp/agsig-e2e/.claude/projects/e2e/recap.jsonl <<'EOF'
{"type":"user","message":{"content":"Fix the parser bug"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Working on it."}]}}
{"type":"user","isCompactSummary":true,"message":{"content":"This session is being continued from a previous conversation. Summary: fixing the parser bug."}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Recap: fixed. Would you like me to continue?"}]}}
EOF
cat > /tmp/agsig-e2e/.claude/projects/e2e/question.jsonl <<'EOF'
{"type":"user","message":{"content":"Add auth"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Which auth approach do you want — JWT or sessions?"}]}}
EOF

# 1. The reported false positive: idle Notification during recap → would_notify false
echo '{"hook_event_name":"Notification","session_id":"e2e1","cwd":"/tmp/p","message":"Claude is waiting for your input","transcript_path":"/tmp/agsig-e2e/.claude/projects/e2e/recap.jsonl"}' \
  | ~/.local/bin/agent-signal notify --client "Claude Code" --dry-run | python3 -c "import json,sys; d=json.load(sys.stdin); assert d['would_notify'] is False and d['kind']=='idle', d; print('PASS idle-during-recap dropped')"

# 2. Stop on the recap turn → would_notify false
echo '{"type":"Stop","hook_event_name":"Stop","cwd":"/tmp/p","thread-id":"t","turn-id":"u","transcript_path":"/tmp/agsig-e2e/.claude/projects/e2e/recap.jsonl"}' \
  | ~/.local/bin/agent-signal notify --client "Claude Code" --dry-run | python3 -c "import json,sys; d=json.load(sys.stdin); assert d['would_notify'] is False, d; print('PASS recap Stop dropped')"

# 3. Genuine closing question → would_notify true, body carries the ask
echo '{"type":"Stop","hook_event_name":"Stop","cwd":"/tmp/p","thread-id":"t","turn-id":"u2","transcript_path":"/tmp/agsig-e2e/.claude/projects/e2e/question.jsonl"}' \
  | ~/.local/bin/agent-signal notify --client "Claude Code" --dry-run | python3 -c "import json,sys; d=json.load(sys.stdin); assert d['would_notify'] is True and 'JWT or sessions' in d['payload']['message'], d; print('PASS question notifies with ask in body')"

# 4. One real banner end-to-end (live daemon): permission prompt → banner only
echo '{"hook_event_name":"Notification","session_id":"e2e2","cwd":"/tmp/p","message":"Claude needs your permission to use Bash"}' \
  | ~/.local/bin/agent-signal notify --client "Claude Code"
echo "CHECK: a 'Claude Code needs permission' banner appeared, with NO Glass sound"
```

Expected: three PASS lines, then one visible banner titled "Claude Code needs permission" with no sound. If the banner check can't be observed in this session, state that explicitly and hand Nathan the one command to re-run.

- [ ] **Step 5: Update HANDOFF.md**

Append a dated section to `HANDOFF.md` summarizing: what shipped (triage, compaction-awareness, unified rules, informative bodies, replace-not-stack, cooldown, focus gate, config files), that it is deployed via install-native.sh, and the two config paths.

- [ ] **Step 6: Commit and push the branch**

```bash
git add README.md HANDOFF.md
git commit -m "docs: notification triage, cadence, and tuning config"
git push -u origin notification-rework
```

Then open a PR titled "Notification rework: triage, compaction-awareness, informative cards" with a body summarizing the spec's Problem/Design sections. No Claude attribution anywhere.

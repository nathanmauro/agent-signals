//! Replay real notification events captured from the live daemon state on
//! `in8-mac` (`~/.local/state/agent-signals/`) back through the parse → envelope
//! pipeline, and assert the derived severity / group key / rendered payload.
//!
//! The fixtures in `tests/fixtures/` were generated from actual events:
//!   * Claude Code `Stop` / `Notification` hooks rendered into zellij and
//!     cwd-fallback notifications (the `expect` block is the byte-for-byte
//!     output the daemon really produced).
//!   * Verbatim Codex `agent-turn-complete` payloads (real key naming:
//!     `thread-id`, `turn-id`, `last-assistant-message`, `input-messages`).
//!
//! `parse_hook_payload` reads the *current* environment for multiplexer
//! context (`detect_mux`), which is non-deterministic in CI. So we assert the
//! environment-independent parsed fields straight off the parse, then inject
//! the fixture's captured mux (or an empty mux) before building the envelope —
//! making the rendered-payload assertions fully reproducible anywhere.

use std::fs;
use std::path::PathBuf;

use agent_signals_native::{build_envelope, parse_hook_payload, MuxContext};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct Vector {
    name: String,
    label: String,
    raw: Value,
    #[serde(default)]
    inject_mux: Option<MuxContext>,
    expect: Expect,
}

#[derive(Debug, Deserialize)]
struct Expect {
    parsed: bool,
    #[serde(default)]
    client: String,
    #[serde(default)]
    event_type: String,
    #[serde(default)]
    thread_id: String,
    #[serde(default)]
    turn_id: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    subtitle: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    group_key: String,
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn load_vectors() -> Vec<Vector> {
    let mut paths: Vec<_> = fs::read_dir(fixtures_dir())
        .expect("fixtures dir must exist")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let text = fs::read_to_string(&p).unwrap();
            serde_json::from_str::<Vector>(&text)
                .unwrap_or_else(|e| panic!("fixture {} failed to parse: {e}", p.display()))
        })
        .collect()
}

#[test]
fn replays_every_real_event_fixture() {
    let vectors = load_vectors();
    assert!(
        vectors.len() >= 8,
        "expected the real-event fixture corpus, found only {}",
        vectors.len()
    );

    for v in vectors {
        let raw = serde_json::to_string(&v.raw).unwrap();
        let parsed = parse_hook_payload(&v.label, &raw);

        if !v.expect.parsed {
            assert!(
                parsed.is_none(),
                "[{}] expected event to be ignored, but it parsed",
                v.name
            );
            continue;
        }

        let mut event = parsed.unwrap_or_else(|| panic!("[{}] expected event to parse", v.name));

        // Environment-independent fields straight off the parse.
        assert_eq!(event.client, v.expect.client, "[{}] client", v.name);
        assert_eq!(
            event.event_type, v.expect.event_type,
            "[{}] event_type",
            v.name
        );
        assert_eq!(event.thread_id, v.expect.thread_id, "[{}] thread_id", v.name);
        assert_eq!(event.turn_id, v.expect.turn_id, "[{}] turn_id", v.name);
        assert_eq!(event.cwd, v.expect.cwd, "[{}] cwd", v.name);

        // Pin the mux to the captured value so `detect_mux()` reading the test
        // host's real environment can't perturb the rendered payload.
        event.mux = v.inject_mux.clone().unwrap_or_default();

        let envelope = build_envelope(event);
        assert_eq!(envelope.severity, v.expect.severity, "[{}] severity", v.name);
        assert_eq!(
            envelope.group_key, v.expect.group_key,
            "[{}] group_key",
            v.name
        );
        assert_eq!(
            envelope.payload.title, v.expect.title,
            "[{}] title",
            v.name
        );
        assert_eq!(
            envelope.payload.subtitle, v.expect.subtitle,
            "[{}] subtitle",
            v.name
        );
        assert_eq!(
            envelope.payload.message, v.expect.message,
            "[{}] message",
            v.name
        );
    }
}

/// Regression: the live Codex hook invoked `agent-signal notify --client "<the
/// entire raw JSON payload>"`, so the whole payload landed in `event.client`
/// and contaminated `group_key` / `notification_id` — every Codex turn produced
/// a distinct, JSON-shaped group key instead of a stable per-pane one. This
/// pins that real degenerate behavior so a fix to the hook (or to client
/// labeling) trips this test and is updated deliberately. See HANDOFF.md.
#[test]
fn real_codex_client_blob_regression() {
    let text = fs::read_to_string(fixtures_dir().join("codex-turn-complete-desktop.json")).unwrap();
    let v: Vector = serde_json::from_str(&text).unwrap();
    // The bug: the payload itself was passed as the --client label.
    let payload = serde_json::to_string(&v.raw).unwrap();

    let mut event = parse_hook_payload(&payload, &payload).expect("payload still parses");
    assert_eq!(
        event.client, payload,
        "the entire payload ends up as the client label"
    );

    event.mux = MuxContext::default();
    let envelope = build_envelope(event);
    assert!(
        envelope.group_key.starts_with("agent-signals:{"),
        "group_key degenerates into JSON: {}",
        envelope.group_key
    );
    // ...and is NOT the stable per-cwd key the clean invocation would yield.
    assert_ne!(
        envelope.group_key,
        "agent-signals:Codex Desktop:cwd:/Users/nathan/.codex/worktrees/b7c6/cockpit"
    );
}

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

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

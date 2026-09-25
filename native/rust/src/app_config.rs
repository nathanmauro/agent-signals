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

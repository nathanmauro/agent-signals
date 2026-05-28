from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

DEFAULT_VOICE_CONFIG: dict[str, Any] = {
    "defaults": {
        "voice": "af_bella",
        "speed": "1.0",
        "lang": "en-us",
        "max_chars": 650,
    },
    "rules": [],
}

DEFAULT_STATE_DIR = Path.home() / ".local" / "state" / "agent-signals"
DEFAULT_VOICE_STATE_DIR = Path.home() / ".local" / "state" / "agent-voice"
DEFAULT_SOUND = Path("/System/Library/Sounds/Glass.aiff")
DEFAULT_KOKORO_ROOT = Path.home() / ".local" / "share" / "kokoro-tts"
DEFAULT_DEDUP_SECONDS = 20


@dataclass
class VoiceRule:
    name: str = "default"
    patterns: list[str] = field(default_factory=list)
    voice: str = "af_bella"
    speed: str = "1.0"
    lang: str = "en-us"
    max_chars: int = 650
    prefix: str = ""

    @classmethod
    def from_dict(cls, data: dict[str, Any], defaults: dict[str, Any]) -> VoiceRule:
        return cls(
            name=str(data.get("name", "matched")),
            patterns=data.get("patterns", []),
            voice=str(data.get("voice", defaults.get("voice", "af_bella"))),
            speed=str(data.get("speed", defaults.get("speed", "1.0"))),
            lang=str(data.get("lang", defaults.get("lang", "en-us"))),
            max_chars=int(data.get("max_chars", defaults.get("max_chars", 650))),
            prefix=str(data.get("prefix", "")),
        )


@dataclass
class VoiceConfig:
    defaults: dict[str, Any] = field(default_factory=lambda: dict(DEFAULT_VOICE_CONFIG["defaults"]))
    rules: list[VoiceRule] = field(default_factory=list)

    @classmethod
    def load(cls, path: Path | None = None) -> VoiceConfig:
        if not path or not path.exists():
            return cls()
        try:
            data = json.loads(path.read_text())
        except Exception:
            return cls()
        if not isinstance(data, dict):
            return cls()
        defaults = dict(DEFAULT_VOICE_CONFIG["defaults"])
        defaults.update(data.get("defaults", {}))
        raw_rules = data.get("rules", [])
        if not isinstance(raw_rules, list):
            raw_rules = []
        rules = [VoiceRule.from_dict(r, defaults) for r in raw_rules if isinstance(r, dict)]
        return cls(defaults=defaults, rules=rules)

    def default_rule(self) -> VoiceRule:
        return VoiceRule(
            voice=self.defaults.get("voice", "af_bella"),
            speed=self.defaults.get("speed", "1.0"),
            lang=self.defaults.get("lang", "en-us"),
            max_chars=int(self.defaults.get("max_chars", 650)),
        )


@dataclass
class ChannelConfig:
    bell: bool = True
    sound: bool = True
    notify: bool = True
    notify_ignore_dnd: bool = False


@dataclass
class PolicyConfig:
    normal: ChannelConfig = field(default_factory=lambda: ChannelConfig(
        bell=True, sound=True, notify=True, notify_ignore_dnd=False,
    ))
    needs_input: ChannelConfig = field(default_factory=lambda: ChannelConfig(
        bell=True, sound=True, notify=True, notify_ignore_dnd=False,
    ))
    error: ChannelConfig = field(default_factory=lambda: ChannelConfig(
        bell=True, sound=True, notify=True, notify_ignore_dnd=True,
    ))

    def for_severity(self, severity: str) -> ChannelConfig:
        return getattr(self, severity, self.normal)

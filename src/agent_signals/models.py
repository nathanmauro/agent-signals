from __future__ import annotations

import hashlib
import re
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any


def clean_text(value: object) -> str:
    text = str(value or "")
    text = re.sub(r"\x1b\[[0-9;?]*[ -/]*[@-~]", "", text)
    text = re.sub(r"\s+", " ", text).strip()
    text = re.sub(r"^[⠀-⣿]\s*", "", text).strip()
    return text


@dataclass
class MuxContext:
    type: str = ""
    session: str = ""
    pane_ref: str = ""
    pane_title: str = ""
    tab_name: str = ""
    tab_id: str = ""
    tab_position: str = ""
    cwd: str = ""
    command: str = ""
    session_id: str = ""
    window_index: str = ""
    window_id: str = ""
    window_name: str = ""
    pane_id: str = ""

    def to_dict(self) -> dict[str, str]:
        return {k: v for k, v in asdict(self).items() if v}

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> MuxContext:
        known = {f.name for f in cls.__dataclass_fields__.values()}
        return cls(**{k: str(v) for k, v in data.items() if k in known})


@dataclass
class AgentEvent:
    client: str
    event_type: str = ""
    thread_id: str = ""
    turn_id: str = ""
    cwd: str = ""
    transcript_path: str = ""
    mux: MuxContext = field(default_factory=MuxContext)
    raw: dict[str, Any] = field(default_factory=dict)

    @property
    def cwd_name(self) -> str:
        return Path(self.cwd).name if self.cwd else ""

    def dedup_key(self) -> str:
        seed = "|".join(
            p
            for p in [self.client, self.thread_id, self.turn_id]
            if p
        )
        if not seed:
            raw_str = str(self.raw)[:200]
            seed = "|".join(
                p
                for p in [self.client, self.event_type, self.cwd, raw_str]
                if p
            )
        return hashlib.sha256(seed.encode("utf-8", "ignore")).hexdigest()


@dataclass
class NotifyPayload:
    title: str = ""
    subtitle: str = ""
    message: str = ""
    key: str = ""
    mux: MuxContext = field(default_factory=MuxContext)

    def to_dict(self) -> dict[str, Any]:
        d = asdict(self)
        d["mux"] = self.mux.to_dict()
        return d

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> NotifyPayload:
        mux_data = data.get("mux") or {}
        return cls(
            title=data.get("title", ""),
            subtitle=data.get("subtitle", ""),
            message=data.get("message", ""),
            key=data.get("key", ""),
            mux=MuxContext.from_dict(mux_data),
        )


@dataclass
class SpeechPayload:
    key: str = ""
    text: str = ""
    voice: str = "af_bella"
    speed: str = "1.0"
    lang: str = "en-us"
    rule: str = "default"


@dataclass
class Severity:
    NORMAL = "normal"
    NEEDS_INPUT = "needs_input"
    ERROR = "error"


@dataclass
class ChannelPolicy:
    bell: bool = True
    sound: bool = True
    notify: bool = True
    notify_ignore_dnd: bool = False

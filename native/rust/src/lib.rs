use clap::{Parser, Subcommand};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wait_timeout::ChildExt;

const DEDUP_SECONDS: u64 = 20;
const RETENTION_SECONDS: u64 = 24 * 60 * 60;
const MAX_SPOOL_FILES: usize = 128;
const MAX_DEDUP_FILES: usize = 1024;
const MAX_ACTIVE_NOTIFICATIONS: usize = 32;
const SWEEP_INTERVAL_SECONDS: u64 = 6 * 60 * 60;
const DEFAULT_SOUND: &str = "/System/Library/Sounds/Glass.aiff";

#[derive(Debug, Clone)]
struct Paths {
    state_dir: PathBuf,
    socket: PathBuf,
    contexts_dir: PathBuf,
    dedup_dir: PathBuf,
    spool_dir: PathBuf,
    active_index: PathBuf,
}

impl Paths {
    fn new() -> Self {
        let state_dir = env::var_os("AGENT_SIGNALS_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".local/state/agent-signals"));
        Self::from_state_dir(state_dir)
    }

    fn from_state_dir(state_dir: PathBuf) -> Self {
        Self {
            socket: state_dir.join("agent-signald.sock"),
            contexts_dir: state_dir.join("contexts"),
            dedup_dir: state_dir.join("dedup"),
            spool_dir: state_dir.join("spool"),
            active_index: state_dir.join("active-notifications.json"),
            state_dir,
        }
    }

    fn ensure(&self) -> io::Result<()> {
        fs::create_dir_all(&self.state_dir)?;
        fs::create_dir_all(&self.contexts_dir)?;
        fs::create_dir_all(&self.dedup_dir)?;
        fs::create_dir_all(&self.spool_dir)?;
        Ok(())
    }
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn clean_text(value: impl AsRef<str>) -> String {
    let text = value.as_ref();
    let ansi = Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap();
    let whitespace = Regex::new(r"\s+").unwrap();
    let without_ansi = ansi.replace_all(text, "");
    let trimmed = whitespace.replace_all(without_ansi.trim(), " ");
    trimmed
        .trim_start_matches(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
        .trim()
        .to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MuxContext {
    #[serde(rename = "type", default, skip_serializing_if = "String::is_empty")]
    pub mux_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pane_ref: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pane_title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tab_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tab_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tab_position: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub window_index: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub window_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub window_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pane_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentEvent {
    pub client: String,
    #[serde(default)]
    pub event_type: String,
    #[serde(default)]
    pub thread_id: String,
    #[serde(default)]
    pub turn_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub mux: MuxContext,
    #[serde(default)]
    pub raw: Value,
}

impl AgentEvent {
    fn cwd_name(&self) -> String {
        if self.cwd.is_empty() {
            String::new()
        } else {
            Path::new(&self.cwd)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        }
    }

    fn dedup_key(&self) -> String {
        let seed = [
            self.client.as_str(),
            self.thread_id.as_str(),
            self.turn_id.as_str(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("|");
        if !seed.is_empty() {
            return sha256_hex(&seed);
        }
        let raw = self.raw.to_string();
        let raw_prefix = raw.chars().take(200).collect::<String>();
        let fallback = [
            self.client.as_str(),
            self.event_type.as_str(),
            self.cwd.as_str(),
            raw_prefix.as_str(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("|");
        sha256_hex(&fallback)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotifyPayload {
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    pub message: String,
    pub key: String,
    pub mux: MuxContext,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelConfig {
    pub bell: bool,
    pub sound: bool,
    pub notify: bool,
    pub notify_ignore_dnd: bool,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            bell: true,
            sound: true,
            notify: true,
            notify_ignore_dnd: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotifyEnvelope {
    pub event: AgentEvent,
    pub payload: NotifyPayload,
    pub severity: String,
    pub channels: ChannelConfig,
    pub group_key: String,
    pub notification_id: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireMessage {
    Notify {
        envelope: NotifyEnvelope,
    },
    Focus {
        context: String,
    },
    Health {
        verbose: bool,
    },
    Sweep {
        legacy: bool,
        dry_run: bool,
    },
    NotifierHello {
        pid: Option<u32>,
        authorization: String,
    },
    NotificationClicked {
        id: String,
    },
    PostNotification {
        id: String,
        group: String,
        title: String,
        subtitle: String,
        message: String,
        ignore_dnd: bool,
    },
    Response {
        status: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        channels: Option<ChannelConfig>,
        #[serde(skip_serializing_if = "Option::is_none")]
        notifier_connected: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        authorization: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveNotification {
    id: String,
    group_key: String,
    updated_at: u64,
}

#[derive(Debug, Clone, Default)]
struct NotifierStatus {
    connected: bool,
    authorization: String,
    pid: Option<u32>,
}

#[derive(Parser)]
#[command(name = "agent-signal", about = "AI agent signal infrastructure")]
struct SignalCli {
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: SignalCommand,
}

#[derive(Subcommand)]
enum SignalCommand {
    Notify {
        #[arg(long, default_value = "")]
        client: String,
        #[arg(long)]
        dry_run: bool,
        payload: Option<String>,
    },
    Focus {
        context: String,
    },
    SpeakLast {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Voice {
        action: Option<String>,
    },
    Doctor {
        #[arg(long)]
        verbose: bool,
    },
    Sweep {
        #[arg(long)]
        legacy: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Parser)]
#[command(name = "agent-signald", about = "Agent Signals notification daemon")]
struct SignaldCli {
    #[arg(long)]
    foreground: bool,
}

pub fn run_agent_signal() -> i32 {
    let cli = SignalCli::parse();
    let paths = Paths::new();
    match cli.command {
        SignalCommand::Notify {
            client,
            dry_run,
            payload,
        } => {
            let dry_run = cli.dry_run || dry_run;
            match handle_notify_cli(&paths, client, payload, dry_run) {
                Ok(code) => code,
                Err(err) => {
                    eprintln!("agent-signal notify: {err}");
                    1
                }
            }
        }
        SignalCommand::Focus { context } => match focus_context(&paths, &context) {
            Ok(true) => 0,
            Ok(false) => 1,
            Err(err) => {
                eprintln!("agent-signal focus: {err}");
                1
            }
        },
        SignalCommand::SpeakLast { args } => {
            run_python_subcommand("speak-last", &args, cli.dry_run)
        }
        SignalCommand::Voice { action } => {
            let args = action.into_iter().collect::<Vec<_>>();
            run_python_subcommand("voice", &args, cli.dry_run)
        }
        SignalCommand::Doctor { verbose } => {
            let verbose = verbose || cli.dry_run;
            handle_doctor(&paths, verbose);
            0
        }
        SignalCommand::Sweep { legacy, dry_run } => {
            let dry_run = cli.dry_run || dry_run;
            match handle_sweep_cli(&paths, legacy, dry_run) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!("agent-signal sweep: {err}");
                    1
                }
            }
        }
    }
}

pub fn run_agent_signald() -> i32 {
    let _cli = SignaldCli::parse();
    let paths = Paths::new();
    let daemon = match Daemon::new(paths) {
        Ok(daemon) => daemon,
        Err(err) => {
            eprintln!("agent-signald: {err}");
            return 1;
        }
    };
    match daemon.serve() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("agent-signald: {err}");
            1
        }
    }
}

fn handle_notify_cli(
    paths: &Paths,
    client: String,
    payload_arg: Option<String>,
    dry_run: bool,
) -> io::Result<i32> {
    let raw = read_payload_arg_or_stdin(payload_arg)?;
    let Some(event) = parse_hook_payload(&client, &raw) else {
        return Ok(0);
    };
    if is_suppressed_turn(&event) {
        return Ok(0);
    }
    let envelope = build_envelope(event);

    if dry_run {
        println!("{}", serde_json::to_string_pretty(&envelope).unwrap());
        return Ok(0);
    }

    let request = WireMessage::Notify {
        envelope: envelope.clone(),
    };
    match send_request(paths, &request, Duration::from_millis(900)) {
        Ok(WireMessage::Response {
            status, channels, ..
        }) => {
            if status == "posted" || status == "spooled" {
                if let Some(channels) = channels {
                    send_local_cues(channels);
                }
            }
            Ok(0)
        }
        Ok(_) => Ok(0),
        Err(_) => {
            paths.ensure()?;
            spool_envelope(paths, &envelope)?;
            send_local_cues(envelope.channels);
            Ok(0)
        }
    }
}

fn read_payload_arg_or_stdin(payload_arg: Option<String>) -> io::Result<String> {
    if let Some(payload) = payload_arg {
        if !payload.is_empty() {
            return Ok(payload);
        }
    }
    if io::stdin().is_terminal() {
        return Ok(String::new());
    }
    let mut raw = String::new();
    io::stdin().read_to_string(&mut raw)?;
    Ok(raw)
}

pub fn parse_hook_payload(label: &str, raw: &str) -> Option<AgentEvent> {
    let data: Value = if raw.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(raw).unwrap_or_else(|_| Value::Object(Default::default()))
    };
    let event_type = string_field(&data, &["type", "hook_event_name"]);
    if !event_type.is_empty()
        && event_type != "agent-turn-complete"
        && event_type != "Stop"
        && event_type != "Notification"
    {
        return None;
    }
    let transcript = string_field(
        &data,
        &[
            "transcript_path",
            "transcriptPath",
            "session_path",
            "sessionPath",
        ],
    );
    let client = if !label.is_empty() {
        label.to_string()
    } else if transcript.contains(".claude") {
        "Claude Code".to_string()
    } else {
        "Codex".to_string()
    };
    Some(AgentEvent {
        client,
        event_type,
        thread_id: string_field(&data, &["thread-id", "session_id"]),
        turn_id: string_field(&data, &["turn-id", "turn_id"]),
        cwd: string_field(&data, &["cwd"]),
        transcript_path: transcript,
        mux: detect_mux(),
        raw: data,
    })
}

fn string_field(data: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| data.get(*key))
        .and_then(|v| {
            v.as_str()
                .map(str::to_string)
                .or_else(|| Some(v.to_string()))
        })
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

pub fn build_envelope(event: AgentEvent) -> NotifyEnvelope {
    let key = event.dedup_key();
    let payload = build_payload(&event, &key);
    let severity = if event.event_type == "Notification" {
        "needs_input".to_string()
    } else {
        classify_severity(&extract_last_response(&event.transcript_path))
    };
    let channels = channels_for_severity(&severity);
    let group_key = notification_group_key(&event, &payload);
    let notification_id = sha256_hex(&group_key).chars().take(32).collect::<String>();
    NotifyEnvelope {
        event,
        payload,
        severity,
        channels,
        group_key,
        notification_id,
        created_at: now_secs(),
    }
}

fn build_payload(event: &AgentEvent, key: &str) -> NotifyPayload {
    let is_notification = event.event_type == "Notification";
    let title = if is_notification {
        format!("{} needs input", event.client)
    } else {
        format!("{} responded", event.client)
    };
    let mut subtitle = String::new();
    let mut message = "Prompt response is ready.".to_string();
    let mux = event.mux.clone();

    match mux.mux_type.as_str() {
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
            subtitle = format!("{session} / {tab}");
            message = format!("{pane_title} ({}) is ready. Click to focus.", mux.pane_ref);
        }
        "tmux" => {
            let session = if mux.session.is_empty() {
                "tmux"
            } else {
                &mux.session
            };
            let window = first_nonempty(&[&mux.window_name, &mux.window_index, "window"]);
            let pane_title = first_nonempty(&[&mux.pane_title, &event.cwd_name(), &mux.pane_id]);
            subtitle = format!("{session} / {window}");
            message = format!("{pane_title} ({}) is ready. Click to focus.", mux.pane_id);
        }
        _ if !event.cwd_name().is_empty() => {
            message = format!("{}: response is ready.", event.cwd_name());
        }
        _ => {}
    }

    if is_notification {
        let hook_message = event
            .raw
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        message = if hook_message.is_empty() {
            "Waiting for your input. Click to focus.".to_string()
        } else {
            hook_message.to_string()
        };
    }

    NotifyPayload {
        title,
        subtitle,
        message,
        key: key.to_string(),
        mux,
    }
}

fn first_nonempty(values: &[&str]) -> String {
    values
        .iter()
        .find(|v| !v.is_empty())
        .copied()
        .unwrap_or("")
        .to_string()
}

fn notification_group_key(event: &AgentEvent, payload: &NotifyPayload) -> String {
    let mux = &payload.mux;
    let target = match mux.mux_type.as_str() {
        "zellij" => format!("zellij:{}:{}", mux.session, mux.pane_ref),
        "tmux" => format!("tmux:{}:{}:{}", mux.session, mux.window_id, mux.pane_id),
        _ => format!("cwd:{}", event.cwd),
    };
    format!("agent-signals:{}:{target}", event.client)
}

fn extract_last_response(transcript_path: &str) -> String {
    let path = Path::new(transcript_path);
    if transcript_path.is_empty() || !path.exists() {
        return String::new();
    }
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    if transcript_path.contains(".claude") {
        extract_claude_text(&text)
    } else {
        extract_codex_text(&text)
    }
}

fn extract_claude_text(text: &str) -> String {
    for line in text.lines().rev() {
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if obj.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(content) = obj.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        let parts = content.iter().filter_map(|item| {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                item.get("text").and_then(Value::as_str)
            } else {
                None
            }
        });
        return parts.collect::<Vec<_>>().join("\n").trim().to_string();
    }
    String::new()
}

fn extract_codex_text(text: &str) -> String {
    for line in text.lines().rev() {
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if obj.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = obj.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) != Some("message")
            || payload.get("role").and_then(Value::as_str) != Some("assistant")
        {
            continue;
        }
        let Some(content) = payload.get("content").and_then(Value::as_array) else {
            continue;
        };
        let parts = content.iter().filter_map(|item| {
            let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
            if kind == "output_text" || kind == "text" {
                item.get("text").and_then(Value::as_str)
            } else {
                None
            }
        });
        return parts.collect::<Vec<_>>().join("\n").trim().to_string();
    }
    String::new()
}

/// The agent's final message for this turn. Codex delivers it inline on the
/// hook payload (`last-assistant-message`); Claude Code leaves it in the
/// transcript, which we read back.
fn last_assistant_message(event: &AgentEvent) -> String {
    if let Some(inline) = event
        .raw
        .get("last-assistant-message")
        .and_then(Value::as_str)
    {
        let trimmed = inline.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    extract_last_response(&event.transcript_path)
}

/// True when `text` is a standalone JSON object or array — i.e. the whole
/// message is machine output, not prose. A prose reply that merely mentions or
/// embeds a JSON snippet won't parse cleanly and is left alone.
fn looks_like_json(text: &str) -> bool {
    let trimmed = text.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return false;
    }
    matches!(
        serde_json::from_str::<Value>(trimmed),
        Ok(Value::Object(_)) | Ok(Value::Array(_))
    )
}

/// Suppress turn-completion notifications whose entire response is a JSON blob
/// (e.g. Codex Desktop's internal title-generation turns, whose
/// `last-assistant-message` is `{"title":"…"}`). `Notification` events —
/// permission prompts and "waiting for input" — always surface.
pub fn is_suppressed_turn(event: &AgentEvent) -> bool {
    if event.event_type == "Notification" {
        return false;
    }
    looks_like_json(&last_assistant_message(event))
}

fn classify_severity(text: &str) -> String {
    if text.is_empty() {
        return "normal".to_string();
    }
    let error = Regex::new(r"(?i)\b(blocked|failed|failure|error|traceback|exception|permission denied|unable to|could not|can't)\b").unwrap();
    if error.is_match(text) {
        return "error".to_string();
    }
    let needs_input = Regex::new(
        r"(?i)(\?\s*$|\b(please confirm|which one|choose|send me|tell me|do you want)\b)",
    )
    .unwrap();
    if needs_input.is_match(text) {
        return "needs_input".to_string();
    }
    "normal".to_string()
}

fn channels_for_severity(severity: &str) -> ChannelConfig {
    ChannelConfig {
        bell: true,
        sound: true,
        notify: true,
        notify_ignore_dnd: severity == "error",
    }
}

fn detect_mux() -> MuxContext {
    if env::var_os("ZELLIJ_PANE_ID").is_some() {
        let ctx = zellij_context();
        if !ctx.mux_type.is_empty() {
            return ctx;
        }
    }
    if env::var_os("TMUX_PANE").is_some() {
        let ctx = tmux_context();
        if !ctx.mux_type.is_empty() {
            return ctx;
        }
    }
    MuxContext::default()
}

fn zellij_context() -> MuxContext {
    let pane_id = env::var("ZELLIJ_PANE_ID").unwrap_or_default();
    let session = env::var("ZELLIJ_SESSION_NAME").unwrap_or_default();
    if pane_id.is_empty() {
        return MuxContext::default();
    }
    let pane_ref = if pane_id.starts_with("terminal_") || pane_id.starts_with("plugin_") {
        pane_id.clone()
    } else {
        format!("terminal_{pane_id}")
    };
    let mut command = Command::new("zellij");
    if !session.is_empty() {
        command.args(["--session", &session]);
    }
    command.args([
        "action",
        "list-panes",
        "--json",
        "--all",
        "--state",
        "--tab",
        "--command",
    ]);
    let panes = run_output_with_timeout(command, Duration::from_secs(2))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let bare_id = pane_id
        .trim_start_matches("terminal_")
        .trim_start_matches("plugin_");
    let found = panes.and_then(|v| v.as_array().cloned()).and_then(|items| {
        items.into_iter().find(|pane| {
            pane.get("id")
                .map(|id| {
                    id.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| id.to_string())
                })
                .unwrap_or_default()
                == bare_id
        })
    });

    MuxContext {
        mux_type: "zellij".to_string(),
        session,
        pane_ref,
        pane_title: found
            .as_ref()
            .and_then(|v| v.get("title"))
            .map(value_to_clean_string)
            .unwrap_or_default(),
        tab_name: found
            .as_ref()
            .and_then(|v| v.get("tab_name"))
            .map(value_to_clean_string)
            .unwrap_or_default(),
        tab_id: found
            .as_ref()
            .and_then(|v| v.get("tab_id"))
            .map(value_to_clean_string)
            .unwrap_or_default(),
        tab_position: found
            .as_ref()
            .and_then(|v| v.get("tab_position"))
            .map(value_to_clean_string)
            .unwrap_or_default(),
        cwd: found
            .as_ref()
            .and_then(|v| v.get("pane_cwd"))
            .map(value_to_clean_string)
            .unwrap_or_default(),
        command: found
            .as_ref()
            .and_then(|v| v.get("pane_command"))
            .map(value_to_clean_string)
            .unwrap_or_default(),
        ..MuxContext::default()
    }
}

fn tmux_context() -> MuxContext {
    let pane_id = env::var("TMUX_PANE").unwrap_or_default();
    if pane_id.is_empty() {
        return MuxContext::default();
    }
    let fmt = "#{session_name}\t#{session_id}\t#{window_index}\t#{window_id}\t#{window_name}\t#{pane_id}\t#{pane_title}";
    let mut command = Command::new("tmux");
    command.args(["display-message", "-p", "-t", &pane_id, fmt]);
    let Ok(output) = run_output_with_timeout(command, Duration::from_secs(2)) else {
        return MuxContext::default();
    };
    let parts = output.trim_end().split('\t').collect::<Vec<_>>();
    if parts.len() < 7 {
        return MuxContext::default();
    }
    MuxContext {
        mux_type: "tmux".to_string(),
        session: clean_text(parts[0]),
        session_id: clean_text(parts[1]),
        window_index: clean_text(parts[2]),
        window_id: clean_text(parts[3]),
        window_name: clean_text(parts[4]),
        pane_id: clean_text(parts[5]),
        pane_title: clean_text(parts[6]),
        ..MuxContext::default()
    }
}

fn value_to_clean_string(value: &Value) -> String {
    value
        .as_str()
        .map(clean_text)
        .unwrap_or_else(|| clean_text(value.to_string()))
}

fn run_output_with_timeout(mut command: Command, timeout: Duration) -> io::Result<String> {
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn()?;
    let status = match child.wait_timeout(timeout)? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "command timed out"));
        }
    };
    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "command failed"));
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        stdout.read_to_string(&mut out)?;
    }
    Ok(out)
}

fn send_local_cues(channels: ChannelConfig) {
    if channels.bell {
        let _ = send_bell();
    }
    if channels.sound {
        play_sound();
    }
}

fn send_bell() -> bool {
    let mut pid = std::process::id().to_string();
    while !pid.is_empty() && pid != "1" {
        let tty = Command::new("ps")
            .args(["-o", "tty=", "-p", &pid])
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if !tty.is_empty() && tty != "??" {
            if fs::OpenOptions::new()
                .write(true)
                .open(format!("/dev/{tty}"))
                .and_then(|mut f| f.write_all(b"\x07"))
                .is_ok()
            {
                return true;
            }
        }
        pid = Command::new("ps")
            .args(["-o", "ppid=", "-p", &pid])
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
    }
    false
}

fn play_sound() {
    if Path::new(DEFAULT_SOUND).exists() {
        let _ = Command::new("/usr/bin/afplay")
            .arg(DEFAULT_SOUND)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

fn send_request(
    paths: &Paths,
    request: &WireMessage,
    timeout: Duration,
) -> io::Result<WireMessage> {
    let mut stream = UnixStream::connect(&paths.socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    send_json_line(&mut stream, request)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str(line.trim()).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn send_json_line(stream: &mut UnixStream, message: &WireMessage) -> io::Result<()> {
    let mut data = serde_json::to_vec(message)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    data.push(b'\n');
    stream.write_all(&data)
}

struct Daemon {
    paths: Paths,
    notifier: Arc<Mutex<Option<UnixStream>>>,
    notifier_status: Arc<Mutex<NotifierStatus>>,
}

impl Daemon {
    fn new(paths: Paths) -> io::Result<Self> {
        paths.ensure()?;
        let report = sweep_expired(&paths, false)?;
        if report.total_removed > 0 {
            eprintln!(
                "agent-signald: removed {} expired state files",
                report.total_removed
            );
        }
        Ok(Self {
            paths,
            notifier: Arc::new(Mutex::new(None)),
            notifier_status: Arc::new(Mutex::new(NotifierStatus::default())),
        })
    }

    fn serve(self) -> io::Result<()> {
        if self.paths.socket.exists() {
            if UnixStream::connect(&self.paths.socket).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "daemon already listening at {}",
                        self.paths.socket.display()
                    ),
                ));
            }
            let _ = fs::remove_file(&self.paths.socket);
        }
        let listener = UnixListener::bind(&self.paths.socket)?;
        listener.set_nonblocking(true)?;
        let running = Arc::new(AtomicBool::new(true));
        let signal_flag = running.clone();
        ctrlc::set_handler(move || {
            signal_flag.store(false, Ordering::SeqCst);
        })
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

        let sweep_paths = self.paths.clone();
        let sweep_flag = running.clone();
        thread::spawn(move || {
            let interval = Duration::from_secs(SWEEP_INTERVAL_SECONDS);
            let tick = Duration::from_secs(1);
            let mut waited = Duration::ZERO;
            while sweep_flag.load(Ordering::SeqCst) {
                thread::sleep(tick);
                waited += tick;
                if waited < interval {
                    continue;
                }
                waited = Duration::ZERO;
                match sweep_expired(&sweep_paths, false) {
                    Ok(report) if report.total_removed > 0 => eprintln!(
                        "agent-signald: periodic sweep removed {} expired state files",
                        report.total_removed
                    ),
                    Ok(_) => {}
                    Err(err) => eprintln!("agent-signald: periodic sweep failed: {err}"),
                }
            }
        });

        while running.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let paths = self.paths.clone();
                    let notifier = self.notifier.clone();
                    let status = self.notifier_status.clone();
                    thread::spawn(move || {
                        if let Err(err) = handle_connection(paths, notifier, status, stream) {
                            eprintln!("agent-signald connection: {err}");
                        }
                    });
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(err) => return Err(err),
            }
        }
        let _ = fs::remove_file(&self.paths.socket);
        Ok(())
    }
}

fn handle_connection(
    paths: Paths,
    notifier: Arc<Mutex<Option<UnixStream>>>,
    notifier_status: Arc<Mutex<NotifierStatus>>,
    stream: UnixStream,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let message: WireMessage = serde_json::from_str(line.trim())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    match message {
        WireMessage::NotifierHello { pid, authorization } => {
            {
                let mut status = notifier_status.lock().unwrap();
                status.connected = true;
                status.authorization = authorization;
                status.pid = pid;
            }
            {
                let mut slot = notifier.lock().unwrap();
                *slot = Some(stream.try_clone()?);
            }
            let mut reply_stream = stream.try_clone()?;
            let _ = send_json_line(
                &mut reply_stream,
                &WireMessage::Response {
                    status: "ok".to_string(),
                    message: "notifier registered".to_string(),
                    channels: None,
                    notifier_connected: Some(true),
                    authorization: notifier_status.lock().unwrap().authorization.clone().into(),
                    detail: None,
                },
            );
            let _ = drain_spool(&paths, &notifier);
            loop {
                line.clear();
                if reader.read_line(&mut line)? == 0 {
                    break;
                }
                if let Ok(WireMessage::NotificationClicked { id }) =
                    serde_json::from_str::<WireMessage>(line.trim())
                {
                    let _ = focus_context(&paths, &id);
                }
            }
            *notifier.lock().unwrap() = None;
            notifier_status.lock().unwrap().connected = false;
        }
        WireMessage::Notify { envelope } => {
            let outcome = process_notify(&paths, &notifier, &envelope)?;
            let mut stream = stream;
            send_json_line(
                &mut stream,
                &WireMessage::Response {
                    status: outcome.status,
                    message: outcome.message,
                    channels: outcome.channels,
                    notifier_connected: Some(outcome.notifier_connected),
                    authorization: None,
                    detail: None,
                },
            )?;
        }
        WireMessage::Focus { context } => {
            let ok = focus_context(&paths, &context)?;
            let mut stream = stream;
            send_json_line(
                &mut stream,
                &WireMessage::Response {
                    status: if ok { "focused" } else { "not_found" }.to_string(),
                    message: String::new(),
                    channels: None,
                    notifier_connected: None,
                    authorization: None,
                    detail: None,
                },
            )?;
        }
        WireMessage::Health { verbose } => {
            let status = notifier_status.lock().unwrap().clone();
            let detail = if verbose {
                Some(json!({
                    "state_dir": paths.state_dir,
                    "socket": paths.socket,
                    "contexts_dir": paths.contexts_dir,
                    "spool_dir": paths.spool_dir,
                    "notifier_pid": status.pid,
                }))
            } else {
                None
            };
            let mut stream = stream;
            send_json_line(
                &mut stream,
                &WireMessage::Response {
                    status: "ok".to_string(),
                    message: "agent-signald is running".to_string(),
                    channels: None,
                    notifier_connected: Some(status.connected),
                    authorization: Some(status.authorization),
                    detail,
                },
            )?;
        }
        WireMessage::Sweep { legacy, dry_run } => {
            let report = sweep_expired(&paths, dry_run)?;
            let legacy_detail = if legacy {
                Some(sweep_legacy_terminal_notifier(dry_run)?)
            } else {
                None
            };
            let mut stream = stream;
            send_json_line(
                &mut stream,
                &WireMessage::Response {
                    status: "ok".to_string(),
                    message: "sweep complete".to_string(),
                    channels: None,
                    notifier_connected: None,
                    authorization: None,
                    detail: Some(json!({
                        "expired": report,
                        "legacy": legacy_detail,
                    })),
                },
            )?;
        }
        WireMessage::NotificationClicked { id } => {
            let _ = focus_context(&paths, &id);
        }
        WireMessage::PostNotification { .. } | WireMessage::Response { .. } => {}
    }
    Ok(())
}

struct NotifyOutcome {
    status: String,
    message: String,
    channels: Option<ChannelConfig>,
    notifier_connected: bool,
}

fn process_notify(
    paths: &Paths,
    notifier: &Arc<Mutex<Option<UnixStream>>>,
    envelope: &NotifyEnvelope,
) -> io::Result<NotifyOutcome> {
    if !envelope.channels.notify {
        return Ok(NotifyOutcome {
            status: "posted".to_string(),
            message: "notification channel disabled".to_string(),
            channels: Some(envelope.channels),
            notifier_connected: notifier.lock().unwrap().is_some(),
        });
    }

    if notifier.lock().unwrap().is_none() {
        spool_envelope(paths, envelope)?;
        return Ok(NotifyOutcome {
            status: "spooled".to_string(),
            message: "notifier helper unavailable".to_string(),
            channels: Some(envelope.channels),
            notifier_connected: false,
        });
    }

    if is_duplicate(paths, &envelope.payload.key)? {
        return Ok(NotifyOutcome {
            status: "duplicate".to_string(),
            message: "dedup window suppressed notification".to_string(),
            channels: None,
            notifier_connected: true,
        });
    }

    save_context(paths, envelope)?;
    update_active_index(paths, envelope)?;

    let post = WireMessage::PostNotification {
        id: envelope.notification_id.clone(),
        group: envelope.group_key.clone(),
        title: envelope.payload.title.clone(),
        subtitle: envelope.payload.subtitle.clone(),
        message: envelope.payload.message.clone(),
        ignore_dnd: envelope.channels.notify_ignore_dnd,
    };

    let mut guard = notifier.lock().unwrap();
    if let Some(stream) = guard.as_mut() {
        if send_json_line(stream, &post).is_ok() {
            return Ok(NotifyOutcome {
                status: "posted".to_string(),
                message: "notification posted".to_string(),
                channels: Some(envelope.channels),
                notifier_connected: true,
            });
        }
    }
    *guard = None;
    spool_envelope(paths, envelope)?;
    Ok(NotifyOutcome {
        status: "spooled".to_string(),
        message: "notifier write failed".to_string(),
        channels: Some(envelope.channels),
        notifier_connected: false,
    })
}

fn is_duplicate(paths: &Paths, key: &str) -> io::Result<bool> {
    fs::create_dir_all(&paths.dedup_dir)?;
    let file = paths.dedup_dir.join(key);
    let now = now_secs();
    if let Ok(text) = fs::read_to_string(&file) {
        if let Ok(last) = text.trim().parse::<u64>() {
            if now.saturating_sub(last) < DEDUP_SECONDS {
                return Ok(true);
            }
        }
    }
    fs::write(file, now.to_string())?;
    Ok(false)
}

fn save_context(paths: &Paths, envelope: &NotifyEnvelope) -> io::Result<()> {
    fs::create_dir_all(&paths.contexts_dir)?;
    let path = paths
        .contexts_dir
        .join(format!("{}.json", envelope.notification_id));
    let data = serde_json::to_vec_pretty(&envelope.payload)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(path, data)
}

fn update_active_index(paths: &Paths, envelope: &NotifyEnvelope) -> io::Result<()> {
    let mut index = load_active_index(paths);
    index.insert(
        envelope.notification_id.clone(),
        ActiveNotification {
            id: envelope.notification_id.clone(),
            group_key: envelope.group_key.clone(),
            updated_at: now_secs(),
        },
    );
    let cutoff = now_secs().saturating_sub(RETENTION_SECONDS);
    let mut removed_ids = Vec::new();
    index.retain(|id, item| {
        let keep = item.updated_at >= cutoff;
        if !keep {
            removed_ids.push(id.clone());
        }
        keep
    });
    if index.len() > MAX_ACTIVE_NOTIFICATIONS {
        let mut items = index.values().cloned().collect::<Vec<_>>();
        items.sort_by_key(|item| item.updated_at);
        for stale in items
            .into_iter()
            .take(index.len() - MAX_ACTIVE_NOTIFICATIONS)
        {
            index.remove(&stale.id);
            removed_ids.push(stale.id);
        }
    }
    for id in removed_ids {
        let _ = fs::remove_file(paths.contexts_dir.join(format!("{id}.json")));
    }
    let data = serde_json::to_vec_pretty(&index)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(&paths.active_index, data)
}

fn load_active_index(paths: &Paths) -> HashMap<String, ActiveNotification> {
    fs::read_to_string(&paths.active_index)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn spool_envelope(paths: &Paths, envelope: &NotifyEnvelope) -> io::Result<()> {
    fs::create_dir_all(&paths.spool_dir)?;
    let name = format!("{}-{}.json", now_millis(), envelope.notification_id);
    let path = paths.spool_dir.join(name);
    let data = serde_json::to_vec_pretty(envelope)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(path, data)?;
    cap_spool(paths, false)
}

fn cap_spool(paths: &Paths, dry_run: bool) -> io::Result<()> {
    let mut files = dir_files(&paths.spool_dir)?;
    files.sort_by_key(|path| modified_secs(path).unwrap_or(0));
    let cutoff = now_secs().saturating_sub(RETENTION_SECONDS);
    let remove_count = files.len().saturating_sub(MAX_SPOOL_FILES);
    for path in files.iter().take(remove_count) {
        if !dry_run {
            let _ = fs::remove_file(path);
        }
    }
    for path in files {
        if modified_secs(&path).unwrap_or(now_secs()) < cutoff && !dry_run {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn drain_spool(paths: &Paths, notifier: &Arc<Mutex<Option<UnixStream>>>) -> io::Result<()> {
    let mut files = dir_files(&paths.spool_dir)?;
    files.sort_by_key(|path| modified_secs(path).unwrap_or(0));
    for path in files {
        if notifier.lock().unwrap().is_none() {
            break;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        let Ok(envelope) = serde_json::from_str::<NotifyEnvelope>(&text) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        let outcome = process_notify(paths, notifier, &envelope)?;
        if outcome.status == "posted" || outcome.status == "duplicate" {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SweepReport {
    contexts_removed: usize,
    dedup_removed: usize,
    spool_removed: usize,
    legacy_root_removed: usize,
    active_removed: usize,
    total_removed: usize,
}

fn sweep_expired(paths: &Paths, dry_run: bool) -> io::Result<SweepReport> {
    paths.ensure()?;
    let cutoff = now_secs().saturating_sub(RETENTION_SECONDS);
    let contexts_removed = remove_old_files(&paths.contexts_dir, cutoff, dry_run)?;
    let mut dedup_removed = remove_old_files(&paths.dedup_dir, cutoff, dry_run)?;
    dedup_removed += cap_files_by_count(&paths.dedup_dir, MAX_DEDUP_FILES, dry_run)?;
    let spool_removed = remove_old_files(&paths.spool_dir, cutoff, dry_run)?;
    let legacy_root_removed = remove_old_legacy_state_files(paths, cutoff, dry_run)?;
    let mut active_removed = 0;
    let mut active = load_active_index(paths);
    let before = active.len();
    let mut removed_active_ids = Vec::new();
    active.retain(|id, item| {
        let keep = item.updated_at >= cutoff;
        if !keep {
            removed_active_ids.push(id.clone());
        }
        keep
    });
    active_removed += before.saturating_sub(active.len());
    if active.len() > MAX_ACTIVE_NOTIFICATIONS {
        let mut items = active.values().cloned().collect::<Vec<_>>();
        items.sort_by_key(|item| item.updated_at);
        for stale in items
            .into_iter()
            .take(active.len() - MAX_ACTIVE_NOTIFICATIONS)
        {
            active.remove(&stale.id);
            removed_active_ids.push(stale.id);
            active_removed += 1;
        }
    }
    if !dry_run {
        for id in removed_active_ids {
            let _ = fs::remove_file(paths.contexts_dir.join(format!("{id}.json")));
        }
    }
    if !dry_run {
        let data = serde_json::to_vec_pretty(&active)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        fs::write(&paths.active_index, data)?;
    }
    let total_removed =
        contexts_removed + dedup_removed + spool_removed + legacy_root_removed + active_removed;
    Ok(SweepReport {
        contexts_removed,
        dedup_removed,
        spool_removed,
        legacy_root_removed,
        active_removed,
        total_removed,
    })
}

fn remove_old_files(dir: &Path, cutoff: u64, dry_run: bool) -> io::Result<usize> {
    let mut removed = 0;
    for path in dir_files(dir)? {
        if modified_secs(&path).unwrap_or(now_secs()) < cutoff {
            removed += 1;
            if !dry_run {
                let _ = fs::remove_file(path);
            }
        }
    }
    Ok(removed)
}

fn cap_files_by_count(dir: &Path, cap: usize, dry_run: bool) -> io::Result<usize> {
    let mut files = dir_files(dir)?;
    if files.len() <= cap {
        return Ok(0);
    }
    files.sort_by_key(|path| modified_secs(path).unwrap_or(0));
    let remove_count = files.len() - cap;
    for path in files.iter().take(remove_count) {
        if !dry_run {
            let _ = fs::remove_file(path);
        }
    }
    Ok(remove_count)
}

fn remove_old_legacy_state_files(paths: &Paths, cutoff: u64, dry_run: bool) -> io::Result<usize> {
    let hash = Regex::new(r"^[0-9a-f]{64}(?:\.json)?$").unwrap();
    let mut removed = 0;
    for path in dir_files(&paths.state_dir)? {
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if hash.is_match(name) && modified_secs(&path).unwrap_or(now_secs()) < cutoff {
            removed += 1;
            if !dry_run {
                let _ = fs::remove_file(path);
            }
        }
    }
    Ok(removed)
}

fn dir_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>()
        .pipe(Ok)
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

fn modified_secs(path: &Path) -> Option<u64> {
    path.metadata()
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn focus_context(paths: &Paths, context: &str) -> io::Result<bool> {
    let payload = load_context(paths, context)?;
    let Some(payload) = payload else {
        return Ok(false);
    };
    activate_terminal();
    match payload.mux.mux_type.as_str() {
        "zellij" => focus_zellij(&payload.mux),
        "tmux" => focus_tmux(&payload.mux),
        _ => Ok(false),
    }
}

fn load_context(paths: &Paths, context: &str) -> io::Result<Option<NotifyPayload>> {
    let direct = PathBuf::from(context);
    let candidates = if direct.exists() {
        vec![direct]
    } else {
        vec![
            paths.contexts_dir.join(format!("{context}.json")),
            paths.state_dir.join(format!("{context}.json")),
            paths.state_dir.join(context),
        ]
    };
    for path in candidates {
        if path.exists() {
            let text = fs::read_to_string(path)?;
            let payload = serde_json::from_str::<NotifyPayload>(&text)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            return Ok(Some(payload));
        }
    }
    Ok(None)
}

fn activate_terminal() {
    let _ = Command::new("/usr/bin/osascript")
        .args(["-e", "tell application \"Ghostty\" to activate"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .or_else(|_| {
            Command::new("/usr/bin/open")
                .args(["-a", "Ghostty"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
        });
}

fn focus_zellij(mux: &MuxContext) -> io::Result<bool> {
    if mux.session.is_empty() || mux.pane_ref.is_empty() {
        return Ok(false);
    }
    let mut ok = true;
    for mut command in zellij_focus_commands(mux) {
        ok &= run_status_with_timeout(&mut command, Duration::from_secs(2)).unwrap_or(false);
    }
    Ok(ok)
}

fn zellij_focus_commands(mux: &MuxContext) -> Vec<Command> {
    let mut commands = Vec::new();
    if !mux.tab_id.is_empty() {
        let mut command = Command::new("zellij");
        command.args([
            "--session",
            &mux.session,
            "action",
            "go-to-tab-by-id",
            &mux.tab_id,
        ]);
        commands.push(command);
    }
    let mut command = Command::new("zellij");
    command.args([
        "--session",
        &mux.session,
        "action",
        "focus-pane-id",
        &mux.pane_ref,
    ]);
    commands.push(command);
    commands
}

fn focus_tmux(mux: &MuxContext) -> io::Result<bool> {
    let mut ok = true;
    for mut command in tmux_focus_commands(mux) {
        ok &= run_status_with_timeout(&mut command, Duration::from_secs(2)).unwrap_or(false);
    }
    Ok(ok)
}

fn tmux_focus_commands(mux: &MuxContext) -> Vec<Command> {
    let mut commands = Vec::new();
    if !mux.session.is_empty() {
        let mut command = Command::new("tmux");
        command.args(["switch-client", "-t", &mux.session]);
        commands.push(command);
    }
    if !mux.window_id.is_empty() {
        let mut command = Command::new("tmux");
        command.args(["select-window", "-t", &mux.window_id]);
        commands.push(command);
    }
    if !mux.pane_id.is_empty() {
        let mut command = Command::new("tmux");
        command.args(["select-pane", "-t", &mux.pane_id]);
        commands.push(command);
    }
    commands
}

fn run_status_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<bool> {
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command.spawn()?;
    let status = match child.wait_timeout(timeout)? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
    };
    Ok(status.success())
}

fn handle_doctor(paths: &Paths, verbose: bool) {
    println!("agent-signals doctor");
    println!(
        "  {:>7}  state dir: {}",
        if paths.state_dir.exists() {
            "ok"
        } else {
            "MISSING"
        },
        paths.state_dir.display()
    );
    println!(
        "  {:>7}  socket: {}",
        if paths.socket.exists() {
            "ok"
        } else {
            "MISSING"
        },
        paths.socket.display()
    );
    let app_path = home_dir().join("Applications/AgentSignalsNotifier.app");
    println!(
        "  {:>7}  notifier app: {}",
        if app_path.exists() { "ok" } else { "MISSING" },
        app_path.display()
    );
    let daemon_plist = home_dir().join("Library/LaunchAgents/com.nathan.agent-signald.plist");
    let helper_plist =
        home_dir().join("Library/LaunchAgents/com.nathan.AgentSignalsNotifier.plist");
    println!(
        "  {:>7}  daemon LaunchAgent: {}",
        if daemon_plist.exists() {
            "ok"
        } else {
            "MISSING"
        },
        daemon_plist.display()
    );
    println!(
        "  {:>7}  notifier LaunchAgent: {}",
        if helper_plist.exists() {
            "ok"
        } else {
            "MISSING"
        },
        helper_plist.display()
    );

    match send_request(
        paths,
        &WireMessage::Health { verbose },
        Duration::from_millis(900),
    ) {
        Ok(WireMessage::Response {
            status,
            notifier_connected,
            authorization,
            detail,
            ..
        }) => {
            println!("  {:>7}  daemon health: {status}", "ok");
            println!(
                "  {:>7}  notifier connected: {}",
                if notifier_connected == Some(true) {
                    "ok"
                } else {
                    "MISSING"
                },
                notifier_connected.unwrap_or(false)
            );
            let authorization = authorization.unwrap_or_else(|| "unknown".to_string());
            println!(
                "  {:>7}  notification auth: {}",
                if authorization == "authorized" {
                    "ok"
                } else {
                    "check"
                },
                authorization
            );
            if authorization != "authorized" && authorization != "unknown" {
                println!(
                    "          fix: open System Settings > Notifications > AgentSignalsNotifier and allow notifications"
                );
                println!(
                    "               or run: open \"x-apple.systempreferences:com.apple.preference.notifications\""
                );
            }
            if verbose {
                if let Some(detail) = detail {
                    println!("{}", serde_json::to_string_pretty(&detail).unwrap());
                }
            }
        }
        _ => {
            println!("  {:>7}  daemon health: not running", "MISSING");
        }
    }
}

fn handle_sweep_cli(paths: &Paths, legacy: bool, dry_run: bool) -> io::Result<()> {
    let request = WireMessage::Sweep { legacy, dry_run };
    if let Ok(response) = send_request(paths, &request, Duration::from_secs(2)) {
        println!("{}", serde_json::to_string_pretty(&response).unwrap());
        return Ok(());
    }
    let report = sweep_expired(paths, dry_run)?;
    let legacy_detail = if legacy {
        Some(sweep_legacy_terminal_notifier(dry_run)?)
    } else {
        None
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "expired": report,
            "legacy": legacy_detail,
        }))
        .unwrap()
    );
    Ok(())
}

fn sweep_legacy_terminal_notifier(dry_run: bool) -> io::Result<Value> {
    let output = Command::new("pgrep")
        .args(["-fl", "terminal-notifier.*agent-signal focus"])
        .output();
    let matches = match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    if !dry_run && !matches.is_empty() {
        let _ = Command::new("pkill")
            .args(["-f", "terminal-notifier.*agent-signal focus"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    Ok(json!({ "dry_run": dry_run, "matches": matches, "count": matches.len() }))
}

fn run_python_subcommand(subcommand: &str, args: &[String], dry_run: bool) -> i32 {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(&manifest_dir);
    let python = {
        let venv_python = project_root.join(".venv/bin/python");
        if venv_python.exists() {
            venv_python
        } else {
            PathBuf::from("python3")
        }
    };
    let mut command = Command::new(python);
    command.env("PYTHONPATH", project_root.join("src"));
    command.arg("-m").arg("agent_signals.cli");
    if dry_run {
        command.arg("--dry-run");
    }
    command.arg(subcommand);
    command.args(args);
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("agent-signal {subcommand}: failed to run Python compatibility path: {err}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_claude_stop_payload() {
        let raw = r#"{
          "type": "Stop",
          "hook_event_name": "Stop",
          "transcript_path": "/Users/nathan/.claude/projects/example/abc123.jsonl",
          "cwd": "/Users/nathan/Developer/proj/test",
          "thread-id": "thread_abc123",
          "turn-id": "turn_001"
        }"#;
        let event = parse_hook_payload("Claude Code", raw).unwrap();
        assert_eq!(event.client, "Claude Code");
        assert_eq!(event.thread_id, "thread_abc123");
        assert_eq!(event.turn_id, "turn_001");
        assert_eq!(event.cwd_name(), "test");
    }

    #[test]
    fn ignores_irrelevant_events() {
        assert!(parse_hook_payload("", r#"{"type":"tool_call"}"#).is_none());
    }

    #[test]
    fn notification_event_uses_hook_message() {
        let raw = r#"{
          "hook_event_name": "Notification",
          "session_id": "sess_1",
          "cwd": "/Users/nathan/Developer/proj/test",
          "message": "Claude needs your permission to use Bash"
        }"#;
        let event = parse_hook_payload("Claude Code", raw).unwrap();
        assert_eq!(event.event_type, "Notification");
        let envelope = build_envelope(event);
        assert_eq!(envelope.severity, "needs_input");
        assert!(envelope.payload.title.contains("needs input"));
        assert_eq!(
            envelope.payload.message,
            "Claude needs your permission to use Bash"
        );
    }

    #[test]
    fn suppresses_json_only_turn_completion() {
        // Codex Desktop's title-generation turn: the whole response is JSON.
        let raw = r#"{
          "type": "agent-turn-complete",
          "thread-id": "t1",
          "turn-id": "u1",
          "cwd": "/tmp/project",
          "last-assistant-message": "{\"title\":\"Create Claude Code ACP branch\"}"
        }"#;
        let event = parse_hook_payload("Codex Desktop", raw).unwrap();
        assert!(is_suppressed_turn(&event));
    }

    #[test]
    fn keeps_prose_turn_completion() {
        let raw = r#"{
          "type": "agent-turn-complete",
          "thread-id": "t1",
          "turn-id": "u1",
          "cwd": "/tmp/project",
          "last-assistant-message": "Created the files you asked for. No tests run."
        }"#;
        let event = parse_hook_payload("Codex Desktop", raw).unwrap();
        assert!(!is_suppressed_turn(&event));
    }

    #[test]
    fn keeps_notification_event_even_when_message_is_jsonish() {
        // A needs-input Notification must always surface, JSON-shaped or not.
        let raw = r#"{
          "hook_event_name": "Notification",
          "session_id": "s1",
          "cwd": "/tmp/project",
          "message": "{\"x\":1}"
        }"#;
        let event = parse_hook_payload("Claude Code", raw).unwrap();
        assert!(!is_suppressed_turn(&event));
    }

    #[test]
    fn dedup_key_is_stable() {
        let e1 = AgentEvent {
            client: "Claude Code".to_string(),
            thread_id: "t1".to_string(),
            turn_id: "turn1".to_string(),
            event_type: String::new(),
            cwd: String::new(),
            transcript_path: String::new(),
            mux: MuxContext::default(),
            raw: Value::Null,
        };
        let mut e2 = e1.clone();
        assert_eq!(e1.dedup_key(), e2.dedup_key());
        e2.turn_id = "turn2".to_string();
        assert_ne!(e1.dedup_key(), e2.dedup_key());
    }

    #[test]
    fn context_roundtrip() {
        let temp = tempdir().unwrap();
        let paths = Paths::from_state_dir(temp.path().to_path_buf());
        paths.ensure().unwrap();
        let event = AgentEvent {
            client: "Claude Code".to_string(),
            event_type: "Stop".to_string(),
            thread_id: "t1".to_string(),
            turn_id: "t2".to_string(),
            cwd: "/tmp/project".to_string(),
            transcript_path: String::new(),
            mux: MuxContext {
                mux_type: "zellij".to_string(),
                session: "main".to_string(),
                pane_ref: "terminal_12".to_string(),
                ..MuxContext::default()
            },
            raw: Value::Null,
        };
        let envelope = build_envelope(event);
        save_context(&paths, &envelope).unwrap();
        let loaded = load_context(&paths, &envelope.notification_id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.mux.mux_type, "zellij");
        assert_eq!(loaded.mux.pane_ref, "terminal_12");
    }

    #[test]
    fn active_index_is_capped() {
        let temp = tempdir().unwrap();
        let paths = Paths::from_state_dir(temp.path().to_path_buf());
        paths.ensure().unwrap();
        for idx in 0..40 {
            let event = AgentEvent {
                client: "Codex".to_string(),
                thread_id: format!("t{idx}"),
                turn_id: "turn".to_string(),
                cwd: format!("/tmp/{idx}"),
                mux: MuxContext::default(),
                event_type: String::new(),
                transcript_path: String::new(),
                raw: Value::Null,
            };
            let mut envelope = build_envelope(event);
            envelope.notification_id = format!("{idx:032}");
            envelope.group_key = format!("group-{idx}");
            update_active_index(&paths, &envelope).unwrap();
        }
        assert!(load_active_index(&paths).len() <= MAX_ACTIVE_NOTIFICATIONS);
    }

    #[test]
    fn sweep_removes_expired_files() {
        let temp = tempdir().unwrap();
        let paths = Paths::from_state_dir(temp.path().to_path_buf());
        paths.ensure().unwrap();
        let stale = paths.contexts_dir.join("stale.json");
        fs::write(&stale, "{}").unwrap();
        let report = sweep_expired(&paths, false).unwrap();
        assert_eq!(report.total_removed, 0);
    }

    #[test]
    fn dedup_dir_is_capped_by_count() {
        let temp = tempdir().unwrap();
        let paths = Paths::from_state_dir(temp.path().to_path_buf());
        paths.ensure().unwrap();
        let extra = 50;
        let total = MAX_DEDUP_FILES + extra;
        // Create more than the cap, with strictly increasing mtimes so the
        // newest (highest index) files are the ones expected to survive.
        let base = UNIX_EPOCH + Duration::from_secs(1_000_000);
        for idx in 0..total {
            let path = paths.dedup_dir.join(format!("{idx:08}"));
            fs::write(&path, "x").unwrap();
            let mtime = base + Duration::from_secs(idx as u64);
            fs::File::open(&path)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
        }
        let removed = cap_files_by_count(&paths.dedup_dir, MAX_DEDUP_FILES, false).unwrap();
        assert_eq!(removed, extra);
        let remaining = dir_files(&paths.dedup_dir).unwrap();
        assert!(remaining.len() <= MAX_DEDUP_FILES);
        // The newest files (indices extra..total) must still be present.
        for idx in extra..total {
            assert!(paths.dedup_dir.join(format!("{idx:08}")).exists());
        }
        // The oldest files (indices 0..extra) must have been removed.
        for idx in 0..extra {
            assert!(!paths.dedup_dir.join(format!("{idx:08}")).exists());
        }
    }

    #[test]
    fn focus_command_builders_are_explicit() {
        let zellij = MuxContext {
            mux_type: "zellij".to_string(),
            session: "main".to_string(),
            tab_id: "3".to_string(),
            pane_ref: "terminal_12".to_string(),
            ..MuxContext::default()
        };
        assert_eq!(zellij_focus_commands(&zellij).len(), 2);
        let tmux = MuxContext {
            mux_type: "tmux".to_string(),
            session: "s".to_string(),
            window_id: "@2".to_string(),
            pane_id: "%3".to_string(),
            ..MuxContext::default()
        };
        assert_eq!(tmux_focus_commands(&tmux).len(), 3);
    }
}

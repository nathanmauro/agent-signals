#!/usr/bin/env bash
# uninstall-native.sh — tear down the native macOS notification daemon stack for agent-signals.
#
# Reverses install-native.sh:
#   - unloads + removes both LaunchAgents (daemon + notifier app)
#   - removes ~/Applications/AgentSignalsNotifier.app
#   - removes ~/.local/bin/agent-signal, agent-signald, and the hook wrapper shims
#   - removes the agent-response-notify / agent-focus-pane symlinks from voice/bin
#   - leaves the state dir alone by default (pass --purge-state to delete it)
#
# Tolerant of missing pieces: safe to run even on a partial install.
#
# Does NOT touch hook wiring in ~/.claude/settings.json — remove that by hand.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/Applications/AgentSignalsNotifier.app"
LA_DIR="$HOME/Library/LaunchAgents"
STATE_DIR="$HOME/.local/state/agent-signals"
VOICE_BIN="$HOME/Developer/config/voice/bin"

DAEMON_PLIST="com.nathan.agent-signald"
NOTIFIER_PLIST="com.nathan.AgentSignalsNotifier"

PURGE_STATE=0

# ---------------------------------------------------------------------------
# 0. Args
# ---------------------------------------------------------------------------
for arg in "$@"; do
  case "$arg" in
    --purge-state) PURGE_STATE=1 ;;
    -h|--help)
      echo "usage: uninstall-native.sh [--purge-state]"
      echo "  --purge-state   also delete $STATE_DIR (logs/state)"
      exit 0
      ;;
    *)
      echo "!! unknown argument: $arg" >&2
      echo "   usage: uninstall-native.sh [--purge-state]" >&2
      exit 1
      ;;
  esac
done

echo "==> agent-signals native uninstall"
echo "    bin:   $BIN_DIR"
echo "    app:   $APP_DIR"
echo "    state: $STATE_DIR"

# ---------------------------------------------------------------------------
# 1. Unload + remove LaunchAgents
# ---------------------------------------------------------------------------
echo "==> removing LaunchAgents"
for plist in "$DAEMON_PLIST" "$NOTIFIER_PLIST"; do
  path="$LA_DIR/$plist.plist"
  launchctl unload "$path" 2>/dev/null || true
  if [[ -f "$path" ]]; then
    rm -f "$path" && echo "    removed $path"
  else
    echo "    (skip) no plist at $path"
  fi
done

# ---------------------------------------------------------------------------
# 2. Remove the notifier .app
# ---------------------------------------------------------------------------
echo "==> removing notifier app"
if [[ -d "$APP_DIR" ]]; then
  rm -rf "$APP_DIR" && echo "    removed $APP_DIR"
else
  echo "    (skip) no app at $APP_DIR"
fi

# ---------------------------------------------------------------------------
# 3. Remove installed binaries + hook wrappers
# ---------------------------------------------------------------------------
# Derive the wrapper list from the repo's bin/ when available so it never drifts
# from what install-native.sh installs; fall back to the known set otherwise.
wrappers=()
if [[ -d "$repo_root/bin" ]]; then
  for wrapper in "$repo_root"/bin/*; do
    [[ -f "$wrapper" ]] && wrappers+=("$(basename "$wrapper")")
  done
fi
if [[ ${#wrappers[@]} -eq 0 ]]; then
  wrappers=(agent-response-notify agent-focus-pane agent-speak-last \
            agent-voice-stop voice-toggle kokoro-speak)
fi

echo "==> removing binaries and hook wrappers"
for name in agent-signal agent-signald "${wrappers[@]}"; do
  dst="$BIN_DIR/$name"
  if [[ -e "$dst" || -L "$dst" ]]; then
    rm -f "$dst" && echo "    removed $dst"
  else
    echo "    (skip) no binary at $dst"
  fi
done

# install-native.sh also symlinks two wrappers into the voice config bin.
echo "==> removing voice/bin symlinks"
for name in agent-response-notify agent-focus-pane; do
  dst="$VOICE_BIN/$name"
  if [[ -L "$dst" || -e "$dst" ]]; then
    rm -f "$dst" && echo "    removed $dst"
  else
    echo "    (skip) no symlink at $dst"
  fi
done

# ---------------------------------------------------------------------------
# 4. State dir (preserved unless --purge-state)
# ---------------------------------------------------------------------------
if [[ "$PURGE_STATE" -eq 1 ]]; then
  echo "==> purging state dir"
  if [[ -d "$STATE_DIR" ]]; then
    rm -rf "$STATE_DIR" && echo "    removed $STATE_DIR"
  else
    echo "    (skip) no state dir at $STATE_DIR"
  fi
else
  echo "==> preserving state dir"
  echo "    kept $STATE_DIR (pass --purge-state to delete it)"
fi

# ---------------------------------------------------------------------------
# 5. Summary
# ---------------------------------------------------------------------------
echo "==> done."
echo "    removed: LaunchAgents ($DAEMON_PLIST, $NOTIFIER_PLIST)"
echo "    removed: $APP_DIR"
echo "    removed: agent-signal, agent-signald + hook wrappers from $BIN_DIR"
echo "    removed: agent-response-notify, agent-focus-pane symlinks from $VOICE_BIN"
if [[ "$PURGE_STATE" -eq 1 ]]; then
  echo "    removed: $STATE_DIR"
else
  echo "    kept:    $STATE_DIR"
fi
echo "    NOTE: hook wiring in ~/.claude/settings.json was NOT touched — remove it manually if desired."

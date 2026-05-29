#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="$HOME/.local/bin"
app_dir="$HOME/Applications"
state_dir="$HOME/.local/state/agent-signals"
launch_agents="$HOME/Library/LaunchAgents"
uid="$(id -u)"

mkdir -p "$bin_dir" "$app_dir" "$state_dir" "$launch_agents"

cargo build --release --manifest-path "$repo_root/native/rust/Cargo.toml"
rm -f "$bin_dir/agent-signal" "$bin_dir/agent-signald"
install -m 0755 "$repo_root/native/rust/target/release/agent-signal" "$bin_dir/agent-signal"
install -m 0755 "$repo_root/native/rust/target/release/agent-signald" "$bin_dir/agent-signald"
ln -sf "$repo_root/bin/agent-response-notify" "$bin_dir/agent-response-notify"
ln -sf "$repo_root/bin/agent-focus-pane" "$bin_dir/agent-focus-pane"

echo "==> Installing hook wrappers → $bin_dir"
for wrapper in "$repo_root"/bin/*; do
  [ -f "$wrapper" ] || continue
  install -m 0755 "$wrapper" "$bin_dir/$(basename "$wrapper")"
done

app_path="$("$repo_root/native/swift/build-app.sh")"
rm -rf "$app_dir/AgentSignalsNotifier.app"
cp -R "$app_path" "$app_dir/AgentSignalsNotifier.app"

install -m 0644 "$repo_root/launchd/com.nathan.agent-signald.plist" "$launch_agents/com.nathan.agent-signald.plist"
install -m 0644 "$repo_root/launchd/com.nathan.AgentSignalsNotifier.plist" "$launch_agents/com.nathan.AgentSignalsNotifier.plist"

if [[ -d "$HOME/Developer/config/voice/bin" ]]; then
  ln -sf "$repo_root/bin/agent-response-notify" "$HOME/Developer/config/voice/bin/agent-response-notify"
  ln -sf "$repo_root/bin/agent-focus-pane" "$HOME/Developer/config/voice/bin/agent-focus-pane"
fi

# (Re)load each LaunchAgent. bootstrap fails if the service is already
# loaded (re-install), so it's best-effort; kickstart -k then guarantees a
# fresh start from the new on-disk binary whether or not bootstrap ran.
reload_agent() {
  local label="$1"
  local plist="$launch_agents/$label.plist"
  launchctl bootout "gui/$uid/$label" >/dev/null 2>&1 || true
  launchctl bootstrap "gui/$uid" "$plist" >/dev/null 2>&1 || true
  launchctl kickstart -k "gui/$uid/$label"
}

reload_agent com.nathan.agent-signald
reload_agent com.nathan.AgentSignalsNotifier

"$bin_dir/agent-signal" sweep --legacy --dry-run
"$bin_dir/agent-signal" doctor --verbose

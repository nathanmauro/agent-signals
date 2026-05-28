#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src_dir="$script_dir/AgentSignalsNotifier"
build_dir="$script_dir/build"
app="$build_dir/AgentSignalsNotifier.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

swiftc \
  "$src_dir/main.swift" \
  -O \
  -framework Foundation \
  -framework UserNotifications \
  -o "$app/Contents/MacOS/AgentSignalsNotifier"

cp "$src_dir/Info.plist" "$app/Contents/Info.plist"

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$app" >/dev/null
fi

printf '%s\n' "$app"

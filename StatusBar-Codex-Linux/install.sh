#!/bin/sh
set -eu

prefix="${PREFIX:-$HOME/.local}"
bin_dir="$prefix/bin"
apps_dir="$HOME/.local/share/applications"

mkdir -p "$bin_dir" "$apps_dir"

install -m 0755 StatusBar-Codex-Linux "$bin_dir/StatusBar-Codex-Linux"
install -m 0644 StatusBar-Codex-Linux.desktop.example "$apps_dir/StatusBar-Codex-Linux.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$apps_dir" >/dev/null 2>&1 || true
fi

printf 'Installed StatusBar-Codex-Linux to %s\n' "$bin_dir/StatusBar-Codex-Linux"
printf 'Installed desktop entry to %s\n' "$apps_dir/StatusBar-Codex-Linux.desktop"
printf 'Run it with: StatusBar-Codex-Linux\n'

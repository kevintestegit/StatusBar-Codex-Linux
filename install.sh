#!/bin/sh
set -eu

prefix="${PREFIX:-$HOME/.local}"
bin_dir="$prefix/bin"
apps_dir="$HOME/.local/share/applications"

mkdir -p "$bin_dir" "$apps_dir"

install -m 0755 codex-usage-tray "$bin_dir/codex-usage-tray"
install -m 0644 codex-usage-tray.desktop.example "$apps_dir/codex-usage-tray.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$apps_dir" >/dev/null 2>&1 || true
fi

printf 'Installed codex-usage-tray to %s\n' "$bin_dir/codex-usage-tray"
printf 'Installed desktop entry to %s\n' "$apps_dir/codex-usage-tray.desktop"
printf 'Run it with: codex-usage-tray\n'

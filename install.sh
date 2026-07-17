#!/bin/sh
set -eu

repo="kevintestegit/StatusBar-Codex-Linux"
branch="main"
bin_name="codex-usage-tray"
prefix="${PREFIX:-$HOME/.local}"
bin_dir="$prefix/bin"
apps_dir="$HOME/.local/share/applications"
config_dir="$HOME/.config/$bin_name"

# deps
if command -v apt-get >/dev/null 2>&1; then
  pkgs="libgtk-3-dev libayatana-appindicator3-dev libgtk-layer-shell-dev"
  for pkg in $pkgs; do
    if ! dpkg -s "$pkg" >/dev/null 2>&1; then
      echo "Installing $pkg..."
      sudo apt-get install -y "$pkg"
    fi
  done
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust not found. Install via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  exit 1
fi

tmpdir="$(mktemp -d)"
echo "Downloading $repo..."
curl -fsSL "https://github.com/$repo/archive/refs/heads/$branch.tar.gz" | tar -xz -C "$tmpdir"
srcdir="$tmpdir/StatusBar-Codex-Linux-$branch"

echo "Building..."
(cd "$srcdir" && cargo build --release)

mkdir -p "$bin_dir" "$apps_dir" "$config_dir"
install -m 0755 "$srcdir/target/release/statusbar-codex-linux" "$bin_dir/$bin_name"
install -m 0644 "$srcdir/StatusBar-Codex-Linux.desktop.example" "$apps_dir/$bin_name.desktop"

if ! grep -q "$bin_dir" "$HOME/.profile" 2>/dev/null; then
  printf '\nexport PATH="$PATH:%s"\n' "$bin_dir" >> "$HOME/.profile"
fi

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$apps_dir" >/dev/null 2>&1 || true
fi

rm -rf "$tmpdir"

echo "Installed $bin_name to $bin_dir/$bin_name"
echo "Run: DISPLAY=:0 $bin_name"
echo ""
echo "For autostart:"
echo "  mkdir -p ~/.config/autostart"
echo "  cp $apps_dir/$bin_name.desktop ~/.config/autostart/"

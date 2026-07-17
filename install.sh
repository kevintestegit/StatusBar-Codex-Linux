#!/bin/sh
set -eu

repo="kevintestegit/StatusBar-Codex-Linux"
branch="main"
bin_name="codex-usage-tray"
prefix="${PREFIX:-$HOME/.local}"
bin_dir="$prefix/bin"
apps_dir="$HOME/.local/share/applications"
config_dir="$HOME/.config/$bin_name"

prompt_yes_no() {
  printf "%s (Y/n) " "$1"
  read -r ans </dev/tty
  case "$ans" in
    [Nn]*) return 1 ;;
    *) return 0 ;;
  esac
}

# deps
if command -v apt-get >/dev/null 2>&1; then
  pkgs="libgtk-3-dev libayatana-appindicator3-dev libgtk-layer-shell-dev"
  missing=""
  for pkg in $pkgs; do
    if ! dpkg -s "$pkg" >/dev/null 2>&1; then
      missing="$missing $pkg"
    fi
  done
  if [ -n "$missing" ]; then
    if prompt_yes_no "Missing system deps:$missing. Install?"; then
      sudo apt-get install -y $missing
    else
      echo "Aborted."
      exit 1
    fi
  fi
fi

if ! command -v cargo >/dev/null 2>&1; then
  if prompt_yes_no "Rust not found. Install Rust via rustup?"; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    . "$HOME/.cargo/env"
  else
    echo "Aborted."
    exit 1
  fi
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

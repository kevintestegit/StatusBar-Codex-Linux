# Codex Usage Tray

<p align="center">
  <strong>A small native Linux tray applet for watching local Codex usage without opening a terminal.</strong>
</p>

<p align="center">
  <img width="260" alt="Codex Usage Tray menu" src="https://github.com/user-attachments/assets/de3ac8bb-75ae-4f6f-a974-4aca186d2f89">
</p>

<p align="center">
  <img width="760" alt="Codex Usage details dashboard" src="https://github.com/user-attachments/assets/aad08a49-1a9b-406d-abb2-b7f3f3b8b523">
</p>

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-stable-b7410e?style=flat-square">
  <img alt="GTK" src="https://img.shields.io/badge/GTK-3-4a86cf?style=flat-square">
  <img alt="AppIndicator" src="https://img.shields.io/badge/Ayatana-AppIndicator-35a16b?style=flat-square">
  <img alt="Wayland" src="https://img.shields.io/badge/Wayland-gtk--layer--shell-6b7280?style=flat-square">
</p>

## Pr's are welcome!

Codex Usage Tray reads your local Codex session event logs and turns them into a compact status indicator: current rate-limit usage, reset times, token totals, and API-equivalent cost estimates. It is built as a native Linux desktop applet, not a shell widget or window-manager plugin.

## What It Is

This project is best described as:

- **Rust GTK tray applet**
- **Linux AppIndicator app**
- **GTK3 + Ayatana AppIndicator status applet**
- **Wayland-friendly GTK tray utility**

It is **not** a Niri app, Quickshell widget, GNOME extension, KDE plasmoid, or Codex Desktop plugin. It should work on any Linux desktop that exposes AppIndicator/status-notifier items, including setups that use panels such as Waybar, AGS, Quickshell, KDE Plasma, GNOME with an indicator extension, and many wlroots-based environments.

## Tech Stack

- **Rust** — native binary with no background web runtime.
- **GTK3** — tray menu, labels, settings-style UI, and transient windows.
- **Ayatana AppIndicator** — Linux tray/status-notifier integration.
- **GTK Layer Shell** — fullscreen Wayland overlay used by party mode.
- **Cairo** — lightweight confetti drawing for reset celebrations.
- **Local JSONL parsing** — reads Codex session events directly from disk.

## Features

- Shows 5-hour and weekly Codex rate-limit usage in the tray.
- Displays reset countdowns and local reset times.
- Lets you choose the refresh interval from the tray menu.
- Tracks whether the current 5-hour window is ahead of expected pace.
- Estimates today's, current month's, and all-time API-equivalent cost.
- Breaks down input, cached input, output, and reasoning tokens.
- Opens a local HTML dashboard with model-level usage details.
- Sends desktop notifications for reset events and fast usage pace.
- Includes optional party mode: a fullscreen confetti overlay when a rate-limit window resets.
- Lets you toggle party mode from the tray menu; reset notifications still work when it is off.
- Keeps background work modest: cached JSONL parsing, configurable refresh intervals, and no network calls.
- Reads only local Codex JSONL event files.

## Privacy

The app does not send usage data anywhere. It reads local files under:

```text
$CODEX_HOME/sessions
$CODEX_HOME/archived_sessions
```

If `CODEX_HOME` is not set, it falls back to:

```text
$HOME/.codex
```

The repository is designed so private data is not committed accidentally:

- `.gitignore` excludes `.codex/`, session JSONL files, memories, rollout summaries, `.env` files, logs, caches, screenshots, and build output.
- Generated HTML and tray icons are written to the system temp directory.
- No username, hostname, home directory, or machine-specific path is required in source.

Before publishing screenshots, use synthetic or redacted data. Real tray screenshots may reveal usage totals, model names, plan names, or reset timing.

## Requirements

Install Rust plus the native GTK/AppIndicator development libraries. `gtk-layer-shell` is required for the fullscreen party-mode reset overlay.

On Arch Linux:

```bash
sudo pacman -S rust pkgconf gtk3 libayatana-appindicator gtk-layer-shell
```

On Debian/Ubuntu:

```bash
sudo apt install cargo pkg-config libgtk-3-dev libayatana-appindicator3-dev libgtk-layer-shell-dev
```

Package names vary slightly by distro. The build script checks for:

```text
gtk+-3.0
ayatana-appindicator3-0.1
gtk-layer-shell-0
```

## Build

```bash
cargo build --release
```

Run it directly:

```bash
./target/release/codex-usage-tray
```

Print a terminal summary:

```bash
./target/release/codex-usage-tray --once
```

Print the HTML dashboard to stdout:

```bash
./target/release/codex-usage-tray --html
```

## Install Locally

The easiest path: download the latest GitHub Release, unpack it, and run:

```bash
./install.sh
```

The installer copies the binary and desktop entry only. It does not enable login autostart.

Or just ask your AI agent to install the latest release of `codex-linux-usage-tray-indicator` from GitHub.

Agents can follow [AGENT_INSTALL.md](AGENT_INSTALL.md) for exact download, install, verification, and post-install explanation steps.

Copy the binary somewhere on your `PATH`:

```bash
install -Dm755 target/release/codex-usage-tray ~/.local/bin/codex-usage-tray
```

Create a desktop entry:

```ini
[Desktop Entry]
Type=Application
Name=Codex Usage Tray
Comment=Show local Codex usage, rate limits, and token-cost estimates
Exec=codex-usage-tray
Icon=codex-desktop
Terminal=false
Categories=Utility;Development;
```

Save that as:

```text
~/.local/share/applications/codex-usage-tray.desktop
```

Login autostart is manual and optional. To enable it, copy the same desktop entry to:

```text
~/.config/autostart/codex-usage-tray.desktop
```

## Optional: Start Only When Codex Desktop Is Running

If you prefer the tray to appear only while Codex Desktop is open, run it through a small user-session watcher instead of direct autostart. Keep the watcher outside this repo if it contains local process names or paths for your machine.

A generic version can watch for known Codex Desktop process names and launch `codex-usage-tray` when needed. Different installs may expose different process names, so this is intentionally not hard-coded into the app.

## How It Works

Codex stores session events as JSONL. This app walks the local session directories, parses `token_count` events, tracks the latest rate-limit payload, and aggregates usage by day, month, and model. It keeps a lightweight file cache keyed by file size and modification timestamp so refreshes stay cheap.

The tray itself is native GTK3 with Ayatana AppIndicator. The reset celebration overlay uses `gtk-layer-shell`, which makes it suitable for Wayland compositors.

## System Usage

Codex Usage Tray is designed to sit quietly in the background. It does not poll the network, does not keep a database, and does not reparse unchanged session files on every refresh. The refresh interval is configurable from the tray menu, so you can choose a live-feeling 5-second update or a quieter 5-minute cadence.

## Party Mode

When a 5-hour or weekly rate-limit window resets, Codex Usage Tray can show a short fullscreen "rate limit has been reset" celebration overlay with confetti. The overlay is implemented with GTK Layer Shell, which is why `gtk-layer-shell` is part of the native dependency stack.

This is intentionally separate from notifications: desktop notifications always fire, while the fullscreen confetti overlay can be turned on or off from the tray menu.

The setting is stored in:

```text
$XDG_CONFIG_HOME/codex-usage-tray/config.json
```

If `XDG_CONFIG_HOME` is not set, it falls back to:

```text
$HOME/.config/codex-usage-tray/config.json
```

The Details dashboard shows whether party mode is currently enabled.

## Naming

Good repository names:

- `codex-usage-tray`
- `codex-usage-indicator`
- `codex-linux-tray`

Good short description:

```text
A native Linux tray applet for local Codex usage, rate limits, and token-cost estimates.
```

Good topic tags:

```text
codex, rust, gtk, appindicator, tray, linux, wayland, gtk-layer-shell
```

## Caveats

- Cost estimates use public API-style token pricing where known. Subscription billing and Codex product limits are not the same thing.
- Model prices can change; update `price()` when needed.
- The parser depends on Codex local event shapes that may change over time.
- AppIndicator visibility depends on your desktop shell or panel setup.

## License

Licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

```text
        _../|_
      ='__   _~-.
           \'  ~-`\._
                 |/~'
```

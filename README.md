# DeckLink

**Steam Deck as a Wi‑Fi Xbox controller / keyboard+mouse for a Windows PC.**

DeckLink captures Deck input, maps it to gamepad or desktop HID reports, and streams them over UDP to a small Windows host that uses **ViGEmBus** (virtual Xbox 360) plus SendInput for mouse/keyboard.

Same Wi‑Fi network required. No Bluetooth — UDP only.

**Desktop Mode only** on the Deck — launch from the KDE application menu.

## Features

- **Xbox Controller** — sticks, triggers, face buttons, D-pad, grips (ViGEm on PC)
- **Keyboard & Mouse** — soft keyboard + Deck trackpads
- **Seamless switch** — UI tabs, or **Select+Start** on the Deck
- **Recent hosts** — remembers PCs you have connected to

## Requirements

### Steam Deck

- SteamOS / Linux x86_64
- Same LAN (Wi‑Fi) as the PC
- **Desktop Mode** for install and daily use

### Windows PC

1. Install [ViGEmBus](https://github.com/ViGEm/ViGEmBus/releases) (one-time driver)
2. Run `decklink-host.exe` (allow firewall UDP port **31415** if prompted)

## Quick start

1. On the PC: start `decklink-host` (shows bind address / port).
2. On the Deck: open **DeckLink** → enter the PC’s LAN IP → **Connect**.
3. Use Xbox or Keyboard & Mouse mode as usual.

CLI (Deck):

```bash
decklink-bt --host 192.168.1.20 --connect
# or headless:
decklink-bt --headless --host 192.168.1.20 --connect
```

PC:

```bash
decklink-host
# optional:
decklink-host --bind 0.0.0.0:31415 --name "Living Room PC"
```

## Install on Steam Deck (Desktop Mode)

### From GitHub Releases (recommended)

1. Download `decklink-bt-linux-x86_64.tar.gz` from the [latest release](https://github.com/bastianjosekottekudy-cmyk/DeckLink-BT/releases/latest).
2. In Desktop Mode Konsole:

```bash
cd ~/Downloads   # or wherever you saved the file
tar -xzf decklink-bt-linux-x86_64.tar.gz
bash decklink-bt-linux-x86_64/scripts/install-deck.sh ./decklink-bt-linux-x86_64.tar.gz
```

Use `bash …` (not `./scripts/…`) so a missing execute bit cannot cause “Permission denied”. Enter your sudo password when prompted (udev + SteamOS read-only unlock).

3. Open **DeckLink** → set PC IP → Connect.

### Windows host binary

Build on Windows (this machine):

```bash
cargo build --release -p decklink-host
# → target\release\decklink-host.exe
```

Or download a release asset if published as `decklink-host-windows-x86_64.zip`.

### Uninstall (Deck)

```bash
bash scripts/uninstall-deck.sh
```

Or paste this in Konsole (Desktop Mode) to wipe everything without the script:

```bash
pkill -x decklink-bt 2>/dev/null || true
rm -rf ~/.local/share/decklink-bt ~/.config/decklink-bt
rm -f ~/.local/bin/decklink-bt ~/.local/share/applications/decklink-bt.desktop
rm -rf ~/homebrew/plugins/decklink_bt
sudo steamos-readonly disable 2>/dev/null || true
sudo rm -f /etc/udev/rules.d/99-decklink-bt.rules
sudo udevadm control --reload-rules; sudo udevadm trigger
sudo steamos-readonly enable 2>/dev/null || true
```

### Build from source on Deck

```bash
git clone https://github.com/bastianjosekottekudy-cmyk/DeckLink-BT.git
cd DeckLink-BT
cargo build --release -p decklink-app
./scripts/install-deck.sh
```

## Publishing releases (maintainers)

There is **no GitHub Actions CI/release workflow**. Build the Deck tarball locally (WSL on Windows) and replace assets on the current version tag:

```bash
python scripts/publish_release.py
```

Also build and attach the Windows host when cutting a Wi‑Fi release:

```bash
cargo build --release -p decklink-host
```

- Reuses `Cargo.toml` version (e.g. `1.0.0` → release `v1.0.0`) and **replaces** assets.
- Bump version **only** when asked explicitly:

```bash
python scripts/publish_release.py --bump patch   # or minor / major
```

Enable auto-publish after commits that touch `crates/` / `packaging/`:

```bash
python scripts/install_git_hooks.py
```

Skip once with `DECKLINK_SKIP_PUBLISH=1` or `git commit --no-verify`.

## Protocol

UDP port **31415**, magic `DLNK`, version 1. Deck sends Hello → host HelloAck, then HID frames (gamepad=1, mouse=2, keyboard=3).

## Architecture

```
Deck:  hidraw → profiles → UDP (decklink-net)
PC:    decklink-host → ViGEm (Xbox 360) + SendInput (mouse/keyboard)
```

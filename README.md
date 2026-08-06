# DeckLink

**Steam Deck as a Wi‑Fi Xbox controller / keyboard+mouse for a Windows PC.**

Deck streams input over UDP. The Windows host shows a tray UI, auto-installs **ViGEmBus** if missing, and appears as an Xbox 360 pad (plus SendInput mouse/keyboard).

Same Wi‑Fi network required. No Bluetooth. No typing IP addresses — the Deck finds the PC automatically.

**Desktop Mode only** on the Deck — launch from the KDE application menu.

## Features

- **Xbox Controller** — sticks, triggers, face buttons, D-pad → ViGEm Xbox 360
- **Keyboard & Mouse** — Steam trackpads → mouse; soft keyboard for typing
- **Auto Wi‑Fi find** — Connect discovers `decklink-host` on the LAN
- **Windows tray app** — close to tray; ViGEmBus installed on first run (UAC)
- **Select+Start** — toggle Xbox ↔ Keyboard+Mouse

## Requirements

### Steam Deck

- SteamOS / Linux x86_64
- Same LAN (Wi‑Fi) as the PC
- **Desktop Mode** for install and daily use

### Windows PC

- Run `decklink-host.exe` once (allow firewall UDP **31415** if prompted)
- On first launch it installs **ViGEmBus** automatically (UAC yes) — or place `ViGEmBusSetup_x64.msi` next to the exe

## Quick start

1. **PC:** run `decklink-host.exe` (tray app). Approve ViGEm install if asked.
2. **Deck:** open **DeckLink** → **Connect** (no IP).
3. Stay on **Xbox Controller** for games. Use **Keyboard & Mouse** for trackpad mouse + soft keys.

CLI (optional):

```bash
# Deck — auto-discover
decklink-bt --connect
# or pin a host
decklink-bt --host 192.168.1.20 --connect
```

```bash
# PC
decklink-host
decklink-host --headless
decklink-host --skip-vigem-install
```

## Install on Steam Deck (Desktop Mode)

### From GitHub Releases (recommended)

1. Download `decklink-bt-linux-x86_64.tar.gz` from the [latest release](https://github.com/bastianjosekottekudy-cmyk/DeckLink-BT/releases/latest).
2. In Desktop Mode Konsole:

```bash
cd ~/Downloads
tar -xzf decklink-bt-linux-x86_64.tar.gz
bash decklink-bt-linux-x86_64/scripts/install-deck.sh ./decklink-bt-linux-x86_64.tar.gz
```

3. Open **DeckLink** → **Connect**.

### Windows host

Download `decklink-host-windows-x86_64.zip` from the same release, extract, run `decklink-host.exe`.

Build locally:

```bash
cargo build --release -p decklink-host
```

ViGEmBus setup is GPL-3 (Nefarius). DeckLink downloads/bundles their official MSI and runs it elevated; see [ViGEmBus releases](https://github.com/nefarius/ViGEmBus/releases).

## Architecture

```
Deck:  hidraw → profiles → UDP discover/hello/HID
PC:    decklink-host (tray) → ViGEm Xbox 360 + SendInput
```

UDP port **31415**, magic `DLNK`. Discover/Announce for LAN find; then Hello + HID frames.

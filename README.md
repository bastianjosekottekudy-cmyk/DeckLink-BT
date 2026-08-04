# DeckLink BT

**Steam Deck as a universal, driverless Bluetooth gamepad / keyboard+mouse.**

DeckLink BT turns your Steam Deck into a BLE HID device (HOGP). The host PC, phone, tablet, or console sees a standard Bluetooth gamepad or keyboard+mouse — **no host drivers or companion apps**.

**Desktop Mode only** — launch from the KDE application menu. Gaming Mode / Non-Steam shortcuts are not used.

## Features

- **Xbox Controller** — sticks, triggers, face buttons, D-pad, grips
- **Keyboard & Mouse** — on-screen trackpad + full soft keyboard (TapBoard-style); physical Deck trackpad works too
- **Seamless switch** — UI tabs, or **Select+Start** on the Deck (no re-pair)
- **Battery service** — exposes Deck battery to the host
- **Paired targets** — remembers hosts you have connected to

## Requirements

- Steam Deck / SteamOS (or any Linux x86_64 with BlueZ)
- Bluetooth enabled
- **Desktop Mode** for install and daily use

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

3. Open **DeckLink BT** from the application menu → advertise → pair on the host.

### Uninstall

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

On the host: Bluetooth → Forget **DeckLink BT**. Remove any leftover Steam Non-Steam shortcut from older versions.

### Build from source on Deck

```bash
git clone https://github.com/bastianjosekottekudy-cmyk/DeckLink-BT.git
cd DeckLink-BT
cargo build --release -p decklink-app
./scripts/install-deck.sh
```

## Publishing releases (maintainers)

There is **no GitHub Actions CI/release workflow**. Build locally (WSL on Windows) and replace assets on the current version tag:

```bash
python scripts/publish_release.py
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

## Releases

- **Publish**: `python scripts/publish_release.py` — builds Linux x86_64 tarball in WSL (or native Linux), uploads to `v{version}`.
- Do **not** bump version unless the user asks (`--bump`).
- Do **not** add GitHub Actions for releases.

## Pairing (host)

1. Start DeckLink BT in Desktop Mode (auto-advertises from the app menu entry).
2. On Windows / macOS / Android: add Bluetooth device → **DeckLink BT**.
3. Confirm it appears as a game controller and/or keyboard+mouse as needed.

## Profiles

| Profile | Behavior |
|---------|----------|
| Xbox Controller | Standard gamepad HID |
| Keyboard & Mouse | On-screen trackpad + full soft keyboard (TapBoard-style); Deck trackpad also works |

Config lives in `~/.config/decklink-bt/config.json`.

## Build from source

```bash
# On Linux / Steam Deck
sudo pacman -S --needed rust base-devel dbus pkgconf bluez bluez-libs  # SteamOS/Arch-like
cargo build --release -p decklink-app
./target/release/decklink-bt --advertise
```

Headless (no UI, for debugging):

```bash
./target/release/decklink-bt --headless --advertise
```

## Architecture

```
Deck controls → evdev → profile mapper → HID reports → BlueZ HOGP GATT → host BLE stack
```

Crates: `decklink-hid`, `decklink-input`, `decklink-bt`, `decklink-profiles`, `decklink-ui`, `decklink-app`.

## Troubleshooting

| Issue | Fix |
|-------|-----|
| Permission denied on `/dev/input` | Re-run install script (udev rules) or add user to `input` group |
| Advertise fails | Ensure Bluetooth is on; close other BLE peripherals using the adapter |
| Host does not see gamepad | Forget device, re-advertise, pair again; confirm HOGP/HID over GATT |
| Host pairs but no input | Confirm advertising/connected status in the UI; try Select+Start to switch profile |
| Advertise fails / Desktop connect broken | Update to latest release; check UI status; `bluetoothctl power on` |
| Stuck after uninstall/reinstall | Host: Forget DeckLink BT; Deck: `bluetoothctl power off && bluetoothctl power on` |
| High latency | Keep Deck close to host; some hosts ignore 7.5 ms interval requests |

## License

MIT OR Apache-2.0

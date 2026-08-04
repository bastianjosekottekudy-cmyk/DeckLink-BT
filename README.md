# DeckLink BT

**Steam Deck as a universal, driverless Bluetooth gamepad.**

DeckLink BT turns your Steam Deck into a BLE HID gamepad (HOGP). The host PC, phone, tablet, or console sees a standard Bluetooth gamepad — **no host drivers or companion apps**.

## Features

- **Gamepad mode** — Xbox-style mapping (sticks, triggers, face buttons, D-pad, grips)
- **Desktop & Media** — right trackpad mouse, triggers as clicks, media keys
- **Flight Sim** — precision stick / throttle-oriented mapping
- **Racing (Gyro)** — yaw gyro steers; triggers throttle/brake
- **Battery service** — exposes Deck battery to the host
- **Paired targets** — remembers hosts you have connected to
- **Gaming Mode** — register as a Non-Steam game via the install script

## Requirements

- Steam Deck / SteamOS (or any Linux x86_64 with BlueZ)
- Bluetooth enabled
- Desktop Mode once for install (udev + Flatpak / binary)

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

### Uninstall

```bash
# From an extracted release folder, or from a git clone:
bash scripts/uninstall-deck.sh
```

Or paste this in Konsole (Desktop Mode) to wipe everything without the script:

```bash
pkill -x decklink-bt 2>/dev/null || true
rm -rf ~/.local/share/decklink-bt ~/.config/decklink-bt
rm -f ~/.local/bin/decklink-bt ~/.local/share/applications/decklink-bt.desktop
rm -rf ~/homebrew/plugins/decklink_bt
# udev (SteamOS may ask to unlock read-only):
sudo steamos-readonly disable 2>/dev/null || true
sudo rm -f /etc/udev/rules.d/99-decklink-bt.rules
sudo udevadm control --reload-rules; sudo udevadm trigger
sudo steamos-readonly enable 2>/dev/null || true
```

Then in Steam: remove the **DeckLink BT** Non-Steam shortcut. On the host: Bluetooth → Forget **DeckLink BT**.

3. Follow the **Gaming Mode checklist** below (Steam Input must be disabled).

### Build from source on Deck

```bash
git clone https://github.com/bastianjosekottekudy-cmyk/DeckLink-BT.git
cd DeckLink-BT
cargo build --release -p decklink-app
./scripts/install-deck.sh
```

## Gaming Mode checklist (required)

Steam Gaming Mode often looks “connected once in Desktop but dead in Game Mode” unless you do this:

1. Non-Steam shortcut should point at `~/.local/share/decklink-bt/launch.sh` (installer creates it; always passes `--advertise`).
2. In Steam: **DeckLink BT → gear → Properties → Controller → Disable Steam Input**.  
   If Steam Input stays on, Deck controls never reach DeckLink.
3. Launch **DeckLink BT** from Gaming Mode — wait until status shows Advertising ON.
4. On the **host**: connect to **DeckLink BT**. If an old Desktop Mode bond is stuck, **Forget** it on the host and pair again while Gaming Mode is advertising.
5. Disconnect Deck Bluetooth headphones/audio while testing (Steam can hold the radio).

## Publishing releases (maintainers)

There is **no tag-triggered release workflow**. After commits are pushed to `main`, CI builds the Linux tarball artifact. Publish/replace the current version’s GitHub Release with:

```bash
python scripts/publish_latest_release.py
```

- Reuses `Cargo.toml` version (e.g. `1.0.0` → release `v1.0.0`) and **replaces** assets.
- Bump version **only** when asked explicitly:

```bash
python scripts/publish_latest_release.py --bump patch   # or minor / major
```

## Pairing (host)

1. Start advertising on the Deck.
2. On Windows / macOS / Android: add Bluetooth device → **DeckLink BT**.
3. Confirm it appears as a game controller (Windows: *Set up USB game controllers*).

## Profiles

| Profile | Behavior |
|---------|----------|
| Gamepad (Xbox) | Standard gamepad HID |
| Desktop & Media | Trackpad mouse + media keys |
| Flight Sim | Sticks/triggers tuned for flight |
| Racing (Gyro) | Gyro yaw → steer axis |

Config lives in `~/.config/decklink-bt/config.json`.

## CI / Releases

- **CI** (`.github/workflows/ci.yml`): on every `main` push, tests + builds `decklink-bt-linux-x86_64.tar.gz` as an Actions artifact.
- **Publish**: run `python scripts/publish_latest_release.py` after push — replaces assets on the current `v{version}` release. Do **not** bump version unless the user asks (`--bump`).

## Build from source

```bash
# On Linux / Steam Deck
sudo pacman -S --needed rust base-devel dbus pkgconf bluez bluez-libs  # SteamOS/Arch-like
cargo build --release -p decklink-app
./target/release/decklink-bt
```

Headless (no UI):

```bash
./target/release/decklink-bt --headless --advertise --profile gamepad
```

## Architecture

```
Deck controls → evdev → profile mapper → HID reports → BlueZ HOGP GATT → host BLE stack
```

Crates: `decklink-hid`, `decklink-input`, `decklink-bt`, `decklink-profiles`, `decklink-ui`, `decklink-app`.

## Decky Loader

A thin plugin under [`packaging/decky`](packaging/decky) launches the installed Flatpak/binary from Game Mode. Copy `packaging/decky/decklink_bt` into `~/homebrew/plugins/`.

## Troubleshooting

| Issue | Fix |
|-------|-----|
| Permission denied on `/dev/input` | Re-run install script (udev rules) or add user to `input` group |
| Advertise fails | Ensure Bluetooth is on; close other BLE peripherals using the adapter |
| Host does not see gamepad | Forget device, re-advertise, pair again; confirm HOGP/HID over GATT |
| Steam Input conflicts | Properties → Controller → **Disable Steam Input** for DeckLink BT |
| Works in Desktop, not Gaming Mode | Use `launch.sh` wrapper; disable Steam Input; forget+re-pair on host |
| Host pairs but no input | Steam Input still enabled, or advertising stopped when leaving Desktop Mode |
| Advertise fails in Gaming Mode | Disconnect Deck Bluetooth headphones; ensure BlueZ/`bluetoothctl power on` |
| High latency | Keep Deck close to host; some hosts ignore 7.5 ms interval requests |

## License

MIT OR Apache-2.0

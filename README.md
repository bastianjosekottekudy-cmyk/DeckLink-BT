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

### Option A — one-liner from GitHub Releases

```bash
curl -fsSL https://raw.githubusercontent.com/bastianjosekottekudy-cmyk/DeckLink-BT/main/scripts/install-deck.sh | bash
```

### Option B — manual

1. Download the latest `.tar.gz` or `.flatpak` from [Releases](https://github.com/bastianjosekottekudy-cmyk/DeckLink-BT/releases).
2. Run:

```bash
chmod +x install-deck.sh
./install-deck.sh /path/to/decklink-bt-*.tar.gz
```

3. Return to **Gaming Mode**, launch **DeckLink BT**, tap **Start Advertising**, then pair from the host Bluetooth settings.

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
| Steam Input conflicts | Launch DeckLink BT as Non-Steam game; disable Steam Input for it if needed |
| High latency | Keep Deck close to host; some hosts ignore 7.5 ms interval requests |

## License

MIT OR Apache-2.0

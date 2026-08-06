# Changelog

## 1.0.0 — 2026-08-06

### Changed
- **Wi‑Fi only** — removed BlueZ / BLE HOGP stack
- Deck streams HID over UDP (`decklink-net`) to Windows `decklink-host`
- **No PC IP** — Deck auto-discovers the host on LAN
- **Xbox mode** — sticks/triggers go to ViGEm only (no WASD on sticks)
- **Keyboard & Mouse** — Steam trackpads drive host mouse; soft keyboard for keys

### Added
- `decklink-net` — DLNK UDP protocol (port 31415) + Discover/Announce
- `decklink-host` — Windows tray app (eframe + system tray)
- **ViGEmBus auto-install** — bundled MSI (or download); elevated silent install on first run
- Release zip includes `ViGEmBusSetup_x64.msi` next to `decklink-host.exe`

### Kept
- Deck input capture, profiles (Xbox / Keyboard+Mouse), Slint UI, install scripts

# Changelog

## 1.0.0 — 2026-08-06

### Changed
- **Wi‑Fi only** — removed BlueZ / BLE HOGP stack (`decklink-bt`)
- Deck streams HID over UDP (`decklink-net`) to Windows `decklink-host` (ViGEm Xbox 360 + SendInput mouse/keyboard)
- Physical Steam Deck trackpads stay on the Deck; host mouse uses soft UI only

### Added
- `decklink-net` — DLNK UDP protocol (port 31415)
- `decklink-host` — Windows companion (requires ViGEmBus)
- Connect UI: PC LAN IP + Connect / Disconnect

### Kept
- Deck input capture, profiles (Xbox / Keyboard+Mouse), Slint UI, install scripts

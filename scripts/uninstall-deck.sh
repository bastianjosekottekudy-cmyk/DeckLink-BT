#!/usr/bin/env bash
# DeckLink BT — full uninstall (Steam Deck / SteamOS)
# Usage: bash scripts/uninstall-deck.sh
set -euo pipefail

echo "==> Uninstalling DeckLink BT"

STEAMOS_RO_TOUCHED=0
is_steamos() {
  [[ -f /etc/os-release ]] && grep -qiE 'steamos|holo' /etc/os-release
}
steamos_rw_begin() {
  if is_steamos && command -v steamos-readonly >/dev/null 2>&1; then
    if steamos-readonly status 2>/dev/null | grep -qi 'enabled\|read-only'; then
      echo "==> Temporarily disabling SteamOS read-only root…"
      sudo steamos-readonly disable
      STEAMOS_RO_TOUCHED=1
    fi
  fi
}
steamos_rw_end() {
  if [[ "$STEAMOS_RO_TOUCHED" -eq 1 ]]; then
    echo "==> Re-enabling SteamOS read-only root…"
    sudo steamos-readonly enable || true
    STEAMOS_RO_TOUCHED=0
  fi
}
trap steamos_rw_end EXIT

# Stop running process if any
pkill -x decklink-bt 2>/dev/null || true
pkill -f '[d]ecklink-bt' 2>/dev/null || true

# App files
rm -rf "${HOME}/.local/share/decklink-bt"
rm -f "${HOME}/.local/bin/decklink-bt"
rm -f "${HOME}/.local/share/applications/decklink-bt.desktop"

# Config
rm -rf "${HOME}/.config/decklink-bt"

# Flatpak (if ever installed)
if command -v flatpak >/dev/null 2>&1; then
  flatpak uninstall -y --user io.github.bastianjosekottekudy_cmyk.DeckLinkBT 2>/dev/null || true
fi

# Decky plugin (if copied)
rm -rf "${HOME}/homebrew/plugins/decklink_bt"

# udev rule
RULE="/etc/udev/rules.d/99-decklink-bt.rules"
if [[ -f "$RULE" ]]; then
  echo "==> Removing udev rule (sudo)…"
  steamos_rw_begin
  sudo rm -f "$RULE"
  sudo udevadm control --reload-rules || true
  sudo udevadm trigger || true
  steamos_rw_end
fi

# Optional: downloaded release leftovers in common places
rm -f "${HOME}/Downloads/decklink-bt-linux-x86_64.tar.gz" 2>/dev/null || true
rm -rf "${HOME}/Downloads/decklink-bt-linux-x86_64" 2>/dev/null || true

echo
echo "Removed app files, config, desktop entry, udev rule, and Flatpak/Decky copies if present."
echo
echo "Manual (Steam):"
echo "  Gaming Mode or Desktop Steam → Library → Non-Steam → DeckLink BT → Remove"
echo
echo "Manual (host PC / phone):"
echo "  Bluetooth settings → Forget / remove 'DeckLink BT'"
echo
echo "Done."

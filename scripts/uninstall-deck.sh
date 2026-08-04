#!/usr/bin/env bash
# DeckLink BT — remove every prior installation artifact (Steam Deck / SteamOS)
# Usage: bash scripts/uninstall-deck.sh
set -euo pipefail

echo "==> Uninstalling DeckLink BT (full purge)"

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

# Stop running process if any (exact name only — broad -f matches this script argv)
pkill -x decklink-bt 2>/dev/null || true
sleep 0.3 || true

# --- Core install locations --------------------------------------------------
rm -rf "${HOME}/.local/share/decklink-bt"
rm -f "${HOME}/.local/bin/decklink-bt"
rm -rf "${HOME}/.config/decklink-bt"
rm -rf "${HOME}/.cache/decklink-bt"

# App menu + Desktop shortcuts (current + legacy names)
rm -f "${HOME}/.local/share/applications/decklink-bt.desktop"
rm -f "${HOME}/.local/share/applications/DeckLink BT.desktop"
rm -f "${HOME}/.local/share/applications/DeckLink-BT.desktop"
rm -f "${HOME}/Desktop/decklink-bt.desktop"
rm -f "${HOME}/Desktop/DeckLink BT.desktop"
rm -f "${HOME}/Desktop/DeckLink-BT.desktop"
rm -f "${HOME}/Desktop/DeckLink BT"
if command -v xdg-user-dir >/dev/null 2>&1; then
  XDG_DESKTOP="$(xdg-user-dir DESKTOP 2>/dev/null || true)"
  if [[ -n "$XDG_DESKTOP" && -d "$XDG_DESKTOP" ]]; then
    rm -f "${XDG_DESKTOP}/decklink-bt.desktop" \
      "${XDG_DESKTOP}/DeckLink BT.desktop" \
      "${XDG_DESKTOP}/DeckLink-BT.desktop" \
      "${XDG_DESKTOP}/DeckLink BT" 2>/dev/null || true
  fi
fi

# Any leftover .desktop mentioning DeckLink under common places
find "${HOME}/.local/share/applications" "${HOME}/Desktop" \
  -maxdepth 1 -type f \( -iname '*decklink*' -o -iname '*DeckLink*' \) \
  -delete 2>/dev/null || true

# Legacy Gaming Mode / Non-Steam launcher names (if share dir was recreated)
rm -f "${HOME}/.local/share/decklink-bt/launch.sh" 2>/dev/null || true
rm -f "${HOME}/.local/share/decklink-bt/DeckLink BT" 2>/dev/null || true
rm -f "${HOME}/.local/share/decklink-bt/DeckLink BT.desktop" 2>/dev/null || true

# Flatpak (if ever installed)
if command -v flatpak >/dev/null 2>&1; then
  flatpak uninstall -y --user io.github.bastianjosekottekudy_cmyk.DeckLinkBT 2>/dev/null || true
  flatpak uninstall -y --system io.github.bastianjosekottekudy_cmyk.DeckLinkBT 2>/dev/null || true
fi

# Decky plugin (if copied)
rm -rf "${HOME}/homebrew/plugins/decklink_bt"
rm -rf "${HOME}/homebrew/plugins/DeckLinkBT" 2>/dev/null || true

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

# Downloaded / extracted release leftovers in common places
rm -f "${HOME}/Downloads/decklink-bt-linux-x86_64.tar.gz" 2>/dev/null || true
rm -rf "${HOME}/Downloads/decklink-bt-linux-x86_64" 2>/dev/null || true
rm -f /tmp/decklink-bt-latest.tar.gz 2>/dev/null || true
rm -rf /tmp/decklink-bt-linux-x86_64 2>/dev/null || true

# Logs that may live outside share dir
rm -f "${HOME}/.local/state/decklink-bt/decklink.log" 2>/dev/null || true
rm -rf "${HOME}/.local/state/decklink-bt" 2>/dev/null || true

# Refresh desktop database
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "${HOME}/.local/share/applications" 2>/dev/null || true
fi

echo
echo "Purged:"
echo "  ~/.local/share/decklink-bt"
echo "  ~/.local/bin/decklink-bt"
echo "  ~/.config/decklink-bt"
echo "  app menu + Desktop shortcuts"
echo "  udev rule, Flatpak/Decky copies, download leftovers (if present)"
echo
echo "Still manual if present:"
echo "  Steam Library → remove old Non-Steam 'DeckLink BT' / launch.sh"
echo "  Host Bluetooth → Forget 'DeckLink BT'"
echo
echo "Done."

#!/usr/bin/env bash
# DeckLink BT — Steam Deck / SteamOS installer
set -euo pipefail

APP_ID="io.github.bastianjosekottekudy_cmyk.DeckLinkBT"
REPO="bastianjosekottekudy-cmyk/DeckLink-BT"
INSTALL_DIR="${HOME}/.local/share/decklink-bt"
BIN_DIR="${HOME}/.local/bin"
UDEV_RULE_SRC="$(cd "$(dirname "$0")/.." && pwd)/packaging/udev/99-decklink-bt.rules"
FLATPAK_ID="$APP_ID"

echo "==> DeckLink BT installer"

mkdir -p "$INSTALL_DIR" "$BIN_DIR"

ARTIFACT="${1:-}"
if [[ -z "$ARTIFACT" ]]; then
  echo "==> Fetching latest release asset…"
  if ! command -v gh >/dev/null 2>&1 && ! command -v curl >/dev/null 2>&1; then
    echo "Need curl or gh to download releases." >&2
    exit 1
  fi
  API="https://api.github.com/repos/${REPO}/releases/latest"
  URL=$(curl -fsSL "$API" | grep -oE 'https://[^"]+decklink-bt-linux-x86_64[^"]+\.tar\.gz' | head -n1 || true)
  if [[ -z "$URL" ]]; then
    echo "Could not find tar.gz asset. Pass a local path: $0 /path/to/decklink-bt-*.tar.gz" >&2
    exit 1
  fi
  ARTIFACT="/tmp/decklink-bt-latest.tar.gz"
  curl -fsSL "$URL" -o "$ARTIFACT"
fi

if [[ "$ARTIFACT" == *.flatpak ]]; then
  echo "==> Installing Flatpak…"
  flatpak install --user -y --noninteractive "$ARTIFACT" || flatpak install --user -y "$ARTIFACT"
  LAUNCH="flatpak run ${FLATPAK_ID}"
elif [[ "$ARTIFACT" == *.tar.gz ]]; then
  echo "==> Extracting tarball to ${INSTALL_DIR}…"
  tar -xzf "$ARTIFACT" -C "$INSTALL_DIR" --strip-components=1 2>/dev/null || tar -xzf "$ARTIFACT" -C "$INSTALL_DIR"
  BIN="$(find "$INSTALL_DIR" -type f -name decklink-bt | head -n1)"
  if [[ -z "$BIN" ]]; then
    echo "decklink-bt binary not found in archive" >&2
    exit 1
  fi
  chmod +x "$BIN"
  ln -sfn "$BIN" "${BIN_DIR}/decklink-bt"
  LAUNCH="${BIN_DIR}/decklink-bt"
else
  echo "Unsupported artifact: $ARTIFACT" >&2
  exit 1
fi

# udev rules (needs sudo once)
RULE_DST="/etc/udev/rules.d/99-decklink-bt.rules"
if [[ -f "$UDEV_RULE_SRC" ]]; then
  echo "==> Installing udev rules (sudo)…"
  sudo cp "$UDEV_RULE_SRC" "$RULE_DST"
  sudo udevadm control --reload-rules || true
  sudo udevadm trigger || true
else
  echo "==> Writing embedded udev rules…"
  sudo tee "$RULE_DST" >/dev/null <<'EOF'
# DeckLink BT — allow input access for decklink user sessions
KERNEL=="event*", SUBSYSTEM=="input", MODE="0660", GROUP="input", TAG+="uaccess"
KERNEL=="js*", SUBSYSTEM=="input", MODE="0660", GROUP="input", TAG+="uaccess"
EOF
  sudo udevadm control --reload-rules || true
  sudo udevadm trigger || true
fi

# Ensure user in input group when possible
if command -v usermod >/dev/null 2>&1; then
  sudo usermod -aG input "$USER" || true
fi

# Desktop entry
APPS="${HOME}/.local/share/applications"
mkdir -p "$APPS"
cat > "${APPS}/decklink-bt.desktop" <<EOF
[Desktop Entry]
Name=DeckLink BT
Comment=Steam Deck as a BLE gamepad
Exec=${LAUNCH}
Icon=input-gaming
Terminal=false
Type=Application
Categories=Game;Utility;
EOF

# Steam non-Steam shortcut helper
STEAM_SCRIPT="$(cd "$(dirname "$0")/.." && pwd)/packaging/steam/add-nonsteam-shortcut.sh"
if [[ -f "$STEAM_SCRIPT" ]]; then
  bash "$STEAM_SCRIPT" "$LAUNCH" || true
else
  echo "==> Add Non-Steam game manually: ${LAUNCH}"
fi

echo
echo "Done. Launch from Desktop or Gaming Mode (Non-Steam game 'DeckLink BT')."
echo "Then: Start Advertising → pair from host Bluetooth settings."
echo "Binary/Flatpak launch: ${LAUNCH}"

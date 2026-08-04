#!/usr/bin/env bash
# DeckLink BT — Steam Deck / SteamOS installer
# Usage:
#   bash scripts/install-deck.sh ./decklink-bt-linux-x86_64.tar.gz
#   bash scripts/install-deck.sh          # download latest release
set -euo pipefail

APP_ID="io.github.bastianjosekottekudy_cmyk.DeckLinkBT"
REPO="bastianjosekottekudy-cmyk/DeckLink-BT"
INSTALL_DIR="${HOME}/.local/share/decklink-bt"
BIN_DIR="${HOME}/.local/bin"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
UDEV_RULE_SRC="${ROOT_DIR}/packaging/udev/99-decklink-bt.rules"
FLATPAK_ID="$APP_ID"
STEAMOS_RO_TOUCHED=0

echo "==> DeckLink BT installer"

mkdir -p "$INSTALL_DIR" "$BIN_DIR"

# --- SteamOS read-only root helpers ------------------------------------------
is_steamos() {
  [[ -f /etc/os-release ]] && grep -qiE 'steamos|holo' /etc/os-release
}

steamos_rw_begin() {
  if is_steamos && command -v steamos-readonly >/dev/null 2>&1; then
    if steamos-readonly status 2>/dev/null | grep -qi 'enabled\|read-only'; then
      echo "==> Temporarily disabling SteamOS read-only root (needs sudo)…"
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

cleanup() {
  steamos_rw_end
}
trap cleanup EXIT

# --- Artifact ----------------------------------------------------------------
ARTIFACT="${1:-}"
if [[ -n "$ARTIFACT" && ! -f "$ARTIFACT" ]]; then
  echo "ERROR: artifact not found: $ARTIFACT" >&2
  echo "Pass the .tar.gz path, e.g.:" >&2
  echo "  bash scripts/install-deck.sh ./decklink-bt-linux-x86_64.tar.gz" >&2
  exit 1
fi

if [[ -z "$ARTIFACT" ]]; then
  echo "==> Fetching latest release asset…"
  if ! command -v curl >/dev/null 2>&1; then
    echo "Need curl to download releases." >&2
    exit 1
  fi
  API="https://api.github.com/repos/${REPO}/releases/latest"
  URL=$(curl -fsSL "$API" | grep -oE 'https://[^"]+decklink-bt-linux-x86_64[^"]+\.tar\.gz' | head -n1 || true)
  if [[ -z "$URL" ]]; then
    echo "Could not find tar.gz asset. Pass a local path:" >&2
    echo "  bash $0 /path/to/decklink-bt-linux-x86_64.tar.gz" >&2
    exit 1
  fi
  ARTIFACT="/tmp/decklink-bt-latest.tar.gz"
  curl -fsSL "$URL" -o "$ARTIFACT"
fi

# Resolve to absolute path (tar cwd changes later)
ARTIFACT="$(cd "$(dirname "$ARTIFACT")" && pwd)/$(basename "$ARTIFACT")"

if [[ "$ARTIFACT" == *.flatpak ]]; then
  echo "==> Installing Flatpak…"
  flatpak install --user -y --noninteractive "$ARTIFACT" || flatpak install --user -y "$ARTIFACT"
  LAUNCH="flatpak run ${FLATPAK_ID}"
elif [[ "$ARTIFACT" == *.tar.gz ]]; then
  echo "==> Extracting tarball to ${INSTALL_DIR}…"
  # Clear previous install contents (keep dir)
  find "$INSTALL_DIR" -mindepth 1 -maxdepth 1 -exec rm -rf {} + 2>/dev/null || true
  tar -xzf "$ARTIFACT" -C "$INSTALL_DIR"

  # Tarball may contain a single top-level folder
  if [[ ! -f "${INSTALL_DIR}/decklink-bt" ]]; then
    INNER="$(find "$INSTALL_DIR" -mindepth 1 -maxdepth 1 -type d | head -n1 || true)"
    if [[ -n "$INNER" && -f "${INNER}/decklink-bt" ]]; then
      # Flatten: move children up
      shopt -s dotglob nullglob
      mv "$INNER"/* "$INSTALL_DIR"/
      shopt -u dotglob nullglob
      rmdir "$INNER" 2>/dev/null || rm -rf "$INNER"
    fi
  fi

  BIN="$(find "$INSTALL_DIR" -type f -name decklink-bt | head -n1 || true)"
  if [[ -z "$BIN" ]]; then
    echo "decklink-bt binary not found in archive" >&2
    ls -la "$INSTALL_DIR" >&2 || true
    exit 1
  fi

  # Fix execute bits lost by zip/copy/FAT downloads
  chmod +x "$BIN" || true
  find "$INSTALL_DIR" -type f \( -name '*.sh' -o -name decklink-bt \) -exec chmod +x {} + 2>/dev/null || true

  # Ensure binary is actually executable by this user
  if [[ ! -x "$BIN" ]]; then
    echo "ERROR: cannot mark binary executable: $BIN" >&2
    ls -la "$BIN" >&2 || true
    exit 1
  fi

  mkdir -p "$BIN_DIR"
  ln -sfn "$BIN" "${BIN_DIR}/decklink-bt"
  LAUNCH="${BIN_DIR}/decklink-bt"

  # Prefer packaging/udev from the extracted tree when present
  if [[ -f "${INSTALL_DIR}/packaging/udev/99-decklink-bt.rules" ]]; then
    UDEV_RULE_SRC="${INSTALL_DIR}/packaging/udev/99-decklink-bt.rules"
  fi
  if [[ -f "${INSTALL_DIR}/packaging/steam/add-nonsteam-shortcut.sh" ]]; then
    STEAM_SCRIPT="${INSTALL_DIR}/packaging/steam/add-nonsteam-shortcut.sh"
  fi
else
  echo "Unsupported artifact: $ARTIFACT" >&2
  exit 1
fi

# --- udev (one-time; needs sudo; SteamOS may be read-only) --------------------
RULE_DST="/etc/udev/rules.d/99-decklink-bt.rules"
echo "==> Installing udev rules (sudo; may prompt for password)…"
steamos_rw_begin
if [[ -f "$UDEV_RULE_SRC" ]]; then
  sudo cp "$UDEV_RULE_SRC" "$RULE_DST"
else
  sudo tee "$RULE_DST" >/dev/null <<'EOF'
# DeckLink BT — allow input access for decklink user sessions
KERNEL=="event*", SUBSYSTEM=="input", MODE="0660", GROUP="input", TAG+="uaccess"
KERNEL=="js*", SUBSYSTEM=="input", MODE="0660", GROUP="input", TAG+="uaccess"
KERNEL=="hidraw*", MODE="0660", GROUP="input", TAG+="uaccess"
EOF
fi
sudo udevadm control --reload-rules || true
sudo udevadm trigger || true

if command -v usermod >/dev/null 2>&1; then
  sudo usermod -aG input "$USER" || true
fi
steamos_rw_end

# --- Desktop entry -----------------------------------------------------------
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

# Ensure launch wrapper exists / is refreshed for Gaming Mode
if [[ -f "${INSTALL_DIR}/packaging/steam/add-nonsteam-shortcut.sh" ]]; then
  bash "${INSTALL_DIR}/packaging/steam/add-nonsteam-shortcut.sh" "$LAUNCH" || true
elif [[ -f "${ROOT_DIR}/packaging/steam/add-nonsteam-shortcut.sh" ]]; then
  bash "${ROOT_DIR}/packaging/steam/add-nonsteam-shortcut.sh" "$LAUNCH" || true
fi

echo
echo "Done."
echo "  Launch (Desktop UI): ${LAUNCH}"
echo "  Launch (Gaming Mode Steam target): ${HOME}/.local/share/decklink-bt/DeckLink BT"
echo "Remove any Steam shortcut still named launch.sh, then use 'DeckLink BT'."
echo "Then: Start Advertising / auto-advertise → pair from host Bluetooth settings."
echo
echo "If sudo/udev failed: open Konsole and re-run with a password prompt available."
echo "If you still see Permission denied on the script itself, run:"
echo "  bash scripts/install-deck.sh ./decklink-bt-linux-x86_64.tar.gz"

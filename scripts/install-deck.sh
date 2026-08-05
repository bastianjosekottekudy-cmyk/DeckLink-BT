#!/usr/bin/env bash
# DeckLink BT — Steam Deck / SteamOS installer (Desktop Mode)
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
DESKTOP_FILE_NAME="decklink-bt.desktop"

echo "==> DeckLink BT installer"
# Stop any running copy so the new binary can replace grabs/locks.
# Do NOT pkill -f 'decklink-bt' — that matches this installer's argv
# (./decklink-bt-linux-….tar.gz) and SIGTERMs the script (exit 143).
pkill -x decklink-bt 2>/dev/null || true
if [[ -x "${HOME}/.local/bin/decklink-bt" ]]; then
  pkill -f "${HOME}/.local/bin/decklink-bt" 2>/dev/null || true
fi
if [[ -x "${HOME}/.local/share/decklink-bt/decklink-bt" ]]; then
  pkill -f "${HOME}/.local/share/decklink-bt/decklink-bt" 2>/dev/null || true
fi
sleep 0.5
# Rotate old log so a fresh session is obvious.
if [[ -f "${HOME}/.local/share/decklink-bt/decklink.log" ]]; then
  mv -f "${HOME}/.local/share/decklink-bt/decklink.log" \
    "${HOME}/.local/share/decklink-bt/decklink.log.bak" 2>/dev/null || true
fi

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

# Allow DeckLink to toggle kernel lizard mode without a password prompt.
SUDOERS_DST="/etc/sudoers.d/decklink-bt-lizard"
sudo tee "$SUDOERS_DST" >/dev/null <<EOF
# DeckLink BT — non-interactive lizard_mode toggle
${USER} ALL=(root) NOPASSWD: /usr/bin/tee /sys/module/hid_steam/parameters/lizard_mode
EOF
sudo chmod 440 "$SUDOERS_DST" || true
if [[ -e /sys/module/hid_steam/parameters/lizard_mode ]]; then
  echo N | sudo tee /sys/module/hid_steam/parameters/lizard_mode >/dev/null || true
  echo "==> hid_steam lizard_mode set to N (Desktop stick-mouse off at kernel)"
fi
steamos_rw_end

if [[ -x "$LAUNCH" ]]; then
  echo "==> Installed binary: $LAUNCH"
  ls -la "$LAUNCH" || true
  sha256sum "$LAUNCH" 2>/dev/null || shasum -a 256 "$LAUNCH" 2>/dev/null || true
fi

# --- Desktop entry (app menu + Desktop shortcut) -----------------------------
write_desktop_file() {
  local dest="$1"
  cat > "$dest" <<EOF
[Desktop Entry]
Version=1.0
Type=Application
Name=DeckLink BT
Comment=Steam Deck as a BLE gamepad / keyboard+mouse
Exec=env WINIT_UNIX_BACKEND=x11 SLINT_BACKEND=winit ${LAUNCH} --advertise
Icon=input-gaming
Terminal=false
Categories=Game;Utility;
StartupNotify=true
EOF
  chmod +x "$dest" || true
  # KDE/Plasma: mark as trusted so double-click works without a prompt
  if command -v gio >/dev/null 2>&1; then
    gio set "$dest" metadata::trusted true 2>/dev/null || true
  fi
  if command -v dbus-launch >/dev/null 2>&1; then
    true
  fi
}

APPS="${HOME}/.local/share/applications"
mkdir -p "$APPS"
write_desktop_file "${APPS}/${DESKTOP_FILE_NAME}"

# Visible Desktop shortcut (Steam Deck Desktop Mode)
DESKTOP_DIR="${HOME}/Desktop"
if [[ ! -d "$DESKTOP_DIR" ]]; then
  DESKTOP_DIR="$(xdg-user-dir DESKTOP 2>/dev/null || true)"
fi
if [[ -n "${DESKTOP_DIR:-}" && -d "$DESKTOP_DIR" ]]; then
  write_desktop_file "${DESKTOP_DIR}/${DESKTOP_FILE_NAME}"
  echo "==> Desktop shortcut: ${DESKTOP_DIR}/${DESKTOP_FILE_NAME}"
else
  mkdir -p "${HOME}/Desktop"
  write_desktop_file "${HOME}/Desktop/${DESKTOP_FILE_NAME}"
  echo "==> Desktop shortcut: ${HOME}/Desktop/${DESKTOP_FILE_NAME}"
fi

# Refresh app menu cache when available
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APPS" 2>/dev/null || true
fi

# Remove legacy Gaming Mode / Non-Steam launchers from older installs
rm -f "${INSTALL_DIR}/launch.sh" \
  "${INSTALL_DIR}/DeckLink BT" \
  "${INSTALL_DIR}/DeckLink BT.desktop" 2>/dev/null || true

echo
echo "Done. Desktop Mode only."
echo "  App menu: DeckLink BT"
echo "  Desktop shortcut: ~/Desktop/${DESKTOP_FILE_NAME}"
echo "  Binary: ${LAUNCH}"
if [[ -x "$LAUNCH" ]]; then
  ls -la "$LAUNCH" | awk '{print "  binary:", $0}'
fi
echo "  After open: status bar must say Ready v1.0.4 — if not, install failed."
echo "  On PC: Forget ALL steamdeck + DeckLink BT bonds, then pair DeckLink BT only."
echo
echo "If sudo/udev failed: open Konsole and re-run with a password prompt available."
echo "If Permission denied on the script: bash scripts/install-deck.sh ./decklink-bt-linux-x86_64.tar.gz"

#!/usr/bin/env bash
# Best-effort Non-Steam shortcut registration for Steam on SteamOS.
set -euo pipefail

LAUNCH_BIN="${1:-${HOME}/.local/bin/decklink-bt}"
NAME="DeckLink BT"
STEAM_DIR="${HOME}/.steam/steam"
USERDATA="${STEAM_DIR}/userdata"
WRAPPER="${HOME}/.local/share/decklink-bt/launch.sh"
DESKTOP="${HOME}/.local/share/applications/decklink-bt.desktop"

echo "==> Registering Non-Steam shortcut: ${NAME} -> ${LAUNCH_BIN}"

mkdir -p "$(dirname "$WRAPPER")"

# Gaming Mode wrapper:
# - Always start BLE advertising (no need to click UI under gamescope)
# - Avoid Steam Runtime interfering with system BlueZ / D-Bus
# - Hint SDL not to exclusive-grab in ways that break other apps
cat > "$WRAPPER" <<EOF
#!/usr/bin/env bash
set -euo pipefail
export DECKLINK_GAMING_MODE=1
# Prefer system libs / bluetoothd over Steam runtime isolation when possible
unset LD_LIBRARY_PATH || true
export SDL_VIDEODRIVER="\${SDL_VIDEODRIVER:-wayland}"
exec "${LAUNCH_BIN}" --advertise "\$@"
EOF
chmod +x "$WRAPPER"

# steamos-add-to-steam when present (adds Non-Steam entry)
if command -v steamos-add-to-steam >/dev/null 2>&1; then
  steamos-add-to-steam "$WRAPPER" || steamos-add-to-steam "$LAUNCH_BIN" || true
fi

# Desktop entry for Desktop Mode + Discoverability
mkdir -p "$(dirname "$DESKTOP")"
cat > "$DESKTOP" <<EOF
[Desktop Entry]
Name=${NAME}
Comment=Steam Deck as a BLE gamepad (auto-advertises)
Exec=${WRAPPER}
Icon=input-gaming
Terminal=false
Type=Application
Categories=Game;Utility;
EOF

if [[ ! -d "$USERDATA" ]]; then
  echo "Steam userdata not found yet."
fi

echo
echo "IMPORTANT for Gaming Mode:"
echo "  1. Steam → Library → ${NAME} (Non-Steam) → gear → Properties → Controller"
echo "     → Disable Steam Input  (required or Deck controls never reach DeckLink)"
echo "  2. Launch ${NAME} from Gaming Mode — advertising starts automatically."
echo "  3. On the host: pair/connect to 'DeckLink BT' (forget+re-pair if Desktop Mode"
echo "     bond is stuck)."
echo
echo "Wrapper path to add manually if needed:"
echo "  ${WRAPPER}"

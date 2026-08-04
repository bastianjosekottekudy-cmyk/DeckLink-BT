#!/usr/bin/env bash
# Register DeckLink BT as a Non-Steam game with the correct display name.
set -euo pipefail

LAUNCH_BIN="${1:-${HOME}/.local/bin/decklink-bt}"
NAME="DeckLink BT"
SHARE="${HOME}/.local/share/decklink-bt"
# Steam uses the file basename as the default shortcut title — do NOT use launch.sh
STEAM_LAUNCHER="${SHARE}/DeckLink BT"
LOG="${SHARE}/decklink.log"
DESKTOP_STEAM="${SHARE}/DeckLink BT.desktop"
DESKTOP_UI="${HOME}/.local/share/applications/decklink-bt.desktop"

echo "==> Registering Non-Steam shortcut: ${NAME}"

mkdir -p "$SHARE"

# Gaming Mode launcher binary Steam will show as "DeckLink BT"
cat > "$STEAM_LAUNCHER" <<EOF
#!/usr/bin/env bash
set -euo pipefail
export DECKLINK_GAMING_MODE=1
mkdir -p "${SHARE}"
unset LD_LIBRARY_PATH || true
unset LD_PRELOAD || true
bluetoothctl power on >/dev/null 2>&1 || true
echo "\$(date -Iseconds) starting DeckLink BT (gaming/headless)" >> "${LOG}"
exec "${LAUNCH_BIN}" --gaming >> "${LOG}" 2>&1
EOF
chmod +x "$STEAM_LAUNCHER"

# Keep old launch.sh as a symlink for anyone who already pointed Steam at it
ln -sfn "$STEAM_LAUNCHER" "${SHARE}/launch.sh"

# .desktop used by steamos-add-to-steam (Name= becomes library title)
cat > "$DESKTOP_STEAM" <<EOF
[Desktop Entry]
Name=${NAME}
Comment=Steam Deck as a BLE gamepad
Exec="${STEAM_LAUNCHER}"
Icon=input-gaming
Terminal=false
Type=Application
Categories=Game;
EOF

# Desktop Mode app menu entry (UI)
mkdir -p "$(dirname "$DESKTOP_UI")"
cat > "$DESKTOP_UI" <<EOF
[Desktop Entry]
Name=${NAME}
Comment=Steam Deck as a BLE gamepad
Exec=env -u DECKLINK_GAMING_MODE WINIT_UNIX_BACKEND=x11 SLINT_BACKEND=winit ${LAUNCH_BIN} --advertise
Icon=input-gaming
Terminal=false
Type=Application
Categories=Game;Utility;
EOF

added=0
if command -v steamos-add-to-steam >/dev/null 2>&1; then
  # Prefer .desktop so Steam picks up Name=DeckLink BT
  if steamos-add-to-steam "$DESKTOP_STEAM" 2>/dev/null; then
    added=1
  elif steamos-add-to-steam "$STEAM_LAUNCHER" 2>/dev/null; then
    added=1
  fi
fi

echo
if [[ "$added" -eq 1 ]]; then
  echo "Added to Steam as '${NAME}'."
else
  echo "Add Non-Steam game manually:"
  echo "  Steam → Games → Add a Non-Steam Game → Browse →"
  echo "  ${STEAM_LAUNCHER}"
  echo "  Then set the shortcut name to: ${NAME}"
fi
echo
echo "Gaming Mode checklist:"
echo "  1. Remove any old shortcut named 'launch.sh' from Steam"
echo "  2. Use shortcut '${NAME}' → Properties → Controller → Disable Steam Input"
echo "  3. Launch '${NAME}', then pair/connect on the host"
echo "  4. Logs:  cat ${LOG}"
echo
echo "Desktop Mode UI: application menu '${NAME}' or:  ${LAUNCH_BIN} --advertise"

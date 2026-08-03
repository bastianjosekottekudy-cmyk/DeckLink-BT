#!/usr/bin/env bash
# Best-effort Non-Steam shortcut registration for Steam on SteamOS.
set -euo pipefail

LAUNCH="${1:-decklink-bt}"
NAME="DeckLink BT"
STEAM_DIR="${HOME}/.steam/steam"
USERDATA="${STEAM_DIR}/userdata"

echo "==> Registering Non-Steam shortcut: ${NAME} -> ${LAUNCH}"

# Prefer steamos-add-to-steam when present
if command -v steamos-add-to-steam >/dev/null 2>&1; then
  steamos-add-to-steam "$LAUNCH" || true
fi

# Create a small wrapper desktop file Steam can import
WRAPPER="${HOME}/.local/share/decklink-bt/launch.sh"
mkdir -p "$(dirname "$WRAPPER")"
cat > "$WRAPPER" <<EOF
#!/usr/bin/env bash
exec ${LAUNCH} "\$@"
EOF
chmod +x "$WRAPPER"

# Shortcuts.vdf patching is brittle; document manual fallback
if [[ ! -d "$USERDATA" ]]; then
  echo "Steam userdata not found. In Desktop Mode: Steam → Add Non-Steam Game → ${WRAPPER}"
  exit 0
fi

echo "If the shortcut does not appear, use Steam → Games → Add a Non-Steam Game → browse to:"
echo "  ${WRAPPER}"
echo "Then set name to '${NAME}' and return to Gaming Mode."

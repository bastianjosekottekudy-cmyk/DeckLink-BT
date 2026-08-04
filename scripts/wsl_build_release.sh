#!/usr/bin/env bash
set -euo pipefail
export PATH="$HOME/.cargo/bin:$HOME/.local/zig:$PATH"
TMPAPT="$HOME/.local/apt"
SYS="$HOME/decklink-sysroot"
ROOT="$(wslpath -a 'C:/Users/USER/Projects/DeckLink-BT')"

mkdir -p "$TMPAPT/lists/partial" "$TMPAPT/cache/archives/partial" "$TMPAPT/etc/apt/preferences.d"
if [[ ! -f "$TMPAPT/etc/apt/sources.list" ]]; then
  cp /etc/apt/sources.list "$TMPAPT/etc/apt/" 2>/dev/null || true
  cp -a /etc/apt/sources.list.d "$TMPAPT/etc/apt/" 2>/dev/null || true
fi

APT_OPTS=(-o "Dir::State=$TMPAPT" -o "Dir::Cache=$TMPAPT/cache" -o "Dir::Etc=$TMPAPT/etc/apt")
apt-get "${APT_OPTS[@]}" update -qq

rm -rf /tmp/debs
mkdir -p /tmp/debs "$SYS"
cd /tmp/debs
apt-get "${APT_OPTS[@]}" download \
  gcc g++ cpp binutils \
  gcc-13 g++-13 cpp-13 \
  gcc-13-x86-64-linux-gnu g++-13-x86-64-linux-gnu cpp-13-x86-64-linux-gnu \
  binutils-x86-64-linux-gnu \
  libc6-dev linux-libc-dev \
  libgcc-13-dev libstdc++-13-dev libgcc-s1 \
  libc6 \
  pkg-config pkgconf pkgconf-bin libpkgconf3 \
  libdbus-1-dev libdbus-1-3 \
  libudev-dev libudev1 \
  libxkbcommon-dev libxkbcommon0 \
  libwayland-dev libwayland-client0 libwayland-cursor0 libwayland-egl1 libwayland-server0 \
  libegl-dev libgl-dev libgles-dev libglvnd-dev \
  libx11-dev libx11-6 libxcursor-dev libxcursor1 \
  libxi-dev libxi6 libxrandr-dev libxrandr2 \
  libfontconfig-dev libfontconfig1 libfreetype-dev libfreetype6 \
  libxext-dev libxext6 libxrender-dev libxrender1 \
  libxcb1-dev libxcb1 libxau-dev libxdmcp-dev \
  libffi-dev libexpat1-dev zlib1g-dev \
  libpng-dev libpng16-16t64 \
  libbrotli-dev libbrotli1

echo "COUNT=$(ls -1 *.deb | wc -l)"
for deb in *.deb; do
  dpkg-deb -x "$deb" "$SYS"
done

# Use Zig as the C linker (self-contained). Keep extracted .deb libs for
# pkg-config / -ldbus-1 etc., but do not use the sysroot gcc (it hardcodes /usr paths).
mkdir -p "$HOME/bin"
cat > "$HOME/bin/zig-cc" <<'EOF'
#!/bin/sh
exec "$HOME/.local/zig/zig" cc "$@"
EOF
cat > "$HOME/bin/zig-c++" <<'EOF'
#!/bin/sh
exec "$HOME/.local/zig/zig" c++ "$@"
EOF
chmod +x "$HOME/bin/zig-cc" "$HOME/bin/zig-c++"
export CC="$HOME/bin/zig-cc"
export CXX="$HOME/bin/zig-c++"
ln -sfn "$CC" "$HOME/bin/cc"
ln -sfn "$CXX" "$HOME/bin/c++"

export PATH="$HOME/bin:$SYS/usr/bin:$PATH"
export PKG_CONFIG="$SYS/usr/bin/pkg-config"
export PKG_CONFIG_PATH="$SYS/usr/lib/x86_64-linux-gnu/pkgconfig:$SYS/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="$SYS"
export LIBRARY_PATH="$SYS/usr/lib/x86_64-linux-gnu:$SYS/lib/x86_64-linux-gnu:${LIBRARY_PATH:-}"
export LD_LIBRARY_PATH="$SYS/usr/lib/x86_64-linux-gnu:$SYS/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH:-}"
export CPATH="$SYS/usr/include:${CPATH:-}"
export C_INCLUDE_PATH="$SYS/usr/include:${C_INCLUDE_PATH:-}"
export CPLUS_INCLUDE_PATH="$SYS/usr/include:${CPLUS_INCLUDE_PATH:-}"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$CC"
export PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1
export PKG_CONFIG_ALLOW_SYSTEM_LIBS=1

echo "CC=$CC PKG_CONFIG=$PKG_CONFIG"
"$CC" --version | head -1 || true
"$PKG_CONFIG" --exists fontconfig && echo fontconfig_ok || echo fontconfig_missing
"$PKG_CONFIG" --exists dbus-1 && echo dbus_ok || echo dbus_missing
cd "$ROOT"
cargo test -p decklink-hid -p decklink-profiles
cargo build --release -p decklink-app

STAGE="dist/decklink-bt-linux-x86_64"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp target/release/decklink-bt "$STAGE/"
cp README.md LICENSE LICENSE-MIT LICENSE-APACHE "$STAGE/" 2>/dev/null || cp README.md LICENSE "$STAGE/"
cp -r scripts packaging "$STAGE/"
chmod +x "$STAGE/decklink-bt"
find "$STAGE/scripts" "$STAGE/packaging" -type f -name '*.sh' -exec chmod +x {} +
mkdir -p dist
tar -czf dist/decklink-bt-linux-x86_64.tar.gz -C dist decklink-bt-linux-x86_64
ls -la dist/decklink-bt-linux-x86_64.tar.gz
echo STATUS=build_ok

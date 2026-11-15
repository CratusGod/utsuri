#!/usr/bin/env bash
# Install Utsuri for this user (no root): the binary in ~/.local/bin, its icon and app entry,
# and its right-click entry (Dolphin) / "Open with" entry. Re-run to update.
#
#   packaging/linux/install.sh [--profile release|fast] [--autostart] [--no-build]
#
# Utsuri does NOT start at login unless asked (--autostart): its Shift trigger would fight
# another Shift+drag menu (pick Ctrl or Alt in its Settings first).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROFILE=release
BUILD=1
AUTOSTART=0
while [ $# -gt 0 ]; do
  case "$1" in
    --profile) PROFILE="$2"; shift 2 ;;
    --no-build) BUILD=0; shift ;;
    --autostart) AUTOSTART=1; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

BIN="$HOME/.local/bin"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
ICONS="$DATA/icons/hicolor/128x128/apps"
APPS="$DATA/applications"

need() { command -v "$1" >/dev/null || { echo "missing: $1 ($2)" >&2; MISSING=1; }; }
MISSING=0
need cargo "sudo pacman -S rust"
need pdftoppm "sudo pacman -S poppler"
need heif-dec "sudo pacman -S libheif"
need ffmpeg "sudo pacman -S ffmpeg (video/audio)"
need tesseract "sudo pacman -S tesseract tesseract-data-eng (OCR)"
[ "$MISSING" = 0 ] || echo "(the app runs without the optional tools; its features say what is missing)"

if [ "$BUILD" = 1 ]; then
  echo "== building ($PROFILE)"
  (cd "$ROOT" && cargo build --profile "$PROFILE" -p utsuri)
fi
OUT="$ROOT/target/$PROFILE"
[ -x "$OUT/utsuri" ] || { echo "build first: $OUT/utsuri missing" >&2; exit 1; }

# a running copy keeps its old file open; install by rename so it is never half-written.
# The build keeps line tables for crash logs (hundreds of MB on Linux, inside the binary);
# the installed copy drops them and keeps the symbol names.
install_bin() {
  if command -v strip >/dev/null; then
    strip --strip-debug -o "$BIN/.$1.new" "$OUT/$1"
    chmod 755 "$BIN/.$1.new"
  else
    install -m 755 "$OUT/$1" "$BIN/.$1.new"
  fi
  mv -f "$BIN/.$1.new" "$BIN/$1"
}
refresh() {
  command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$DATA/icons/hicolor" 2>/dev/null || true
}

echo "== installing"
mkdir -p "$BIN" "$ICONS" "$APPS"
install_bin utsuri
install -m 644 "$ROOT/crates/utsuri/assets/utsuri.png" "$ICONS/dev.cratusgod.utsuri.png"
# Utsuri's own app entry, Dolphin service menu and "Open with" (written by the app itself)
"$BIN/utsuri" --register-shell
refresh

if [ "$AUTOSTART" = 1 ]; then
  mkdir -p "$CONFIG/autostart"
  cat > "$CONFIG/autostart/dev.cratusgod.utsuri.desktop" <<DESK
[Desktop Entry]
Type=Application
Name=Utsuri
Exec=$BIN/utsuri --quiet
Icon=dev.cratusgod.utsuri
Terminal=false
X-GNOME-Autostart-enabled=true
DESK
fi

echo "== done: $BIN/utsuri"
id -nG | tr ' ' '\n' | grep -qx input || echo "  The trigger needs the input group: sudo usermod -aG input \$USER, then sign out and in."
case ":$PATH:" in *":$BIN:"*) ;; *) echo "  add $BIN to PATH" ;; esac

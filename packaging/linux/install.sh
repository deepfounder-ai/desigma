#!/usr/bin/env sh
# Installs desigma for the current user. No sudo needed.
#
#   ./install.sh            install under ~/.local
#   PREFIX=/some/dir ./install.sh   install elsewhere (bin/ and share/ below it)
#
# To remove: delete ~/.local/bin/desigma, ~/.local/bin/desigma-diff, ~/.local/bin/desigma-mcp,
# ~/.local/share/applications/desigma.desktop, the desigma.png files under
# ~/.local/share/icons/hicolor/*/apps/ and ~/.local/share/mime/packages/desigma.xml.
#
# DESIGMA_RESOURCES_ONLY=1 skips the binaries: the in-app updater has already swapped them in
# (atomically, while desigma was running) and only needs the desktop entry, icons and file types refreshed.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
prefix=${PREFIX:-$HOME/.local}

install -d "$prefix/bin" "$prefix/share/applications" "$prefix/share/mime/packages"
if [ -z "${DESIGMA_RESOURCES_ONLY:-}" ]; then
  install -m 755 "$here/desigma" "$prefix/bin/desigma"
  [ -f "$here/desigma-diff" ] && install -m 755 "$here/desigma-diff" "$prefix/bin/desigma-diff"
  [ -f "$here/desigma-mcp" ] && install -m 755 "$here/desigma-mcp" "$prefix/bin/desigma-mcp"
fi
# Point Exec at the installed binary so launchers work even when ~/.local/bin is not on their PATH.
sed "s|^Exec=desigma|Exec=$prefix/bin/desigma|" "$here/desigma.desktop" > "$prefix/share/applications/desigma.desktop"
chmod 644 "$prefix/share/applications/desigma.desktop"
[ -f "$here/desigma-mime.xml" ] && install -m 644 "$here/desigma-mime.xml" "$prefix/share/mime/packages/desigma.xml"

for png in "$here"/icons/*.png; do
  size=$(basename "$png" .png)   # 16, 32, ... 512
  d="$prefix/share/icons/hicolor/${size}x${size}/apps"
  install -d "$d"
  install -m 644 "$png" "$d/desigma.png"
done

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" 2>/dev/null || true
command -v update-mime-database >/dev/null 2>&1 && update-mime-database "$prefix/share/mime" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" 2>/dev/null || true

echo "desigma installed to $prefix"
case ":$PATH:" in
  *":$prefix/bin:"*) ;;
  *) echo "note: $prefix/bin is not on your PATH; add it to run 'desigma' from a terminal." ;;
esac

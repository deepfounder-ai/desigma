#!/usr/bin/env bash
# Build the Windows installer (Inno Setup) next to the portable zip.
#
#   scripts/package-windows-installer.sh <version> [bin-dir]
#
# Expects desigma.exe, desigma-diff.exe and desigma-mcp.exe built in release mode (bin-dir defaults to
# target/x86_64-pc-windows-msvc/release, then target/release). Needs `iscc` (Inno Setup 6) on PATH
# or installed in a default location. Writes into dist/:
#   desigma-<version>-x86_64-windows-setup.exe
#   <that file>.sha256     (CI merges these into SHA256SUMS)
# The installer's file version must be numeric, so 0.1.0-alpha.9 becomes 0.1.0.9 (the full string
# stays the displayed AppVersion); a non-numeric suffix such as 0.0.0-dev.abc1234 becomes 0.0.0.0.
set -euo pipefail

version=${1:?usage: package-windows-installer.sh <version> [bin-dir]}
version=${version#v}
bin_dir=${2:-target/x86_64-pc-windows-msvc/release}
[ -d "$bin_dir" ] || bin_dir=target/release

core=${version%%[-+]*}
suffix=${version#"$core"}
build=$(printf '%s' "$suffix" | grep -oE '[0-9]+$' || true)
case "$build" in ''|*[!0-9]*) build=0 ;; esac
[ "${#build}" -le 5 ] || build=0   # each version field is 16-bit
numeric=$core.$build
printf '%s' "$numeric" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' || numeric=0.0.0.0

iscc=$(command -v iscc || command -v ISCC || true)
if [ -z "$iscc" ]; then
  for d in "/c/Program Files (x86)/Inno Setup 6" "/c/Program Files/Inno Setup 6" "${LOCALAPPDATA:-}/Programs/Inno Setup 6"; do
    if [ -x "$d/ISCC.exe" ]; then iscc="$d/ISCC.exe"; break; fi
  done
fi
[ -n "$iscc" ] || { echo "package-windows-installer.sh: iscc (Inno Setup 6) not found" >&2; exit 1; }

root=$(pwd)
bin_win=$(cd "$bin_dir" && { pwd -W 2>/dev/null || pwd; })
mkdir -p dist
# MSYS_NO_PATHCONV stops Git Bash from rewriting the /D switches into paths.
MSYS_NO_PATHCONV=1 "$iscc" "/DAppVersion=$version" "/DNumericVersion=$numeric" "/DBinDir=$bin_win" /Q "$(cygpath -w "$root/scripts/windows/desigma.iss")"

out=desigma-$version-x86_64-windows-setup.exe
cd "$root/dist"
if command -v sha256sum >/dev/null 2>&1; then sha256sum "$out" > "$out.sha256"; else shasum -a 256 "$out" > "$out.sha256"; fi
echo "packaged dist/$out (file version $numeric)"
cat "$out.sha256"

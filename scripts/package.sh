#!/usr/bin/env bash
# Package the release binaries for one target.
#
#   scripts/package.sh <target-triple> <version> [bin-dir]
#
# Expects `desigma`, `desigma-diff` and `desigma-mcp` already built in release mode. bin-dir
# defaults to target/<triple>/release, falling back to target/release (a plain
# `cargo build --release` without --target). Writes into dist/:
#   desigma-<version>-<triple>.zip     (Windows) or .tar.gz (Linux)
#   <archive>.sha256                   (checksum line; CI merges these into SHA256SUMS)
# macOS is packaged by scripts/package-macos.sh (universal .app, .dmg and .tar.gz).
# The Windows exe carries its icon (embedded by crates/desigma-app/build.rs); the Linux
# tarball adds desigma.desktop, icons/<size>.png and install.sh.
set -euo pipefail

target=${1:?usage: package.sh <target-triple> <version> [bin-dir]}
version=${2:?usage: package.sh <target-triple> <version> [bin-dir]}
version=${version#v}
bin_dir=${3:-target/$target/release}
[ -d "$bin_dir" ] || bin_dir=target/release

case "$target" in
  *windows*) exe=.exe; fmt=zip ;;
  *apple-darwin*) echo "package.sh: use scripts/package-macos.sh for macOS" >&2; exit 2 ;;
  *) exe=; fmt=tar.gz ;;
esac

name=desigma-$version-$target
root=$(pwd)
stage=$root/dist/$name
rm -rf "$stage"
mkdir -p "$stage"

for bin in desigma desigma-diff desigma-mcp; do
  cp "$bin_dir/$bin$exe" "$stage/"
done
cp LICENSE-MIT LICENSE-APACHE README.md "$stage/"
mkdir -p "$stage/licenses"
cp "$root/crates/desigma-fonts/assets/inter/OFL.txt" "$stage/licenses/Inter-OFL.txt"
cp "$root/crates/desigma-app/assets/icons/lucide/LICENSE" "$stage/licenses/Lucide-LICENSE.txt"

case "$target" in
  *linux*)
    mkdir -p "$stage/icons"
    for size in 16 32 48 64 128 256 512; do
      cp "$root/assets/icon/png/icon_$size.png" "$stage/icons/$size.png"
    done
    cp "$root/packaging/linux/desigma.desktop" "$root/packaging/linux/desigma-mime.xml" "$stage/"
    cp "$root/packaging/linux/install.sh" "$stage/install.sh"
    chmod +x "$stage/install.sh" "$stage/desigma" "$stage/desigma-diff" "$stage/desigma-mcp"
    ;;
esac

cd "$root/dist"
if [ "$fmt" = zip ]; then
  archive=$name.zip
  rm -f "$archive"
  if command -v zip >/dev/null 2>&1; then
    zip -qr "$archive" "$name"
  elif command -v 7z >/dev/null 2>&1; then
    7z a -tzip -bso0 "$archive" "$name"
  else
    python3 -m zipfile -c "$archive" "$name" 2>/dev/null || python -m zipfile -c "$archive" "$name"
  fi
else
  archive=$name.tar.gz
  tar -czf "$archive" "$name"
fi
rm -rf "$stage"

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$archive" > "$archive.sha256"
else
  shasum -a 256 "$archive" > "$archive.sha256"
fi

echo "packaged dist/$archive"
cat "$archive.sha256"

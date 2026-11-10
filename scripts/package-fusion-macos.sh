#!/usr/bin/env bash
# Package fusion-desigma for macOS: rebrands the upstream desigma macOS artifacts
# without touching any upstream file.
#
#   scripts/package-fusion-macos.sh <version> [arm64-bin-dir] [x86_64-bin-dir]
#
# Runs the upstream scripts/package-macos.sh (which stages, signs and writes
# dist/desigma-<version>-macos.dmg and dist/desigma-<version>-macos-universal.tar.gz),
# then rewrites the bundle identity in each artifact:
#   - bundle directory renamed desigma.app -> fusion-desigma.app
#   - CFBundleName / CFBundleDisplayName -> fusion-desigma (CFBundleIdentifier and
#     CFBundleExecutable stay upstream: the identifier is pinned by upstream tests,
#     the executable name is what the upstream scripts and CLI expect)
#   - artifacts renamed to fusion-desigma-<version>-*, checksums regenerated
#
# All upstream-named artifacts are removed afterwards, so dist/ carries only the
# fusion-branded output.
set -euo pipefail

version=${1:?usage: package-fusion-macos.sh <version> [arm64-bin-dir] [x86_64-bin-dir]}
version=${version#v}
shift || true

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
dist=$root/dist

log() { echo "package-fusion-macos.sh: $*"; }

# 1. Upstream packaging, unmodified.
"$root/scripts/package-macos.sh" "$version" "$@"

upstream_dmg="$dist/desigma-$version-macos.dmg"
upstream_tarball="$dist/desigma-$version-macos-universal.tar.gz"
fusion_dmg="$dist/fusion-desigma-$version-macos.dmg"
fusion_tarball="$dist/fusion-desigma-$version-macos-universal.tar.gz"

rebrand_app() {
    # Renames $1/desigma.app -> $1/fusion-desigma.app and applies the fusion display name.
    # Separate `local` statements: in bash 3.2 (macOS / CI runners) a single
    # declaration line expands $dir before the first assignment lands (set -u).
    local dir="$1"
    local app="$dir/desigma.app"
    mv "$app" "$dir/fusion-desigma.app"
    app="$dir/fusion-desigma.app"
    plutil -replace CFBundleName -string fusion-desigma "$app/Contents/Info.plist"
    plutil -replace CFBundleDisplayName -string fusion-desigma "$app/Contents/Info.plist"
    plutil -lint "$app/Contents/Info.plist"
    codesign --force --deep --sign - "$app"
    codesign --verify --deep --strict "$app"
}

# 2. DMG: attach, rebrand in place, repack.
log "rebranding $upstream_dmg"
mount=$(mktemp -d)
hdiutil attach -nobrowse -readonly -mountpoint "$mount" "$upstream_dmg" >/dev/null
rebrand_app "$mount"
rm -f "$fusion_dmg"
hdiutil create -volname fusion-desigma -srcfolder "$mount" -ov -format UDZO "$fusion_dmg" >/dev/null
hdiutil detach "$mount" >/dev/null
rm -f "$upstream_dmg"

# 3. tar.gz: extract, rebrand, repack.
log "rebranding $upstream_tarball"
work=$(mktemp -d)
COPYFILE_DISABLE=1 tar -C "$work" -xzf "$upstream_tarball"
rebrand_app "$work/desigma-$version-macos-universal"
rm -f "$fusion_tarball"
COPYFILE_DISABLE=1 tar -C "$work" -czf "$fusion_tarball" "desigma-$version-macos-universal"
rm -rf "$work" "$upstream_tarball"

# 4. Checksums for the fusion-named artifacts only.
rm -f "$dist/desigma-$version-macos.dmg.sha256" "$dist/desigma-$version-macos-universal.tar.gz.sha256"
for f in "$fusion_dmg" "$fusion_tarball"; do
    (cd "$dist" && shasum -a 256 "$(basename "$f")" > "$(basename "$f").sha256")
    cat "$f.sha256"
done

log "packaged dist/$(basename "$fusion_dmg") and dist/$(basename "$fusion_tarball")"

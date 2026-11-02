#!/usr/bin/env bash
# Package desigma for macOS: one universal (arm64 + x86_64) desigma.app, shipped as a .dmg and a .tar.gz.
#
#   scripts/package-macos.sh <version> [arm64-bin-dir] [x86_64-bin-dir]
#
# Run on macOS (needs lipo, codesign, hdiutil; iconutil is used when present). Expects `desigma`,
# `desigma-diff` and `desigma-mcp` already built in release mode for both targets. The bin dirs default to
# target/aarch64-apple-darwin/release and target/x86_64-apple-darwin/release. Writes into dist/:
#   desigma-<version>-macos.dmg              drag-to-Applications disk image (the main download)
#   desigma-<version>-macos-universal.tar.gz desigma.app + licenses + README, for scripting
#   <file>.sha256                            checksum lines (CI merges them into SHA256SUMS)
#
# The app is ad-hoc signed (`codesign --sign -`). That is not a Developer ID signature, so Gatekeeper
# still asks the user to confirm once (Open Anyway), but Apple Silicon no longer reports the app as
# "damaged". Bundle identifier: ai.deepfounder.desigma.
set -euo pipefail

version=${1:?usage: package-macos.sh <version> [arm64-bin-dir] [x86_64-bin-dir]}
version=${version#v}
arm_dir=${2:-target/aarch64-apple-darwin/release}
x64_dir=${3:-target/x86_64-apple-darwin/release}

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
dist=$root/dist
work=$dist/macos-work
rm -rf "$work"
mkdir -p "$work/stage" "$dist"

# CFBundleVersion must be dotted numbers; the tag may carry a pre-release suffix (0.2.0-rc.1, 0.0.0-dev.abc1234).
short_version=$version
bundle_version=${version%%[-+]*}

# 1. Universal binaries.
app=$work/stage/desigma.app
macos_dir=$app/Contents/MacOS
res_dir=$app/Contents/Resources
mkdir -p "$macos_dir" "$res_dir"
for bin in desigma desigma-diff desigma-mcp; do
  for d in "$arm_dir" "$x64_dir"; do
    [ -f "$d/$bin" ] || { echo "package-macos.sh: missing $d/$bin" >&2; exit 1; }
  done
  lipo -create -output "$macos_dir/$bin" "$arm_dir/$bin" "$x64_dir/$bin"
  chmod 755 "$macos_dir/$bin"
  lipo -info "$macos_dir/$bin"
done

# 2. Icon: build the .icns from the committed iconset; fall back to the committed .icns.
iconset=$root/assets/icon/desigma.iconset
if command -v iconutil >/dev/null 2>&1; then
  iconutil -c icns -o "$res_dir/desigma.icns" "$iconset"
else
  echo "package-macos.sh: iconutil not found, using the committed assets/icon/desigma.icns" >&2
  cp "$root/assets/icon/desigma.icns" "$res_dir/desigma.icns"
fi

# 3. Info.plist from the template.
sed -e "s/@SHORT_VERSION@/$short_version/g" -e "s/@BUNDLE_VERSION@/$bundle_version/g" \
  "$root/packaging/macos/Info.plist.in" > "$app/Contents/Info.plist"
if command -v plutil >/dev/null 2>&1; then
  plutil -lint "$app/Contents/Info.plist"
fi
printf 'APPL????' > "$app/Contents/PkgInfo"

# Keep embedded asset notices inside the app, including when installed from the DMG.
mkdir -p "$res_dir/licenses"
cp "$root/crates/desigma-fonts/assets/inter/OFL.txt" "$res_dir/licenses/Inter-OFL.txt"
cp "$root/crates/desigma-app/assets/icons/lucide/LICENSE" "$res_dir/licenses/Lucide-LICENSE.txt"

# 4. Ad-hoc sign. Sign the helper tool first, then the bundle.
codesign --force --sign - "$macos_dir/desigma-diff"
codesign --force --sign - "$macos_dir/desigma-mcp"
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict --verbose=2 "$app"

# 5. Disk image: app, an Applications shortcut and the read-me.
stage=$work/stage
ln -s /Applications "$stage/Applications"
cp "$root/packaging/macos/Read me first.txt" "$stage/Read me first.txt"
dmg=$dist/desigma-$version-macos.dmg
rm -f "$dmg"
# hdiutil occasionally fails with "Resource busy" on CI runners; retry a few times.
for attempt in 1 2 3; do
  if hdiutil create -volname desigma -srcfolder "$stage" -ov -format UDZO "$dmg"; then
    break
  fi
  [ "$attempt" = 3 ] && { echo "package-macos.sh: hdiutil failed" >&2; exit 1; }
  sleep 5
done

# 6. Universal tar.gz for CLI and scripting use.
name=desigma-$version-macos-universal
tar_stage=$work/$name
mkdir -p "$tar_stage"
cp -R "$app" "$tar_stage/desigma.app"
cp LICENSE-MIT LICENSE-APACHE README.md "$tar_stage/"
tarball=$dist/$name.tar.gz
rm -f "$tarball"
COPYFILE_DISABLE=1 tar -C "$work" -czf "$tarball" "$name"

# 7. Checksums.
for f in "$dmg" "$tarball"; do
  (cd "$dist" && shasum -a 256 "$(basename "$f")" > "$(basename "$f").sha256")
  cat "$f.sha256"
done

rm -rf "$work"
echo "packaged dist/$(basename "$dmg") and dist/$(basename "$tarball")"

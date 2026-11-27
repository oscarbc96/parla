#!/usr/bin/env bash
# Wraps a built parla binary in a minimal Parla.app so macOS gives the process a
# bundle identifier (notifications and the dock badge need one; `cargo run`
# has none). Usage: dev-bundle.sh <binary> <out.app>
set -euo pipefail

bin="${1:?usage: dev-bundle.sh <binary> <out.app>}"
app="${2:?usage: dev-bundle.sh <binary> <out.app>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
plist_src="$root/crates/parla/macos/Info.plist"
id="com.justparla.parla"

if [ "$(uname -s)" != Darwin ]; then
  exit 0
fi

mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
# Copy only when changed, so an unchanged build keeps its signature.
if ! cmp -s "$bin" "$app/Contents/MacOS/parla"; then
  cp "$bin" "$app/Contents/MacOS/parla"
fi
# Locales come from i18n/locales.toml, the one list the packager check also reads.
for l in $(awk -F'"' '/^code = /{print $2}' "$root/i18n/locales.toml"); do
  mkdir -p "$app/Contents/Resources/$l.lproj"
  cp "$root/crates/parla/macos/$l.lproj/InfoPlist.strings" "$app/Contents/Resources/$l.lproj/"
done
# App icon: an .icns from the release PNGs (assets/icons/app), rebuilt only
# when a source image is newer. Not committed. dev-sign.sh runs after this.
icns="$app/Contents/Resources/AppIcon.icns"
icons="$root/assets/icons/app"
if [ ! -f "$icns" ] || [ -n "$(find "$icons" -name '*.png' -newer "$icns" -print -quit)" ]; then
  set_dir="$(mktemp -d)/AppIcon.iconset"
  mkdir -p "$set_dir"
  # iconset name -> source png and pixel size; sips fills in missing sizes.
  while read -r name px; do
    src="$icons/icon-$px.png"
    if [ -f "$src" ]; then
      cp "$src" "$set_dir/$name.png"
    else
      sips -z "$px" "$px" "$icons/icon-512.png" --out "$set_dir/$name.png" >/dev/null
    fi
  done <<'SIZES'
icon_16x16 16
icon_16x16@2x 32
icon_32x32 32
icon_32x32@2x 64
icon_128x128 128
icon_128x128@2x 256
icon_256x256 256
icon_256x256@2x 512
icon_512x512 512
SIZES
  # 1024 px for icon_512x512@2x is upscaled from the 512 source.
  sips -z 1024 1024 "$icons/icon-512.png" --out "$set_dir/icon_512x512@2x.png" >/dev/null
  iconutil -c icns "$set_dir" -o "$icns"
  rm -rf "$(dirname "$set_dir")"
fi
plist="$app/Contents/Info.plist"
cp "$plist_src" "$plist"
pb=/usr/libexec/PlistBuddy
"$pb" -c "Add :CFBundleIdentifier string $id" "$plist"
"$pb" -c "Add :CFBundleName string Parla" "$plist"
"$pb" -c "Add :CFBundleExecutable string parla" "$plist"
"$pb" -c "Add :CFBundlePackageType string APPL" "$plist"
"$pb" -c "Add :CFBundleVersion string 0" "$plist"
"$pb" -c "Add :CFBundleShortVersionString string 0.0.0-dev" "$plist"
"$pb" -c "Add :LSMinimumSystemVersion string 14.0" "$plist"
"$pb" -c "Add :CFBundleIconFile string AppIcon" "$plist"

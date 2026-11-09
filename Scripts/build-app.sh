#!/bin/bash
# Builds a self-contained build/Loudible.app for Apple Silicon: ad-hoc signed, or with a Developer
# ID and the hardened runtime that notarization needs when SIGNING_IDENTITY names one.
#
#   Scripts/build-app.sh
#   APP_VERSION=1.2.0 Scripts/build-app.sh
#   SIGNING_IDENTITY="Developer ID Application: … (TEAMID)" Scripts/build-app.sh
#
# Run Scripts/verify-app.sh afterwards to check the bundle.
set -euo pipefail
cd "$(dirname "$0")/.."
Scripts/build-ffmpeg.sh
source Scripts/ffmpeg-version.sh
FFMPEG_PREFIX="$PWD/Vendor/ffmpeg"
# actool comes with Xcode, not the Command Line Tools.
if [ -z "${DEVELOPER_DIR:-}" ] && [ -d /Applications/Xcode.app/Contents/Developer ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi

APP_NAME="Loudible"
APP="build/$APP_NAME.app"
VERSION="${APP_VERSION:-0.1.0}"
VERSION="${VERSION%%-*}"
case "$VERSION" in
    *[!0-9.]*|.*|*.|*..*|"") echo "error: '$VERSION' is not a version like 1.2.0" >&2; exit 1 ;;
esac

MACOSX_DEPLOYMENT_TARGET="$FFMPEG_MINIMUM_MACOS" cargo build --release --locked --bin skia-ui

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"
cp target/release/skia-ui "$APP/Contents/MacOS/$APP_NAME"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key><string>en</string>
	<key>CFBundleDisplayName</key><string>$APP_NAME</string>
	<key>CFBundleExecutable</key><string>$APP_NAME</string>
	<key>CFBundleIconFile</key><string>AppIcon</string>
	<key>CFBundleIconName</key><string>AppIcon</string>
	<key>CFBundleIdentifier</key><string>local.loudible.Loudible</string>
	<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
	<key>CFBundleName</key><string>$APP_NAME</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>$VERSION</string>
	<key>CFBundleVersion</key><string>$VERSION</string>
	<key>LSArchitecturePriority</key><array><string>arm64</string></array>
	<key>LSMinimumSystemVersion</key><string>$FFMPEG_MINIMUM_MACOS</string>
	<key>NSHighResolutionCapable</key><true/>
	<key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

echo "Compiling the icon…"
PARTIAL_PLIST="$(mktemp -t loudible-icon)"
xcrun actool Resources/AppIcon.icon --compile "$APP/Contents/Resources" --platform macosx \
    --minimum-deployment-target "$FFMPEG_MINIMUM_MACOS" --app-icon AppIcon \
    --output-partial-info-plist "$PARTIAL_PLIST" > /dev/null
rm -f "$PARTIAL_PLIST"
# The Dock icon for `cargo run`, which has no bundle; commit it when the icon changes.
sips -s format png -Z 512 "$APP/Contents/Resources/AppIcon.icns" --out Resources/AppIcon.png > /dev/null

echo "Bundling libraries…"
Scripts/bundle_dylibs.py "$APP/Contents/MacOS/$APP_NAME" "$APP/Contents/Frameworks" > build/bundled-libraries.tsv
echo "  $(wc -l < build/bundled-libraries.tsv | tr -d ' ') libraries"

echo "Writing third-party notices…"
Scripts/write-notices.py "$FFMPEG_PREFIX" "$APP/Contents/Resources" "bundled in Contents/Frameworks"

if [ -n "${SIGNING_IDENTITY:-}" ]; then
    echo "Signing as ${SIGNING_IDENTITY}…"
    SIGN_FLAGS=(--sign "$SIGNING_IDENTITY" --options runtime --timestamp)
else
    echo "Signing (ad-hoc)…"
    SIGN_FLAGS=(--sign - --timestamp=none)
fi
for dylib in "$APP/Contents/Frameworks/"*.dylib; do
    codesign --force "${SIGN_FLAGS[@]}" "$dylib"
done
codesign --force "${SIGN_FLAGS[@]}" "$APP"

echo "Built $APP (minimum macOS $FFMPEG_MINIMUM_MACOS)"

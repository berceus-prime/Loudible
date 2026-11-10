#!/bin/bash
# Packs build/Loudible.app into build/Loudible.dmg, with an Applications link to drag it onto.
#
#   Scripts/make-dmg.sh
#
# With SIGNING_IDENTITY, also signs the disk image. With NOTARY_APPLE_ID, NOTARY_PASSWORD (an
# app-specific password) and NOTARY_TEAM_ID too, notarizes it and staples the ticket, so
# Gatekeeper opens it offline. Build the app with the same SIGNING_IDENTITY first.
set -euo pipefail
cd "$(dirname "$0")/.."
APP="build/Loudible.app"
DMG="build/Loudible.dmg"
STAGING="build/dmg"
[ -d "$APP" ] || { echo "error: $APP is missing; run Scripts/build-app.sh first" >&2; exit 1; }

rm -rf "$STAGING" "$DMG"
mkdir -p "$STAGING"
cp -R "$APP" "$STAGING/"
ln -s /Applications "$STAGING/Applications"
hdiutil create -volname Loudible -srcfolder "$STAGING" -format UDZO -ov "$DMG" > /dev/null
rm -rf "$STAGING"

if [ -z "${SIGNING_IDENTITY:-}" ]; then
    echo "Built $DMG (unsigned: Gatekeeper will block it on other Macs)"
    exit 0
fi
codesign --force --sign "$SIGNING_IDENTITY" --timestamp "$DMG"

if [ -z "${NOTARY_APPLE_ID:-}" ] || [ -z "${NOTARY_PASSWORD:-}" ] || [ -z "${NOTARY_TEAM_ID:-}" ]; then
    echo "Built $DMG (signed, not notarized)"
    exit 0
fi
# Apple rejects every other kind, such as Apple Development, only after the upload.
if ! codesign -dvv "$APP" 2>&1 | grep -q "^Authority=Developer ID Application:"; then
    echo "error: $APP is not signed with a Developer ID Application certificate" >&2
    exit 1
fi
echo "Notarizing (usually a few minutes)…"
CREDENTIALS=(--apple-id "$NOTARY_APPLE_ID" --password "$NOTARY_PASSWORD" --team-id "$NOTARY_TEAM_ID")
# notarytool exits 0 for a rejected submission too, so read its verdict.
RESULT="$(xcrun notarytool submit "$DMG" --wait --output-format json "${CREDENTIALS[@]}")"
field() { plutil -extract "$1" raw -o - - <<< "$RESULT"; }
if [ "$(field status)" != "Accepted" ]; then
    echo "error: notarization finished as '$(field status)'; Apple's log:" >&2
    xcrun notarytool log "$(field id)" "${CREDENTIALS[@]}" >&2
    exit 1
fi
xcrun stapler staple "$DMG"
spctl --assess --type open --context context:primary-signature -vv "$DMG"
echo "Built $DMG (signed and notarized)"

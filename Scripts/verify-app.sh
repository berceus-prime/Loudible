#!/bin/bash
# Checks build/Loudible.app: ARM64 only, macOS 14 minimum, valid signatures, only the three
# bundled FFmpeg libraries (no Homebrew or other non-system paths), notices, and that the bundled
# FFmpeg loads, is LGPL-licensed, and has every required component.
set -euo pipefail
cd "$(dirname "$0")/.."
source Scripts/ffmpeg-version.sh
APP="${1:-build/Loudible.app}"
EXECUTABLE="$APP/Contents/MacOS/Loudible"
FRAMEWORKS="$APP/Contents/Frameworks"
RESOURCES="$APP/Contents/Resources"
failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

expected="libavcodec.61.dylib libavformat.61.dylib libavutil.59.dylib"
actual="$(ls "$FRAMEWORKS" | sort | tr '\n' ' ' | sed 's/ $//')"
[ "$actual" = "$expected" ] || fail "Frameworks contains '$actual', expected '$expected'"

machos=("$EXECUTABLE" "$FRAMEWORKS"/*.dylib)
echo "Checking ${#machos[@]} Mach-O files…"

for macho in "${machos[@]}"; do
    archs="$(lipo -archs "$macho")"
    [ "$archs" = "arm64" ] || fail "$macho architectures are '$archs', expected arm64"

    # Consume all load commands: early awk exit can SIGPIPE otool under pipefail.
    minos="$(otool -l "$macho" | awk '/LC_BUILD_VERSION/{found=1} found && /minos/ && !printed {print $2; printed=1}')"
    [ "$(printf '%s\n%s\n' "$minos" "$FFMPEG_MINIMUM_MACOS" | sort -V | tail -1)" = "$FFMPEG_MINIMUM_MACOS" ] \
        || fail "$macho requires macOS $minos, newer than $FFMPEG_MINIMUM_MACOS"

    while read -r dependency; do
        case "$dependency" in
            /usr/lib/*|/System/*) ;;
            @rpath/*)
                [ -f "$FRAMEWORKS/${dependency#@rpath/}" ] || fail "$macho needs missing $dependency" ;;
            *) fail "$macho links outside the bundle: $dependency" ;;
        esac
    done < <(otool -L "$macho" | tail -n +2 | awk '{print $1}')

    while read -r rpath; do
        case "$rpath" in
            @executable_path/*|@loader_path*) ;;
            *) fail "$macho has rpath outside the bundle: $rpath" ;;
        esac
    done < <(otool -l "$macho" | awk '/cmd LC_RPATH/{getline; getline; print $2}')
done

plist_minimum="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$APP/Contents/Info.plist")"
[ "$plist_minimum" = "$FFMPEG_MINIMUM_MACOS" ] || fail "LSMinimumSystemVersion is $plist_minimum, expected $FFMPEG_MINIMUM_MACOS"

if codesign --verify --deep --strict "$APP" 2>/dev/null; then
    echo "Signature: valid ($(codesign -dv "$APP" 2>&1 | awk -F= '/^Signature/{print $2}'))"
else
    codesign --verify --deep --strict --verbose=2 "$APP" || true
    fail "code signature is invalid"
fi

for file in THIRD_PARTY_NOTICES.txt Licenses/rust-crates.txt AppIcon.icns \
    Licenses/FFmpeg/COPYING.LGPLv2.1 Licenses/FFmpeg/LICENSE.md; do
    [ -s "$RESOURCES/$file" ] || fail "missing Contents/Resources/$file"
done
grep -q 'FFmpeg' "$RESOURCES/THIRD_PARTY_NOTICES.txt" || fail "FFmpeg notice is missing"
grep -q 'skia-safe' "$RESOURCES/THIRD_PARTY_NOTICES.txt" || fail "crate list is missing"

# Launch the bundled executable headlessly and record every image dyld loads.
log="$(mktemp -t loudible-dyld)"
if DYLD_PRINT_LIBRARIES=1 "$EXECUTABLE" --self-test >"$log" 2>&1; then
    grep -E "self-test|^license|^missing" "$log"
    if grep -E "/opt/homebrew|/usr/local|/Vendor/" "$log"; then
        fail "dyld loaded libraries from outside the bundle"
    fi
    echo "dyld loaded $(grep -c "$FRAMEWORKS" "$log" || true) libraries from Contents/Frameworks"
else
    grep -vE "^dyld" "$log" >&2 || true
    fail "self-test failed (FFmpeg did not load, is not LGPL, or lacks components)"
fi
rm -f "$log"

if [ "$failures" -gt 0 ]; then
    echo "$failures check(s) failed" >&2
    exit 1
fi
echo "All packaging checks passed."

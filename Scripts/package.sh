#!/bin/bash
# Builds a portable folder of the app on Linux or Windows (MSYS2 shell), in build/.
# macOS uses Scripts/build-app.sh instead.
set -euo pipefail
cd "$(dirname "$0")/.."
Scripts/build-ffmpeg.sh
FFMPEG_PREFIX="$PWD/Vendor/ffmpeg"

case "$(uname -s)" in
    Linux)
        # The app finds its FFmpeg next to itself, wherever the folder is moved.
        export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,\$ORIGIN/lib"
        OUT="build/Loudible-linux"
        ;;
    MINGW*|MSYS*)
        OUT="build/Loudible-windows"
        ;;
    *) echo "error: use Scripts/build-app.sh on macOS" >&2; exit 1 ;;
esac

cargo build --release --locked --bin skia-ui
rm -rf "$OUT"
mkdir -p "$OUT"

case "$(uname -s)" in
    Linux)
        mkdir -p "$OUT/lib"
        cp target/release/skia-ui "$OUT/loudible"
        # Copies keep the sonames the executable asks for, and nothing else.
        for library in libavformat.so.61 libavcodec.so.61 libavutil.so.59; do
            cp -L "$FFMPEG_PREFIX/lib/$library" "$OUT/lib/$library"
        done
        EXECUTABLE="$OUT/loudible"
        BUNDLED="bundled in lib/"
        ;;
    *)
        cp target/release/skia-ui.exe "$OUT/Loudible.exe"
        cp "$FFMPEG_PREFIX"/bin/av*.dll "$OUT/"
        EXECUTABLE="$OUT/Loudible.exe"
        BUNDLED="bundled next to the executable"
        ;;
esac

python3 Scripts/write-notices.py "$FFMPEG_PREFIX" "$OUT" "$BUNDLED"
"$EXECUTABLE" --self-test
echo "Built $OUT"

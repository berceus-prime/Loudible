# The FFmpeg release Loudible builds and bundles. Sourced by the other scripts.
#
# To upgrade: change the version, download ffmpeg-$V.tar.xz and .asc from https://ffmpeg.org/releases/,
# verify the signature against FFmpeg's release key (fingerprint FCF9 86EA 15E6 E293 A564
# 4F10 B432 2F04 D676 58D8, https://ffmpeg.org/ffmpeg-devel.asc), then record the SHA-256 here.
FFMPEG_VERSION="7.1.5"
FFMPEG_SHA256="de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f"
FFMPEG_URL="https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz"
FFMPEG_MINIMUM_MACOS="14.0"

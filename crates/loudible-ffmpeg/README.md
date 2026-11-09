# loudible-ffmpeg

Decrypts Audible downloads and remuxes them into M4B through FFmpeg's C API. Ported from the
Swift app's `LoudibleFFmpeg` target.

It implements `loudible-core`'s `AudioConverter` trait with `FfmpegConverter`. It sits beside
`loudible-adapters`, above `loudible-core` and below `loudible-presentation`. The root crate picks
it in `src/composition.rs`.

## Public API

| Item | Purpose |
| --- | --- |
| `FfmpegConverter` | The `AudioConverter`. `FfmpegConverter::new()` starts its worker thread. |
| `metadata_tags(&BookMetadata)` | The tags written over the source's own. Only with FFmpeg. |
| `license()` | The license FFmpeg was configured with, e.g. "LGPL version 2.1 or later". |
| `libavformat_version()` | libavformat's version, e.g. "61.7.103". |
| `missing_components()` | Required FFmpeg components the loaded build lacks; empty when complete. |

## The C bridge

`bridge/ffmpeg_bridge.c` does all the FFmpeg work, behind `bridge/ffmpeg_bridge.h`. The header
uses plain C types only, so FFmpeg's headers and structs stay out of Rust. `src/bridge.rs`
declares the same functions and supplies the callbacks.

| C function | Does |
| --- | --- |
| `aex_remux_to_m4b` | Opens, decrypts and remuxes one file, with chapters, metadata, cancel and progress callbacks. |
| `aex_ffmpeg_license` | `avformat_license()`. |
| `aex_libavformat_version` | `avformat_version()`, packed by `AV_VERSION_INT`. |
| `aex_missing_components` | Checks for the mov demuxer, ipod muxer, file protocol, AAC decoder and AAC parser. |

Functions return 0 or a negative `AVERROR`. A cancelled remux returns `aex_error_cancelled`
(`AVERROR_EXIT`). Failures write a message such as "Opening the download: No such file or
directory" into a caller buffer, naming the stage that failed.

Cancellation is a polled callback, not a C11 atomic, because MSVC only offers those behind an
experimental switch.

## Building

`build.rs` looks for FFmpeg in:

1. `LOUDIBLE_FFMPEG_DIR`, if set.
2. `Vendor/ffmpeg` at the workspace root otherwise.

`Scripts/build-ffmpeg.sh` produces `Vendor/ffmpeg`: a symlink to `Vendor/ffmpeg-<version>`, with
the version pinned in `Scripts/ffmpeg-version.sh` (currently 7.1.5). It downloads the release,
checks its SHA-256, and builds shared libraries with only the components Loudible needs. It runs
on macOS (arm64), Linux, and Windows (MSVC from MSYS2).

When `include/libavformat/avformat.h` exists there, `build.rs`:

* compiles the bridge with `cc`, all warnings as errors;
* links `avformat`, `avcodec` and `avutil` dynamically from `lib/`;
* sets the `has_ffmpeg` cfg.

When it doesn't, the build prints a warning and skips all of that. Without `has_ffmpeg` the crate
exports `src/unavailable.rs` instead:

| Item | Without FFmpeg |
| --- | --- |
| `FfmpegConverter::convert` | Fails with `ExporterError::Conversion("This build does not include FFmpeg.")`. |
| `license()`, `libavformat_version()` | `"none"` |
| `missing_components()` | `["FFmpeg"]` |
| `metadata_tags` | Not exported. |

So the rest of the workspace builds and runs everywhere; only export is unavailable.

The build reruns when `LOUDIBLE_FFMPEG_DIR`, `bridge/` or FFmpeg's `lib/` changes.

## LGPL

FFmpeg must stay LGPL-2.1-or-later. The pieces that keep it so:

* `build-ffmpeg.sh` refuses a configuration that enables GPL, version 3 or non-free parts, and
  copies the license texts into `share/licenses/`.
* `build.rs` links dynamically only, so the libraries can be replaced.
* `license()` and `missing_components()` let the shipped app check the FFmpeg it actually loads.
  The root binary's `--self-test` prints both with `libavformat_version()`, and exits nonzero
  unless the license contains "LGPL" and nothing is missing. `Scripts/verify-app.sh` and
  `Scripts/package.sh` run it.

## FfmpegConverter

| Aspect | Behaviour |
| --- | --- |
| Decryption | Through the MOV demuxer's options. AAXC passes `audible_key` and `audible_iv`; AAX passes `activation_bytes`. All are hex-encoded. |
| Remux | Copies the compressed audio and cover art into the `ipod` muxer with `+faststart`. Nothing is re-encoded. Chapter text tracks and other streams are dropped. A file without audio fails. |
| Chapters | The request's chapters replace the source's. Zero-length chapters are dropped; untitled ones become "Chapter N". With no chapters, the source's are kept. |
| Metadata | Merged over the source's tags; an empty value removes a tag. See below. |
| Text | Strings end at the first NUL, as C reads them. Paths are passed as bytes on Unix and UTF-8 elsewhere. |
| Progress | The primary audio stream's position over its duration, reported at most every 0.5%, then 1.0 at the end. |
| Cancellation | Cancelling the token stops the remux, polled per packet and by FFmpeg's interrupt callback. The result is `ExporterError::Cancelled`. Dropping the future does not stop a started conversion. |
| Failure | Any error or cancellation deletes the output file. FFmpeg errors become `ExporterError::Conversion` with the stage message. |
| Panics | A panic in the progress callback is caught before it can unwind through C. It stops the remux and becomes `ExporterError::Unexpected`. The worker keeps running. |
| Threading | Conversions run one at a time, in order, on a dedicated thread named `Loudible.FFmpegConverter`, never on the async runtime. |

Tags written by `metadata_tags`:

| Tag | Value |
| --- | --- |
| `title`, `album` | The book's title |
| `genre` | `Audiobook` |
| `comment` | `ASIN: <asin>` |
| `artist`, `album_artist` | Authors, comma-separated; cleared if absent |
| `composer` | Narrators, comma-separated; cleared if absent |
| `description` | Subtitle; cleared if absent |
| `copyright` | Publisher; cleared if absent |
| `date` | Release date; cleared if absent |

## Tests

```sh
Scripts/build-ffmpeg.sh          # once, from the workspace root
cargo test -p loudible-ffmpeg
```

The tests need FFmpeg. Without it they compile to nothing and pass trivially.

| Test | Covers |
| --- | --- |
| `src/bridge.rs` | The linked FFmpeg is LGPL, libavformat 61.x, and has every required component. |
| `tests/remux.rs` | Real FFmpeg over a hand-built, unencrypted AAC M4A: audio copied unchanged, `moov` before `mdat`, chapters, metadata, cover art, AAX options, blank and empty chapters, NUL truncation, non-ASCII paths, missing or non-audio input, cancelling before and during, a panicking callback, and serial execution on one thread. |

Real AAX/AAXC decryption is not tested, as in the Swift app: no synthetic encrypted file exists.
The demuxer accepts the key options on the unencrypted file, so only option passing is checked.

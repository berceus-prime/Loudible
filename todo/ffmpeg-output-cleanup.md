# Preserve an existing output when conversion fails

**Context:** `FfmpegConverter::convert` in `crates/loudible-ffmpeg/src/ffmpeg_converter.rs` calls `remux` with the requested output path and removes that path on any error. The current export workflow supplies a unique partial path, but the public converter accepts arbitrary paths.

**Problem:** Cancellation or an input error can delete a file that existed before conversion. If input and output refer to the same file, conversion may truncate and then delete the source.

**Done when:** A failed conversion never removes or corrupts a pre-existing destination or its input. Define temporary-output and replacement behavior, then test failure before opening output, failure after writing, cancellation, and identical input/output paths.

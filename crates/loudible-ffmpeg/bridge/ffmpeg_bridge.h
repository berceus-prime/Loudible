#ifndef LOUDIBLE_FFMPEG_BRIDGE_H
#define LOUDIBLE_FFMPEG_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

// The C side of Loudible's FFmpeg use. Plain C types only, so FFmpeg's headers
// stay out of Rust. Functions returning int return 0 or a negative AVERROR code.

/// Polled by FFmpeg's interrupt callback and the remux loop, on the calling thread.
/// Returns nonzero to stop.
typedef int (*aex_cancel_fn)(void *context);

typedef struct {
    int64_t start_ms;
    int64_t end_ms;
    const char *title;
} aex_chapter;

/// A key/value pair. For metadata, a NULL or empty value removes the key.
typedef struct {
    const char *key;
    const char *value;
} aex_tag;

typedef void (*aex_progress_fn)(double fraction, void *context);

/// Returned when the cancel callback stopped the remux (FFmpeg's AVERROR_EXIT).
extern const int aex_error_cancelled;

/// Decrypts (through the MOV demuxer options) and remuxes the audio and cover art of
/// `input_path` into an M4B at `output_path`, without re-encoding.
///
/// - demuxer_options: e.g. audible_key/audible_iv (AAXC) or activation_bytes (AAX).
/// - chapters: replace the input's chapters; with a count of 0 the input's are kept.
/// - metadata: merged over the input's tags.
/// - message: receives a readable description of a failure.
int aex_remux_to_m4b(const char *input_path, const char *output_path,
                     const aex_tag *demuxer_options, int option_count,
                     const aex_chapter *chapters, int chapter_count,
                     const aex_tag *metadata, int metadata_count,
                     aex_cancel_fn is_cancelled, void *cancel_context,
                     aex_progress_fn progress, void *progress_context,
                     char *message, size_t message_size);

/// The license FFmpeg was configured with, e.g. "LGPL version 2.1 or later".
const char *aex_ffmpeg_license(void);

/// libavformat's version, packed by AV_VERSION_INT.
unsigned aex_libavformat_version(void);

/// Writes a comma-separated list of required FFmpeg components that are missing into
/// `buffer` and returns how many are missing (0 when FFmpeg has everything).
int aex_missing_components(char *buffer, size_t size);

#endif

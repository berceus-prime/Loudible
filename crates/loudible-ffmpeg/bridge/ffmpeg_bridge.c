#include "ffmpeg_bridge.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/dict.h>
#include <libavutil/error.h>
#include <libavutil/mem.h>

const int aex_error_cancelled = AVERROR_EXIT;

// MARK: - Cancellation

// A callback rather than a C11 atomic, which MSVC only offers behind an experimental switch.
typedef struct {
    aex_cancel_fn is_cancelled;
    void *context;
} cancellation;

static int interrupt_requested(void *opaque) {
    const cancellation *cancel = opaque;
    return cancel->is_cancelled ? cancel->is_cancelled(cancel->context) : 0;
}

// MARK: - Remuxing

static void describe_failure(char *message, size_t size, const char *stage, int error) {
    if (!message || size == 0) return;
    char reason[AV_ERROR_MAX_STRING_SIZE] = {0};
    av_strerror(error, reason, sizeof(reason));
    snprintf(message, size, "%s: %s", stage, reason);
}

/// Allocates chapters with FFmpeg's allocator so avformat_free_context releases them.
static int add_chapter(AVFormatContext *context, AVRational time_base, int64_t start, int64_t end,
                       const AVDictionary *metadata, const char *title) {
    AVChapter **chapters = av_realloc_array(context->chapters, context->nb_chapters + 1, sizeof(*chapters));
    if (!chapters) return AVERROR(ENOMEM);
    context->chapters = chapters;

    AVChapter *chapter = av_mallocz(sizeof(*chapter));
    if (!chapter) return AVERROR(ENOMEM);
    chapters[context->nb_chapters++] = chapter;
    chapter->id = context->nb_chapters;
    chapter->time_base = time_base;
    chapter->start = start;
    chapter->end = end;

    int result = metadata ? av_dict_copy(&chapter->metadata, metadata, 0) : 0;
    if (result >= 0 && title) result = av_dict_set(&chapter->metadata, "title", title, 0);
    return result < 0 ? result : 0;
}

static int set_chapters(AVFormatContext *output, const AVFormatContext *input,
                        const aex_chapter *chapters, int count) {
    if (count > 0) {
        for (int i = 0; i < count; i++) {
            int result = add_chapter(output, (AVRational){1, 1000}, chapters[i].start_ms, chapters[i].end_ms,
                                     NULL, chapters[i].title ? chapters[i].title : "");
            if (result < 0) return result;
        }
        return 0;
    }
    for (unsigned i = 0; i < input->nb_chapters; i++) {
        const AVChapter *chapter = input->chapters[i];
        int result = add_chapter(output, chapter->time_base, chapter->start, chapter->end, chapter->metadata, NULL);
        if (result < 0) return result;
    }
    return 0;
}

static int apply_metadata(AVFormatContext *output, const AVFormatContext *input,
                          const aex_tag *tags, int count) {
    int result = av_dict_copy(&output->metadata, input->metadata, 0);
    for (int i = 0; result >= 0 && i < count; i++) {
        const char *value = tags[i].value && tags[i].value[0] ? tags[i].value : NULL;
        result = av_dict_set(&output->metadata, tags[i].key, value, 0);
    }
    return result < 0 ? result : 0;
}

static double duration_seconds(const AVFormatContext *input, const AVStream *audio) {
    if (audio->duration > 0) return audio->duration * av_q2d(audio->time_base);
    if (input->duration > 0) return input->duration / (double)AV_TIME_BASE;
    return 0;
}

int aex_remux_to_m4b(const char *input_path, const char *output_path,
                     const aex_tag *demuxer_options, int option_count,
                     const aex_chapter *chapters, int chapter_count,
                     const aex_tag *metadata, int metadata_count,
                     aex_cancel_fn is_cancelled, void *cancel_context,
                     aex_progress_fn progress, void *progress_context,
                     char *message, size_t message_size) {
    AVFormatContext *input = NULL, *output = NULL;
    AVDictionary *options = NULL, *muxer_options = NULL;
    AVPacket *packet = NULL;
    int *stream_map = NULL;
    int primary_audio = -1;
    const char *stage = "Opening the download";
    int result = 0;
    cancellation cancel = {is_cancelled, cancel_context};
    const AVIOInterruptCB interrupt = {interrupt_requested, &cancel};

    if (message && message_size) message[0] = '\0';

    input = avformat_alloc_context();
    if (!input) { result = AVERROR(ENOMEM); goto end; }
    input->interrupt_callback = interrupt;
    for (int i = 0; i < option_count; i++) {
        if ((result = av_dict_set(&options, demuxer_options[i].key, demuxer_options[i].value, 0)) < 0) goto end;
    }
    // On failure avformat_open_input frees the context and sets it to NULL.
    if ((result = avformat_open_input(&input, input_path, NULL, &options)) < 0) goto end;

    stage = "Reading the download";
    if ((result = avformat_find_stream_info(input, NULL)) < 0) goto end;

    stage = "Creating the M4B";
    if ((result = avformat_alloc_output_context2(&output, NULL, "ipod", output_path)) < 0) goto end;
    output->interrupt_callback = interrupt;

    // Keep audio and cover art; drop chapter text tracks and anything else.
    stream_map = av_calloc(input->nb_streams, sizeof(*stream_map));
    if (!stream_map) { result = AVERROR(ENOMEM); goto end; }
    for (unsigned i = 0; i < input->nb_streams; i++) {
        stream_map[i] = -1;
        const AVStream *source = input->streams[i];
        const int is_cover = (source->disposition & AV_DISPOSITION_ATTACHED_PIC) != 0;
        if (source->codecpar->codec_type != AVMEDIA_TYPE_AUDIO && !is_cover) continue;

        AVStream *destination = avformat_new_stream(output, NULL);
        if (!destination) { result = AVERROR(ENOMEM); goto end; }
        if ((result = avcodec_parameters_copy(destination->codecpar, source->codecpar)) < 0) goto end;
        destination->codecpar->codec_tag = 0;
        if (is_cover) {
            destination->disposition |= AV_DISPOSITION_ATTACHED_PIC;
        } else {
            destination->time_base = source->time_base;
            if (primary_audio < 0) primary_audio = (int)i;
        }
        stream_map[i] = destination->index;
    }
    if (primary_audio < 0) {
        stage = "Reading the download";
        result = AVERROR_STREAM_NOT_FOUND;
        goto end;
    }

    stage = "Writing metadata";
    if ((result = apply_metadata(output, input, metadata, metadata_count)) < 0) goto end;
    if ((result = set_chapters(output, input, chapters, chapter_count)) < 0) goto end;

    stage = "Creating the M4B";
    if ((result = avio_open2(&output->pb, output_path, AVIO_FLAG_WRITE, &output->interrupt_callback, NULL)) < 0) goto end;
    if ((result = av_dict_set(&muxer_options, "movflags", "+faststart", 0)) < 0) goto end;
    if ((result = avformat_write_header(output, &muxer_options)) < 0) goto end;

    packet = av_packet_alloc();
    if (!packet) { result = AVERROR(ENOMEM); goto end; }
    const AVStream *audio = input->streams[primary_audio];
    const double total_seconds = duration_seconds(input, audio);
    double reported = -1;

    for (;;) {
        if (interrupt_requested(&cancel)) { result = AVERROR_EXIT; goto end; }

        stage = "Decrypting audio";
        result = av_read_frame(input, packet);
        if (result == AVERROR_EOF) break;
        if (result < 0) goto end;

        const int index = packet->stream_index;
        if (index < 0 || (unsigned)index >= input->nb_streams || stream_map[index] < 0) {
            av_packet_unref(packet);
            continue;
        }
        const AVStream *source = input->streams[index];
        const AVStream *destination = output->streams[stream_map[index]];

        if (index == primary_audio && progress && total_seconds > 0 && packet->pts != AV_NOPTS_VALUE) {
            double fraction = packet->pts * av_q2d(source->time_base) / total_seconds;
            fraction = fraction < 0 ? 0 : fraction > 1 ? 1 : fraction;
            if (fraction - reported >= 0.005) {
                reported = fraction;
                progress(fraction, progress_context);
            }
        }

        av_packet_rescale_ts(packet, source->time_base, destination->time_base);
        packet->pos = -1;
        packet->stream_index = destination->index;
        stage = "Writing audio";
        // Takes ownership of the packet's data and resets it.
        if ((result = av_interleaved_write_frame(output, packet)) < 0) goto end;
    }

    stage = "Finishing the M4B";
    if ((result = av_write_trailer(output)) < 0) goto end;
    if (progress) progress(1, progress_context);
    result = 0;

end:
    if (result < 0 && interrupt_requested(&cancel)) result = AVERROR_EXIT;
    if (result == AVERROR_EXIT) {
        if (message && message_size) snprintf(message, message_size, "Cancelled");
    } else if (result < 0) {
        describe_failure(message, message_size, stage, result);
    }
    av_packet_free(&packet);
    av_dict_free(&options);
    av_dict_free(&muxer_options);
    av_free(stream_map);
    if (output) {
        avio_closep(&output->pb);
        avformat_free_context(output);
    }
    avformat_close_input(&input);
    return result;
}

// MARK: - Diagnostics

const char *aex_ffmpeg_license(void) {
    return avformat_license();
}

unsigned aex_libavformat_version(void) {
    return avformat_version();
}

static void append_missing(char *buffer, size_t size, int *count, const char *name) {
    if (buffer && size > 0) {
        size_t length = strlen(buffer);
        snprintf(buffer + length, size - length, "%s%s", *count ? ", " : "", name);
    }
    (*count)++;
}

int aex_missing_components(char *buffer, size_t size) {
    int missing = 0;
    if (buffer && size > 0) buffer[0] = '\0';

    if (!av_find_input_format("mov")) append_missing(buffer, size, &missing, "mov demuxer");
    if (!av_guess_format("ipod", NULL, NULL)) append_missing(buffer, size, &missing, "ipod muxer");
    if (!avio_find_protocol_name("file:check")) append_missing(buffer, size, &missing, "file protocol");
    if (!avcodec_find_decoder(AV_CODEC_ID_AAC)) append_missing(buffer, size, &missing, "aac decoder");

    AVCodecParserContext *parser = av_parser_init(AV_CODEC_ID_AAC);
    if (parser) av_parser_close(parser);
    else append_missing(buffer, size, &missing, "aac parser");

    return missing;
}

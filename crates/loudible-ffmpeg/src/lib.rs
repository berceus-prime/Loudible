//! Decryption and remuxing through FFmpeg's C API, ported from `LoudibleFFmpeg`.

mod bridge;
mod ffmpeg_converter;

pub use bridge::{libavformat_version, license, missing_components};
pub use ffmpeg_converter::{FfmpegConverter, metadata_tags};

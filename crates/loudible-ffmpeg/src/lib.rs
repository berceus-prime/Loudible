//! Decryption and remuxing through FFmpeg's C API, ported from `LoudibleFFmpeg`.

#[cfg(has_ffmpeg)]
mod bridge;
#[cfg(has_ffmpeg)]
mod ffmpeg_converter;

#[cfg(has_ffmpeg)]
pub use bridge::{libavformat_version, license, missing_components};
#[cfg(has_ffmpeg)]
pub use ffmpeg_converter::{FfmpegConverter, metadata_tags};

#[cfg(not(has_ffmpeg))]
mod unavailable;
#[cfg(not(has_ffmpeg))]
pub use unavailable::{FfmpegConverter, libavformat_version, license, missing_components};

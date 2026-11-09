use async_trait::async_trait;
use loudible_core::{AudioConverter, ConversionProgressSink, ConversionRequest, ExporterError};
use tokio_util::sync::CancellationToken;

/// Stands in on platforms without an FFmpeg build yet, so everything but exporting still works.
pub struct FfmpegConverter;

impl FfmpegConverter {
    pub fn new() -> Result<Self, ExporterError> {
        Ok(Self)
    }
}

#[async_trait]
impl AudioConverter for FfmpegConverter {
    async fn convert(
        &self,
        _: ConversionRequest,
        _: ConversionProgressSink,
        _: &CancellationToken,
    ) -> Result<(), ExporterError> {
        Err(ExporterError::Conversion(
            "This build does not include FFmpeg.".into(),
        ))
    }
}

pub fn license() -> String {
    "none".into()
}

pub fn libavformat_version() -> String {
    "none".into()
}

pub fn missing_components() -> Vec<String> {
    vec!["FFmpeg".into()]
}

use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::ptr;
use std::sync::mpsc;

use async_trait::async_trait;
use loudible_core::{
    AudioConverter, BookMetadata, ConversionProgressSink, ConversionRequest, Decryption,
    ExporterError, hex_encoding,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::bridge::{
    AexChapter, AexTag, Callbacks, aex_error_cancelled, aex_remux_to_m4b, is_cancelled,
    report_progress,
};

type Job = Box<dyn FnOnce() + Send>;

/// Decrypts AAX/AAXC with FFmpeg's MOV demuxer and remuxes the compressed audio (no
/// re-encoding) and cover art into an M4B with the license's chapters and book metadata.
///
/// Conversions run one at a time on a dedicated thread, never on the async runtime.
pub struct FfmpegConverter {
    jobs: mpsc::Sender<Job>,
}

impl FfmpegConverter {
    pub fn new() -> Result<Self, ExporterError> {
        let (jobs, queue) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("Loudible.FFmpegConverter".into())
            .spawn(move || queue.into_iter().for_each(|job| job()))
            .map_err(|error| ExporterError::Unexpected(error.to_string()))?;
        Ok(Self { jobs })
    }
}

#[async_trait]
impl AudioConverter for FfmpegConverter {
    /// Dropping this future leaves a started conversion running; cancel the token to stop it.
    async fn convert(
        &self,
        request: ConversionRequest,
        progress: ConversionProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), ExporterError> {
        let (reply, result) = oneshot::channel();
        let cancel = cancel.clone();
        let job = Box::new(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| remux(&request, &cancel, &*progress)))
                .unwrap_or_else(|_| Err(panic_error()));
            if outcome.is_err() {
                let _ = std::fs::remove_file(&request.output);
            }
            let _ = reply.send(outcome);
        });
        let stopped = || ExporterError::Unexpected("The conversion thread stopped.".into());
        self.jobs.send(job).map_err(|_| stopped())?;
        result.await.map_err(|_| stopped())?
    }
}

fn panic_error() -> ExporterError {
    ExporterError::Unexpected("The conversion stopped because of an internal error.".into())
}

/// C strings that live as long as this object. C text ends at the first NUL, so does this.
#[derive(Default)]
struct CStrings(Vec<CString>);

impl CStrings {
    fn keep(&mut self, text: CString) -> *const c_char {
        // The pointer is to the heap allocation, which moving the `CString` leaves in place.
        let pointer = text.as_ptr();
        self.0.push(text);
        pointer
    }

    fn add(&mut self, text: &str) -> *const c_char {
        let end = text.find('\0').unwrap_or(text.len());
        self.keep(CString::new(&text[..end]).expect("no NUL before `end`"))
    }

    fn add_path(&mut self, path: &Path) -> *const c_char {
        #[cfg(unix)]
        let bytes = {
            use std::os::unix::ffi::OsStrExt;
            path.as_os_str().as_bytes().to_vec()
        };
        // FFmpeg takes UTF-8 paths on Windows and converts them itself.
        #[cfg(not(unix))]
        let bytes = path.to_string_lossy().into_owned().into_bytes();
        self.keep(CString::new(bytes).unwrap_or_default())
    }

    fn tags(&mut self, tags: &[(String, String)]) -> Vec<AexTag> {
        tags.iter()
            .map(|(key, value)| AexTag {
                key: self.add(key),
                value: self.add(value),
            })
            .collect()
    }
}

fn demuxer_options(decryption: &Decryption) -> Vec<(String, String)> {
    let option = |name: &str, bytes: &[u8]| (name.to_string(), hex_encoding::encode(bytes));
    match decryption {
        Decryption::Aaxc { key, iv } => vec![option("audible_key", key), option("audible_iv", iv)],
        Decryption::Aax { activation_bytes } => vec![option("activation_bytes", activation_bytes)],
    }
}

fn remux(
    request: &ConversionRequest,
    cancel: &CancellationToken,
    progress: &(dyn Fn(f64) + Send + Sync),
) -> Result<(), ExporterError> {
    let mut strings = CStrings::default();
    let input = strings.add_path(&request.input);
    let output = strings.add_path(&request.output);
    let options = strings.tags(&demuxer_options(&request.decryption));
    let metadata = strings.tags(&metadata_tags(&request.metadata));
    let chapters: Vec<AexChapter> = request
        .chapters
        .iter()
        .filter(|chapter| chapter.length_milliseconds > 0)
        .enumerate()
        .map(|(index, chapter)| AexChapter {
            start_ms: chapter.start_milliseconds,
            end_ms: chapter.end_milliseconds(),
            title: if chapter.title.is_empty() {
                strings.add(&format!("Chapter {}", index + 1))
            } else {
                strings.add(&chapter.title)
            },
        })
        .collect();

    let callbacks = Callbacks {
        cancel,
        progress,
        panicked: Cell::new(false),
    };
    let context = ptr::from_ref(&callbacks).cast_mut().cast::<c_void>();
    let mut message: [c_char; 512] = [0; 512];
    let count = |length: usize| c_int::try_from(length).unwrap_or(c_int::MAX);
    // Every pointer outlives the call: `strings`, the vectors, and `callbacks` are all locals.
    let result = unsafe {
        aex_remux_to_m4b(
            input,
            output,
            options.as_ptr(),
            count(options.len()),
            chapters.as_ptr(),
            count(chapters.len()),
            metadata.as_ptr(),
            count(metadata.len()),
            Some(is_cancelled),
            context,
            Some(report_progress),
            context,
            message.as_mut_ptr(),
            message.len(),
        )
    };
    if callbacks.panicked.get() {
        return Err(panic_error());
    }
    if result >= 0 {
        return Ok(());
    }
    if result == unsafe { aex_error_cancelled } {
        return Err(ExporterError::Cancelled);
    }
    // The bridge always NUL-terminates what it writes.
    let message = unsafe { CStr::from_ptr(message.as_ptr()) };
    Err(ExporterError::Conversion(
        message.to_string_lossy().into_owned(),
    ))
}

/// Book details written over the source's tags. An empty value removes the tag.
pub fn metadata_tags(book: &BookMetadata) -> Vec<(String, String)> {
    let authors = book.authors.join(", ");
    let mut tags: Vec<(&str, String)> = vec![
        ("title", book.title.clone()),
        ("album", book.title.clone()),
        ("genre", "Audiobook".to_string()),
        ("comment", format!("ASIN: {}", book.asin)),
    ];
    tags.push(("artist", authors.clone()));
    tags.push(("album_artist", authors));
    tags.push(("composer", book.narrators.join(", ")));
    tags.push(("description", book.subtitle.clone().unwrap_or_default()));
    tags.push(("copyright", book.publisher.clone().unwrap_or_default()));
    tags.push(("date", book.release_date.clone().unwrap_or_default()));
    tags.into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

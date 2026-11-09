//! The C bridge's declarations, and the Rust callbacks it calls.

use std::cell::Cell;
use std::ffi::{CStr, c_char, c_double, c_int, c_uint, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

use tokio_util::sync::CancellationToken;

#[repr(C)]
pub struct AexChapter {
    pub start_ms: i64,
    pub end_ms: i64,
    pub title: *const c_char,
}

#[repr(C)]
pub struct AexTag {
    pub key: *const c_char,
    pub value: *const c_char,
}

pub type CancelFn = unsafe extern "C" fn(*mut c_void) -> c_int;
pub type ProgressFn = unsafe extern "C" fn(c_double, *mut c_void);

unsafe extern "C" {
    pub static aex_error_cancelled: c_int;

    #[allow(clippy::too_many_arguments)]
    pub fn aex_remux_to_m4b(
        input_path: *const c_char,
        output_path: *const c_char,
        demuxer_options: *const AexTag,
        option_count: c_int,
        chapters: *const AexChapter,
        chapter_count: c_int,
        metadata: *const AexTag,
        metadata_count: c_int,
        is_cancelled: Option<CancelFn>,
        cancel_context: *mut c_void,
        progress: Option<ProgressFn>,
        progress_context: *mut c_void,
        message: *mut c_char,
        message_size: usize,
    ) -> c_int;
    fn aex_ffmpeg_license() -> *const c_char;
    fn aex_libavformat_version() -> c_uint;
    fn aex_missing_components(buffer: *mut c_char, size: usize) -> c_int;
}

/// What FFmpeg's callbacks reach, on the thread running the remux.
pub struct Callbacks<'a> {
    pub cancel: &'a CancellationToken,
    pub progress: &'a (dyn Fn(f64) + Send + Sync),
    /// A panic must not unwind through C, so it stops the remux and is reported afterwards.
    pub panicked: Cell<bool>,
}

/// # Safety
/// `context` must point to a live `Callbacks` used only on this thread.
pub unsafe extern "C" fn is_cancelled(context: *mut c_void) -> c_int {
    let callbacks = unsafe { &*context.cast::<Callbacks>() };
    c_int::from(callbacks.panicked.get() || callbacks.cancel.is_cancelled())
}

/// # Safety
/// `context` must point to a live `Callbacks` used only on this thread.
pub unsafe extern "C" fn report_progress(fraction: c_double, context: *mut c_void) {
    let callbacks = unsafe { &*context.cast::<Callbacks>() };
    if callbacks.panicked.get() {
        return;
    }
    if catch_unwind(AssertUnwindSafe(|| (callbacks.progress)(fraction))).is_err() {
        callbacks.panicked.set(true);
    }
}

/// The license FFmpeg was built under, e.g. "LGPL version 2.1 or later".
pub fn license() -> String {
    // FFmpeg returns a static NUL-terminated string.
    unsafe { CStr::from_ptr(aex_ffmpeg_license()) }
        .to_string_lossy()
        .into_owned()
}

/// libavformat version, e.g. "61.7.103".
pub fn libavformat_version() -> String {
    let version = unsafe { aex_libavformat_version() };
    format!(
        "{}.{}.{}",
        version >> 16,
        (version >> 8) & 0xff,
        version & 0xff
    )
}

/// Required FFmpeg components missing from the loaded build; empty when complete.
pub fn missing_components() -> Vec<String> {
    let mut buffer = [0 as c_char; 256];
    if unsafe { aex_missing_components(buffer.as_mut_ptr(), buffer.len()) } <= 0 {
        return vec![];
    }
    // The bridge always NUL-terminates what it writes within `size`.
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .split(", ")
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_the_lgpl_build_with_every_required_component() {
        assert!(license().contains("LGPL"), "license: {}", license());
        assert!(libavformat_version().starts_with("61."));
        assert_eq!(missing_components(), Vec::<String>::new());
    }
}

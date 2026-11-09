//! Output names: `Title [ASIN].m4b`, then `Title [ASIN] (2).m4b`, ... on collisions. Downloads use
//! the same scheme with their own extension (`Title [ASIN].aaxc`).

use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::Audiobook;
use crate::text::{characters, is_ascii_alphanumeric, is_control, split_whitespace};

pub const FILE_EXTENSION: &str = "m4b";
/// The limit on a file name, in UTF-8 bytes. NTFS counts UTF-16 units, which is never more.
const MAXIMUM_FILE_NAME_BYTES: usize = 255;
/// Room kept for a collision suffix such as " (9999)".
const COLLISION_SUFFIX_RESERVE: usize = 7;

const TRIMMED: [char; 2] = ['.', ' '];

const WINDOWS_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn sanitized_asin(asin: &str) -> String {
    characters(asin)
        .filter(|c| is_ascii_alphanumeric(c))
        .collect()
}

pub fn base_name(book: &Audiobook, extension: &str) -> String {
    let asin = sanitized_asin(&book.asin);
    let tail = format!(" [{}]", if asin.is_empty() { "unknown" } else { &asin });
    let budget =
        MAXIMUM_FILE_NAME_BYTES - tail.len() - COLLISION_SUFFIX_RESERVE - extension.len() - 1;
    let title = truncated(&sanitized_title(&book.title), budget);
    format!("{}{title}{tail}", device_name_guard(&title))
}

/// Windows reserves a device name before the first dot, even with text after it. The ` [ASIN]`
/// tail protects a plain `CON`, but not `CON.notes [ASIN].m4b`.
fn device_name_guard(title: &str) -> &'static str {
    match title.split_once('.') {
        Some((stem, _)) if WINDOWS_DEVICE_NAMES.contains(&stem.trim().to_uppercase().as_str()) => {
            "_"
        }
        _ => "",
    }
}

/// Makes a title safe as a file name on common file systems.
pub fn sanitized_title(title: &str) -> String {
    let normalized: String = title.nfc().collect();
    let mut result = String::with_capacity(normalized.len());
    for character in characters(&normalized) {
        match character {
            ":" => result.push_str(" - "),
            "/" | "\\" | "?" | "*" | "\"" | "<" | ">" | "|" => result.push('_'),
            _ if character.chars().all(is_control) => result.push(' '),
            _ => result.push_str(character),
        }
    }
    let collapsed = split_whitespace(&result).join(" ");
    // Leading dots would hide the file; trailing dots and spaces trip up other systems.
    let trimmed = collapsed.trim_matches(TRIMMED);
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn file_name(base: &str, attempt: u32, extension: &str) -> String {
    if attempt <= 1 {
        format!("{base}.{extension}")
    } else {
        format!("{base} ({attempt}).{extension}")
    }
}

/// The first `file_name` in `directory` for which `exists` is false.
pub fn available_path(
    directory: &Path,
    base: &str,
    extension: &str,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let mut attempt = 1;
    loop {
        let path = directory.join(file_name(base, attempt, extension));
        if !exists(&path) {
            return path;
        }
        attempt += 1;
    }
}

/// The ASIN tagged in a file name: the last `[ALNUM]` group of the name without its extension.
/// Matching on the tag rather than the whole name survives title changes and user renames.
/// Hidden files never match.
pub fn asin_in_file_name(name: &str) -> Option<String> {
    if name.starts_with('.') {
        return None;
    }
    let stem = Path::new(name).file_stem()?.to_str()?;
    let mut found = None;
    let mut current: Option<String> = None;
    for character in characters(stem) {
        match character {
            "[" => current = Some(String::new()),
            "]" => {
                if let Some(asin) = current.take().filter(|asin| !asin.is_empty()) {
                    found = Some(asin);
                }
            }
            _ => {
                if let Some(asin) = &mut current {
                    if is_ascii_alphanumeric(character) {
                        asin.push_str(character);
                    } else {
                        current = None;
                    }
                }
            }
        }
    }
    found
}

/// Hidden name for a file still being written: `.<base>.<token>.partial`. The base is
/// shortened if needed so the whole name stays within the file name limit.
pub fn partial_name(base: &str, token: &str) -> String {
    let fixed = ".".len() + token.len() + ".partial".len() + 1;
    format!(
        ".{}.{token}.partial",
        truncated(base, MAXIMUM_FILE_NAME_BYTES - fixed)
    )
}

/// Hidden staging name for a download: `.<ASIN>.download.partial`.
pub fn download_staging_name(asin: &str) -> String {
    format!(".{}.download.partial", sanitized_asin(asin))
}

/// Whether `name` is one of the conversion or download staging names.
pub fn is_partial(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".partial")
}

fn truncated(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut result = String::new();
    for character in characters(text) {
        if result.len() + character.len() > limit {
            break;
        }
        result.push_str(character);
    }
    result.trim_matches(TRIMMED).to_string()
}

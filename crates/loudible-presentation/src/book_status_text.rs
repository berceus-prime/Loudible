use loudible_core::{BookStatus, JobKind, TransferProgress};

/// What a screen reader reads as the book's value, also the badge tooltip. `None` when there is
/// no badge.
pub fn accessibility_text(status: &BookStatus) -> Option<String> {
    let word = |kind: &JobKind, download: &'static str, export: &'static str| match kind {
        JobKind::Download => download,
        JobKind::Export => export,
    };
    match status {
        BookStatus::None => None,
        BookStatus::Queued(kind) => {
            Some(word(kind, "Queued for download", "Queued for export").to_string())
        }
        BookStatus::Running {
            kind,
            progress,
            transferred,
        } => {
            let mut text = format!(
                "{}, {} percent",
                word(kind, "Downloading", "Exporting"),
                (progress * 100.0).floor() as i64
            );
            if let Some(transferred) = transferred {
                text.push_str(&format!(
                    ", {} of {}",
                    megabytes(transferred.bytes_received),
                    megabytes(transferred.bytes_expected)
                ));
            }
            Some(text)
        }
        BookStatus::Failed { kind, message } => {
            // Some reasons already say what failed ("Download failed: HTTP 500").
            let text = if message.to_lowercase().contains("failed:") {
                message.clone()
            } else {
                format!("{} failed: {message}", word(kind, "Download", "Export"))
            };
            Some(text)
        }
        BookStatus::Downloaded => Some("Downloaded".to_string()),
        BookStatus::Exported => Some("Exported to M4B".to_string()),
    }
}

/// "132 MB / 288 MB".
pub fn transfer_text(progress: &TransferProgress) -> String {
    format!(
        "{} / {}",
        megabytes(progress.bytes_received),
        megabytes(progress.bytes_expected)
    )
}

/// Megabytes to three significant digits, as `ByteCountFormatter` shows them in English.
fn megabytes(bytes: i64) -> String {
    const MEGABYTE: u64 = 1_000_000;
    let bytes = bytes.max(0) as u64;
    if bytes == 0 {
        return "Zero KB".to_string();
    }
    // Whole-number arithmetic: 19,950,000 bytes must round up, and 19.95 has no exact float.
    let digits = bytes.ilog10() + 1;
    let unit = 10_u64.pow(digits.saturating_sub(3));
    let rounded = (bytes + unit / 2) / unit * unit;
    if rounded >= 1_000 * MEGABYTE {
        return format!("{} MB", grouped((bytes + MEGABYTE / 2) / MEGABYTE));
    }
    let fraction = format!("{:06}", rounded % MEGABYTE);
    let fraction = fraction.trim_end_matches('0');
    let whole = rounded / MEGABYTE;
    if fraction.is_empty() {
        format!("{whole} MB")
    } else {
        format!("{whole}.{fraction} MB")
    }
}

fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

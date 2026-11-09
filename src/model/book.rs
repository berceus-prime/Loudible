//! Books, their status on the cover, and the actions their menus offer.

pub use loudible_core::Marketplace;

pub use super::platform::Platform;

/// A book's identity, cheap to copy into every hit area.
pub type Asin = std::sync::Arc<str>;

/// A cover drawn as a solid colour with a light stripe, like the synthetic Swift previews.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoverColor(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Audiobook {
    pub asin: Asin,
    pub marketplace: Marketplace,
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
    pub runtime_minutes: Option<u32>,
    /// The synthetic previews' cover.
    pub cover: Option<CoverColor>,
    /// Where a real cover image can be fetched.
    pub cover_url: Option<String>,
    /// Size of the audio file, for the fake download progress.
    pub size_bytes: u64,
}

impl Audiobook {
    pub fn authors_text(&self) -> String {
        self.authors.join(", ")
    }

    /// "10h 42m", or `None` when the runtime is unknown.
    pub fn runtime_text(&self) -> Option<String> {
        self.runtime_minutes
            .map(|m| format!("{}h {}m", m / 60, m % 60))
    }

    /// Whether `query` matches the title, an author or a narrator, ignoring case.
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        std::iter::once(&self.title)
            .chain(&self.authors)
            .chain(&self.narrators)
            .any(|field| field.to_lowercase().contains(&query))
    }

    /// The bytes of this book's audio file received at `progress` (0...1).
    pub fn transferred(&self, progress: f32) -> TransferProgress {
        TransferProgress {
            received: (f64::from(progress) * self.size_bytes as f64) as u64,
            expected: self.size_bytes,
        }
    }

    /// The tooltip lines: title, subtitle, authors, narrators.
    pub fn tooltip(&self) -> String {
        let mut lines = vec![self.title.clone()];
        lines.extend(self.subtitle.clone());
        if !self.authors.is_empty() {
            lines.push(format!("By {}", self.authors_text()));
        }
        if !self.narrators.is_empty() {
            lines.push(format!("Narrated by {}", self.narrators.join(", ")));
        }
        lines.join("\n")
    }
}

/// Bytes of a download received so far, or kept for the next attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferProgress {
    pub received: u64,
    pub expected: u64,
}

impl TransferProgress {
    /// "132 MB / 288 MB".
    pub fn text(&self) -> String {
        loudible_presentation::transfer_text(&loudible_core::TransferProgress::new(
            self.received as i64,
            self.expected as i64,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JobKind {
    Download,
    Export,
}

/// The one status shown on a book's cover.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum BookStatus {
    #[default]
    None,
    Queued(JobKind),
    /// `progress` is 0...1; `transferred` is set while the audio file downloads.
    Running {
        kind: JobKind,
        progress: f32,
        transferred: Option<TransferProgress>,
    },
    /// The latest attempt failed; `kept` is the part of the download kept for next time.
    Failed {
        kind: JobKind,
        message: String,
        kept: Option<TransferProgress>,
    },
    Paused(TransferProgress),
    Downloaded,
    Exported,
}

/// The groups the grid shows books in, in display order, as Heronite groups games by
/// play status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BookSection {
    InProgress,
    NeedsAttention,
    Downloaded,
    Exported,
    NotDownloaded,
}

impl BookSection {
    pub fn title(self) -> &'static str {
        match self {
            Self::InProgress => "In Progress",
            Self::NeedsAttention => "Needs Attention",
            Self::Downloaded => "Downloaded",
            Self::Exported => "Exported",
            Self::NotDownloaded => "Not Downloaded",
        }
    }

    /// Splits books into non-empty sections in display order, keeping their order within each.
    pub fn group<T>(books: Vec<T>, status: impl Fn(&T) -> &BookStatus) -> Vec<(Self, Vec<T>)> {
        let mut sections: Vec<(Self, Vec<T>)> = Vec::new();
        for book in books {
            let section = status(&book).section();
            match sections.iter_mut().find(|(s, _)| *s == section) {
                Some((_, books)) => books.push(book),
                None => sections.push((section, vec![book])),
            }
        }
        sections.sort_by_key(|(section, _)| *section);
        sections
    }
}

impl BookStatus {
    pub fn section(&self) -> BookSection {
        match self {
            Self::Queued(_) | Self::Running { .. } | Self::Paused(_) => BookSection::InProgress,
            Self::Failed { .. } => BookSection::NeedsAttention,
            Self::Downloaded => BookSection::Downloaded,
            Self::Exported => BookSection::Exported,
            Self::None => BookSection::NotDownloaded,
        }
    }

    /// The bytes to show in place of the runtime.
    pub fn transferred(&self) -> Option<TransferProgress> {
        match self {
            Self::Running { transferred, .. } => *transferred,
            Self::Failed { kept, .. } => *kept,
            Self::Paused(kept) => Some(*kept),
            Self::None | Self::Queued(_) | Self::Downloaded | Self::Exported => None,
        }
    }

    pub fn tooltip(&self) -> Option<String> {
        Some(match self {
            Self::None => return None,
            Self::Queued(JobKind::Download) => "Waiting to download".into(),
            Self::Queued(JobKind::Export) => "Waiting to export".into(),
            Self::Running { kind, progress, .. } => {
                let verb = if *kind == JobKind::Download {
                    "Downloading"
                } else {
                    "Exporting"
                };
                format!("{verb}, {}%", (progress * 100.0).round())
            }
            Self::Failed { message, .. } => format!("Failed: {message}"),
            Self::Paused(_) => "Download paused".into(),
            Self::Downloaded => "Downloaded".into(),
            Self::Exported => "Exported".into(),
        })
    }
}

/// Something the user can do to one book from its menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BookAction {
    Download,
    ResumeDownload,
    Cancel(JobKind),
    Export,
    ShowDownload,
    ShowExport,
    DeleteDownload,
}

impl BookAction {
    pub fn title(&self) -> String {
        self.title_on(Platform::CURRENT)
    }

    pub fn title_on(&self, platform: Platform) -> String {
        match self {
            Self::ShowDownload => return format!("Show in {}", platform.file_manager()),
            Self::ShowExport => {
                return format!("Show Exported File in {}", platform.file_manager());
            }
            _ => {}
        }
        match self {
            Self::Download => "Download",
            Self::ResumeDownload => "Resume Download",
            Self::Cancel(JobKind::Download) => "Cancel Download",
            Self::Cancel(JobKind::Export) => "Cancel Export",
            Self::Export => "Export to M4B",
            Self::DeleteDownload => "Delete Download…",
            Self::ShowDownload | Self::ShowExport => unreachable!("returned above"),
        }
        .to_string()
    }
}

/// A menu entry, with a reason when it is disabled.
#[derive(Clone, Debug, PartialEq)]
pub struct BookMenuItem {
    pub action: BookAction,
    pub disabled_reason: Option<String>,
}

impl BookMenuItem {
    pub fn new(action: BookAction) -> Self {
        Self {
            action,
            disabled_reason: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.disabled_reason.is_none()
    }
}

/// What a book's tile shows besides the book itself.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BookTileState {
    pub is_selected: bool,
    pub is_in_library: bool,
    /// Short code of the store holding the book, when it is not the signed-in one.
    pub marketplace_badge: Option<String>,
    pub status: BookStatus,
    pub menu_items: Vec<BookMenuItem>,
}

impl BookTileState {
    pub fn new() -> Self {
        Self {
            is_in_library: true,
            ..Default::default()
        }
    }
}

/// Which menu items a book with `status` offers. Mirrors `BookMenu.items` in the Swift app,
/// reduced to the facts the mock app tracks.
pub fn menu_items(status: &BookStatus, is_in_library: bool) -> Vec<BookMenuItem> {
    use BookAction::*;
    let item = BookMenuItem::new;
    match status {
        BookStatus::Queued(kind) | BookStatus::Running { kind, .. } => vec![item(Cancel(*kind))],
        BookStatus::Downloaded | BookStatus::Exported => {
            let mut export = item(Export);
            if !is_in_library {
                export.disabled_reason = Some("This book is no longer in your library.".into());
            }
            let mut items = vec![export, item(ShowDownload)];
            if *status == BookStatus::Exported {
                items.push(item(ShowExport));
            }
            items.push(item(DeleteDownload));
            items
        }
        BookStatus::Paused(_) | BookStatus::Failed { kept: Some(_), .. } => {
            vec![item(ResumeDownload), item(Export), item(DeleteDownload)]
        }
        BookStatus::None | BookStatus::Failed { .. } if is_in_library => {
            vec![item(Download), item(Export)]
        }
        BookStatus::None | BookStatus::Failed { .. } => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_follow_display_order_and_keep_book_order() {
        let books = vec![
            ("a", BookStatus::None),
            ("b", BookStatus::Downloaded),
            ("c", BookStatus::Queued(JobKind::Download)),
            ("d", BookStatus::None),
            ("e", BookStatus::Exported),
        ];
        let sections = BookSection::group(books, |(_, status)| status);
        let names: Vec<(BookSection, Vec<&str>)> = sections
            .into_iter()
            .map(|(section, books)| (section, books.into_iter().map(|(n, _)| n).collect()))
            .collect();
        assert_eq!(
            names,
            vec![
                (BookSection::InProgress, vec!["c"]),
                (BookSection::Downloaded, vec!["b"]),
                (BookSection::Exported, vec!["e"]),
                (BookSection::NotDownloaded, vec!["a", "d"]),
            ]
        );
    }
}

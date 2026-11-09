use serde::{Deserialize, Serialize};

use crate::{Audiobook, Chapter, Date, Decryption};

/// What is kept next to each download so it can be exported without contacting Audible, and
/// shown after it has left the library. It contains key material: `LocalDownloads` stores it
/// outside the user-visible download folder, readable by the owner only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadManifest {
    pub version: u32,
    pub book: Audiobook,
    pub decryption: Decryption,
    pub chapters: Vec<Chapter>,
    pub file_extension: String,
    pub downloaded_at: Date,
}

impl DownloadManifest {
    pub const CURRENT_VERSION: u32 = 1;

    pub fn new(
        book: Audiobook,
        decryption: Decryption,
        chapters: Vec<Chapter>,
        downloaded_at: Date,
    ) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            book,
            file_extension: decryption.file_extension().to_string(),
            decryption,
            chapters,
            downloaded_at,
        }
    }
}

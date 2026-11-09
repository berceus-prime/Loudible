use serde::{Deserialize, Serialize};

use crate::{Audiobook, Date, Decryption, DownloadRecovery};

/// An unfinished download to resume when possible, or restart with a fresh license.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialDownload {
    pub version: u32,
    pub book: Audiobook,
    /// The account that downloaded it. Another account's license may not decrypt these bytes.
    pub customer_id: String,
    pub file_extension: String,
    pub recovery: DownloadRecovery,
    pub decryption: Decryption,
    pub updated_at: Date,
}

impl PartialDownload {
    pub const CURRENT_VERSION: u32 = 1;

    pub fn new(
        book: Audiobook,
        customer_id: impl Into<String>,
        file_extension: impl Into<String>,
        recovery: DownloadRecovery,
        decryption: Decryption,
        updated_at: Date,
    ) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            book,
            customer_id: customer_id.into(),
            file_extension: file_extension.into(),
            recovery,
            decryption,
            updated_at,
        }
    }
}

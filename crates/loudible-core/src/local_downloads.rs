use std::collections::{HashMap, HashSet};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::export_file_naming::{self as naming, FILE_EXTENSION};
use crate::{DownloadManifest, ExporterError, PartialDownload, TransferProgress};

pub const AUDIO_EXTENSIONS: [&str; 2] = ["aax", "aaxc"];

pub type Trash = Arc<dyn Fn(&Path) -> io::Result<()> + Send + Sync>;

/// The kept encrypted downloads and their manifests.
///
/// The audio file in `folder` is the source of truth for "downloaded": a book is downloaded when a
/// non-hidden `.aax`/`.aaxc` whose name carries `[ASIN]` exists there. Manifests (`<ASIN>.json`,
/// plus the cover as `<ASIN>.cover`) live in `manifest_folder`, owner-only, and a manifest without
/// its audio file is an orphan to delete. Unfinished-download records (`<ASIN>.partial`) live
/// in `manifest_folder`.
#[derive(Clone)]
pub struct LocalDownloads {
    pub folder: PathBuf,
    pub manifest_folder: PathBuf,
    trash: Trash,
}

/// Moves a file to the Trash, without the Finder's Automation permission prompt on macOS.
fn system_trash(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let context = {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut context = trash::TrashContext::default();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context
    };
    #[cfg(not(target_os = "macos"))]
    let context = trash::TrashContext::default();
    context.delete(path).map_err(io::Error::other)
}

/// Whether a file can be created in `directory`. The only check that means the same everywhere.
pub fn is_writable_directory(directory: &Path) -> bool {
    let probe = directory.join(format!(".write-check.{}.tmp", Uuid::new_v4()));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

/// Hidden by the file system's flag rather than a leading dot. Dot files are ruled out by name.
fn has_hidden_flag(metadata: &fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::darwin::fs::MetadataExt;
        const UF_HIDDEN: u32 = 0x8000;
        metadata.st_flags() & UF_HIDDEN != 0
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = metadata;
        false
    }
}

/// Names in `directory`; a missing or unreadable folder has none.
fn names_in(directory: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

fn create_private_directory(directory: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)
}

fn make_private(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Replaces `destination` atomically. Windows refuses while anything, even an indexer or virus
/// scanner, briefly has the file open, so it gets a few tries.
fn replace(destination: &Path, source: &Path) -> io::Result<()> {
    const ATTEMPTS: u32 = if cfg!(windows) { 5 } else { 1 };
    let mut attempt = 1;
    loop {
        match fs::rename(source, destination) {
            Err(error) if attempt < ATTEMPTS && error.kind() == io::ErrorKind::PermissionDenied => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                attempt += 1;
            }
            result => return result,
        }
    }
}

impl LocalDownloads {
    pub fn new(folder: PathBuf, manifest_folder: PathBuf) -> Self {
        Self::with_trash(folder, manifest_folder, Arc::new(system_trash))
    }

    /// `trash` moves a file to the Trash; tests replace it so they never touch the real one.
    pub fn with_trash(folder: PathBuf, manifest_folder: PathBuf, trash: Trash) -> Self {
        Self {
            folder,
            manifest_folder,
            trash,
        }
    }

    /// Creates both folders if needed and checks that the download folder is writable.
    pub fn ensure_folder(&self) -> Result<(), ExporterError> {
        fs::create_dir_all(&self.folder)
            .and_then(|()| create_private_directory(&self.manifest_folder))
            .map_err(|error| {
                ExporterError::FileSystem(format!(
                    "Could not create {}: {error}",
                    self.folder.display()
                ))
            })?;
        if !is_writable_directory(&self.folder) {
            return Err(ExporterError::FileSystem(format!(
                "{} is not writable.",
                self.folder.display()
            )));
        }
        Ok(())
    }

    // MARK: Scanning

    /// Downloaded audio files by ASIN.
    pub fn scan_downloads(&self) -> HashMap<String, PathBuf> {
        Self::scan(&self.folder, &AUDIO_EXTENSIONS)
    }

    /// Exported M4B files in `export_folder` by ASIN.
    pub fn scan_exports(&self, export_folder: &Path) -> HashMap<String, PathBuf> {
        Self::scan(export_folder, &[FILE_EXTENSION])
    }

    /// Non-hidden regular files with one of `extensions` (any case), keyed by the ASIN in their
    /// name. With several matches the first in name order wins.
    fn scan(directory: &Path, extensions: &[&str]) -> HashMap<String, PathBuf> {
        let Ok(entries) = fs::read_dir(directory) else {
            return HashMap::new();
        };
        let mut found: Vec<(String, fs::DirEntry)> = entries
            .flatten()
            .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry)))
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));

        let mut result = HashMap::new();
        for (name, entry) in found {
            let extension = Path::new(&name)
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_lowercase);
            if !extension.is_some_and(|extension| extensions.contains(&extension.as_str())) {
                continue;
            }
            let is_visible_file = entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry.metadata().is_ok_and(|data| !has_hidden_flag(&data));
            if is_visible_file && let Some(asin) = naming::asin_in_file_name(&name) {
                result.entry(asin).or_insert_with(|| directory.join(&name));
            }
        }
        result
    }

    // MARK: Manifests

    pub fn manifest_path(&self, asin: &str) -> PathBuf {
        self.manifest_folder.join(format!("{asin}.json"))
    }

    fn cover_path(&self, asin: &str) -> PathBuf {
        self.manifest_folder.join(format!("{asin}.cover"))
    }

    /// The stored cover image, if one was saved.
    pub fn stored_cover(&self, asin: &str) -> Option<PathBuf> {
        Some(self.cover_path(asin)).filter(|path| path.exists())
    }

    /// Every readable manifest. Unreadable ones are skipped, not deleted.
    pub fn manifests(&self) -> Vec<DownloadManifest> {
        self.manifest_asins()
            .iter()
            .filter_map(|asin| self.manifest(asin))
            .collect()
    }

    pub fn manifest(&self, asin: &str) -> Option<DownloadManifest> {
        Self::decode(&self.manifest_path(asin))
    }

    /// Writes the manifest (and moves `cover_file`, if any, next to it). Both are created with
    /// mode 0600 and appear atomically, so no reader sees a half-written or world-readable file.
    pub fn write_manifest(
        &self,
        manifest: &DownloadManifest,
        cover_file: Option<&Path>,
    ) -> Result<(), ExporterError> {
        let asin = &manifest.book.asin;
        let write = || -> io::Result<()> {
            self.write_privately(manifest, &self.manifest_path(asin))?;
            if let Some(cover) = cover_file.filter(|cover| cover.exists()) {
                make_private(cover)?;
                replace(&self.cover_path(asin), cover)?;
            }
            Ok(())
        };
        write().map_err(|error| {
            ExporterError::FileSystem(format!("Could not save the download details: {error}"))
        })
    }

    /// Writes `value` as JSON with mode 0600, atomically, creating the folder if needed.
    fn write_privately(&self, value: &impl Serialize, destination: &Path) -> io::Result<()> {
        create_private_directory(&self.manifest_folder)?;
        let name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("record");
        let temporary = self
            .manifest_folder
            .join(format!(".{name}.{}.tmp", Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let write = || -> io::Result<()> {
            let mut file = options.open(&temporary)?;
            file.write_all(&serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
            drop(file);
            replace(destination, &temporary)
        };
        let result = write();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn decode<T: DeserializeOwned>(path: &Path) -> Option<T> {
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    }

    pub fn remove_manifest(&self, asin: &str) {
        let _ = fs::remove_file(self.manifest_path(asin));
        let _ = fs::remove_file(self.cover_path(asin));
    }

    /// Deletes manifests whose ASIN is not in `keeping` (the downloaded books, plus any being
    /// downloaded right now). Returns the removed ASINs.
    pub fn remove_orphaned_manifests(&self, keeping: &HashSet<String>) -> HashSet<String> {
        let orphans: HashSet<String> = self
            .manifest_asins()
            .into_iter()
            .filter(|asin| !keeping.contains(asin))
            .collect();
        orphans.iter().for_each(|asin| self.remove_manifest(asin));
        orphans
    }

    fn manifest_asins(&self) -> Vec<String> {
        self.record_names(".json")
    }

    fn record_names(&self, suffix: &str) -> Vec<String> {
        let mut names: Vec<String> = names_in(&self.manifest_folder)
            .into_iter()
            .filter(|name| !name.starts_with('.'))
            .filter_map(|name| name.strip_suffix(suffix).map(str::to_string))
            .collect();
        names.sort();
        names
    }

    // MARK: Cleanup

    /// Moves the book's audio file to the Trash and deletes its manifest and any unfinished
    /// download (which is not worth the Trash). Exports are not touched.
    pub fn trash_download(&self, asin: &str) -> Result<(), ExporterError> {
        self.remove_partial_download(asin);
        if let Some(path) = self.scan_downloads().get(asin) {
            (self.trash)(path).map_err(|error| {
                ExporterError::FileSystem(format!(
                    "Could not move {} to the {}: {error}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    if cfg!(windows) {
                        "Recycle Bin"
                    } else {
                        "Trash"
                    }
                ))
            })?;
        }
        self.remove_manifest(asin);
        Ok(())
    }

    /// Removes app-owned partial files left by interrupted runs, except the bytes a recovery
    /// record can still resume from.
    pub fn sweep_partials(&self) {
        let resumable: HashSet<String> = self
            .partial_downloads()
            .iter()
            .filter(|record| record.recovery.resume_data.is_some())
            .map(|record| naming::resumable_partial_name(&record.book.asin))
            .collect();
        for name in names_in(&self.folder) {
            if naming::is_partial(&name) && !resumable.contains(&name) {
                let _ = fs::remove_file(self.folder.join(name));
            }
        }
        for name in names_in(&self.manifest_folder) {
            if name.starts_with('.') && (name.ends_with(".cover") || name.ends_with(".tmp")) {
                let _ = fs::remove_file(self.manifest_folder.join(name));
            }
        }
    }

    // MARK: Unfinished downloads

    /// Staging destination for a completed download, before final naming.
    pub fn partial_file_path(&self, asin: &str) -> PathBuf {
        self.folder.join(naming::resumable_partial_name(asin))
    }

    fn partial_record_path(&self, asin: &str) -> PathBuf {
        self.manifest_folder.join(format!("{asin}.partial"))
    }

    pub fn partial_download(&self, asin: &str) -> Option<PartialDownload> {
        Self::decode::<PartialDownload>(&self.partial_record_path(asin))
            .filter(|record| record.version == PartialDownload::CURRENT_VERSION)
    }

    /// Every current recovery record, including downloads that must restart from zero.
    pub fn partial_downloads(&self) -> Vec<PartialDownload> {
        self.record_names(".partial")
            .iter()
            .filter_map(|asin| self.partial_download(asin))
            .collect()
    }

    /// Owner-only, like the manifests: the record names the account.
    pub fn write_partial_download(&self, record: &PartialDownload) -> Result<(), ExporterError> {
        self.write_privately(record, &self.partial_record_path(&record.book.asin))
            .map_err(|error| {
                ExporterError::FileSystem(format!("Could not save the download progress: {error}"))
            })
    }

    /// Deletes the record and the bytes.
    pub fn remove_partial_download(&self, asin: &str) {
        let _ = fs::remove_file(self.partial_file_path(asin));
        let _ = fs::remove_file(self.partial_record_path(asin));
    }

    /// Last observed interruption progress.
    pub fn partial_progress(&self, record: &PartialDownload) -> TransferProgress {
        record
            .recovery
            .progress
            .unwrap_or(TransferProgress::new(0, 0))
    }
}

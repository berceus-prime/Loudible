use std::io;
use std::path::{Path, PathBuf};

use tempfile::TempPath;

use crate::ExporterError;
use crate::export_file_naming::file_name;

/// Places completed files without overwriting existing files.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExportFileFinalizer;

impl ExportFileFinalizer {
    /// Moves a finished file to the first free `Title [ASIN] (n).<ext>` name. The move refuses to
    /// overwrite, so a file appearing concurrently just bumps the suffix.
    pub fn move_into_place(
        &self,
        partial: &Path,
        directory: &Path,
        base: &str,
        extension: &str,
    ) -> Result<PathBuf, ExporterError> {
        for attempt in 1..10_000 {
            let candidate = directory.join(file_name(base, attempt, extension));
            if candidate.exists() {
                continue;
            }
            match move_without_overwriting(partial, &candidate) {
                Ok(()) => return Ok(candidate),
                Err(_) if candidate.exists() => {}
                Err(error) => {
                    return Err(ExporterError::FileSystem(format!(
                        "Could not move the file into place: {error}"
                    )));
                }
            }
        }
        Err(ExporterError::FileSystem(format!(
            "Too many files named {base}."
        )))
    }
}

fn move_without_overwriting(source: &Path, destination: &Path) -> io::Result<()> {
    TempPath::try_from_path(source)?
        .persist_noclobber(destination)
        .map_err(|failure| {
            // Dropping a `TempPath` deletes its file, and this one must survive a failed move.
            let _ = failure.path.keep();
            failure.error
        })
}

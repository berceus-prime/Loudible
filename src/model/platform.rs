//! Where the wording differs by operating system.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    pub const CURRENT: Self = if cfg!(target_os = "macos") {
        Self::Mac
    } else if cfg!(windows) {
        Self::Windows
    } else {
        Self::Linux
    };

    pub fn file_manager(self) -> &'static str {
        match self {
            Self::Mac => "Finder",
            Self::Windows => "File Explorer",
            Self::Linux => "File Manager",
        }
    }

    pub fn trash(self) -> &'static str {
        match self {
            Self::Mac | Self::Linux => "Trash",
            Self::Windows => "Recycle Bin",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BookAction;

    #[test]
    fn names_each_systems_file_manager() {
        let title = |platform| BookAction::ShowDownload.title_on(platform);
        assert_eq!(title(Platform::Mac), "Show in Finder");
        assert_eq!(title(Platform::Windows), "Show in File Explorer");
        assert_eq!(title(Platform::Linux), "Show in File Manager");
        assert_eq!(
            BookAction::ShowExport.title_on(Platform::Windows),
            "Show Exported File in File Explorer"
        );
        assert_eq!(
            BookAction::Export.title_on(Platform::Linux),
            "Export to M4B"
        );
    }
}

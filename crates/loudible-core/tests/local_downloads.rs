use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use loudible_core::*;
use loudible_test_support::*;

struct Fixture {
    root: TemporaryDirectory,
    trashed: Arc<Locked<Vec<String>>>,
    downloads: LocalDownloads,
}

impl Fixture {
    fn new() -> Self {
        let root = TemporaryDirectory::default();
        let trashed = Arc::new(Locked::new(Vec::new()));
        let record = trashed.clone();
        let downloads = LocalDownloads::with_trash(
            root.join("Downloads"),
            root.join("Manifests"),
            Arc::new(move |path: &Path| {
                record.with(|all: &mut Vec<String>| {
                    all.push(path.file_name().unwrap().to_string_lossy().into_owned())
                });
                fs::remove_file(path)
            }),
        );
        downloads.ensure_folder().unwrap();
        Self {
            root,
            trashed,
            downloads,
        }
    }

    fn touch(&self, name: &str) {
        self.touch_in(name, &self.downloads.folder);
    }

    fn touch_in(&self, name: &str, folder: &Path) {
        fs::write(folder.join(name), "x").unwrap();
    }

    fn manifest_files(&self) -> Vec<String> {
        contents_of(&self.downloads.manifest_folder)
    }
}

fn manifest(asin: &str) -> DownloadManifest {
    DownloadManifest::new(
        book(asin),
        Decryption::Aaxc {
            key: vec![1; 16],
            iv: vec![2; 16],
        },
        vec![Chapter {
            title: "One".into(),
            start_milliseconds: 0,
            length_milliseconds: 10,
        }],
        reference_date(),
    )
}

fn sorted_keys(map: &std::collections::HashMap<String, PathBuf>) -> Vec<String> {
    let mut keys: Vec<String> = map.keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn scan_matches_on_the_asin_tag_only() {
    let f = Fixture::new();
    for name in [
        "Renamed by me [B1].aaxc",
        "Old [B2].AAX",
        ".Hidden [B3].aaxc",
        ".Book [B4].1234.partial",
        "Notes [B5].txt",
        "Exported [B6].m4b",
        "No tag.aaxc",
    ] {
        f.touch(name);
    }
    fs::create_dir_all(f.downloads.folder.join("Folder [B7].aaxc")).unwrap();
    let found = f.downloads.scan_downloads();
    assert_eq!(sorted_keys(&found), ["B1", "B2"]);
    assert_eq!(found["B1"].file_name().unwrap(), "Renamed by me [B1].aaxc");
}

#[test]
fn duplicates_resolve_in_name_order() {
    let f = Fixture::new();
    for name in ["Book [B1] (2).aaxc", "Book [B1].aaxc", "A copy [B1].aax"] {
        f.touch(name);
    }
    assert_eq!(
        f.downloads.scan_downloads()["B1"].file_name().unwrap(),
        "A copy [B1].aax"
    );
}

#[test]
fn scans_exports_in_the_given_folder() {
    let f = Fixture::new();
    let exports = f.root.join("Exports");
    fs::create_dir_all(&exports).unwrap();
    f.touch_in("Book [B1].m4b", &exports);
    f.touch_in("Book [B2].aaxc", &exports);
    assert_eq!(sorted_keys(&f.downloads.scan_exports(&exports)), ["B1"]);
    assert!(f.downloads.scan_exports(&f.root.join("missing")).is_empty());
}

#[test]
fn manifest_round_trip_is_owner_only() {
    let f = Fixture::new();
    let cover = f.root.join("cover.tmp");
    fs::write(&cover, "png").unwrap();
    f.downloads
        .write_manifest(&manifest("B1"), Some(&cover))
        .unwrap();

    assert_eq!(f.downloads.manifest("B1"), Some(manifest("B1")));
    assert_eq!(f.downloads.manifests(), [manifest("B1")]);
    assert_eq!(f.manifest_files(), ["B1.cover", "B1.json"]);
    #[cfg(unix)]
    for name in f.manifest_files() {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(f.downloads.manifest_folder.join(&name))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{name}");
    }
    assert_eq!(
        fs::read(f.downloads.stored_cover("B1").unwrap()).unwrap(),
        b"png"
    );

    // Writing again replaces it.
    let mut updated = manifest("B1");
    updated.chapters.clear();
    f.downloads.write_manifest(&updated, None).unwrap();
    assert_eq!(f.downloads.manifest("B1").unwrap().chapters, []);
}

#[test]
fn orphaned_manifests_are_removed() {
    let f = Fixture::new();
    f.downloads.write_manifest(&manifest("B1"), None).unwrap();
    f.downloads.write_manifest(&manifest("B2"), None).unwrap();
    let removed = f
        .downloads
        .remove_orphaned_manifests(&HashSet::from(["B1".to_string()]));
    assert_eq!(removed, HashSet::from(["B2".to_string()]));
    assert_eq!(f.manifest_files(), ["B1.json"]);
}

#[test]
fn trash_moves_the_audio_and_deletes_the_manifest_only() {
    let f = Fixture::new();
    f.touch("Book [B1].aaxc");
    f.touch("Book [B1].m4b");
    f.downloads.write_manifest(&manifest("B1"), None).unwrap();
    f.downloads.trash_download("B1").unwrap();
    assert_eq!(f.trashed.current(), ["Book [B1].aaxc"]);
    assert!(f.downloads.scan_downloads().is_empty());
    assert!(f.manifest_files().is_empty());
    assert!(f.downloads.folder.join("Book [B1].m4b").exists());
}

#[test]
fn sweep_removes_only_partials() {
    let f = Fixture::new();
    for name in [
        ".Book [B1].1234.partial",
        "Book [B1].partial",
        ".DS_Store",
        "Book [B2].aaxc",
    ] {
        f.touch(name);
    }
    f.downloads.sweep_partials();
    assert_eq!(
        contents_of(&f.downloads.folder),
        [".DS_Store", "Book [B1].partial", "Book [B2].aaxc"]
    );
}

#[test]
fn sweep_discards_legacy_recovery_and_staging_files() {
    let f = Fixture::new();
    f.touch(".B1.download.partial");
    f.touch(".B3.download.partial");
    f.touch_in("B1.partial", &f.downloads.manifest_folder);
    f.touch_in("B2.partial", &f.downloads.manifest_folder);
    f.touch_in(".B4.1234.cover", &f.downloads.manifest_folder);
    f.touch_in(".B4.json.5678.tmp", &f.downloads.manifest_folder);
    // As `tempfile` names a manifest write a crash left behind.
    f.touch_in(".B5.json.aB3xZ9.tmp", &f.downloads.manifest_folder);
    f.downloads.write_manifest(&manifest("B4"), None).unwrap();
    let reloaded = LocalDownloads::new(
        f.downloads.folder.clone(),
        f.downloads.manifest_folder.clone(),
    );
    reloaded.sweep_partials();
    assert!(contents_of(&f.downloads.folder).is_empty());
    assert_eq!(f.manifest_files(), ["B4.json"]);
}

#[test]
fn removing_staging_also_removes_legacy_record() {
    let f = Fixture::new();
    f.touch(".B1.download.partial");
    f.touch_in("B1.partial", &f.downloads.manifest_folder);
    f.downloads.remove_partial_download("B1");
    assert!(contents_of(&f.downloads.folder).is_empty());
    assert!(f.manifest_files().is_empty());
}

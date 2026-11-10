//! Builds the real app: the one place that picks concrete adapters and folders.

use std::path::PathBuf;
use std::sync::Arc;

use directories::{BaseDirs, UserDirs};
use loudible_adapters::{
    AudibleHttpApi, ReqwestDownloader, ReqwestTransport, SystemRandomSource,
    default_credential_store,
};
use loudible_core::{
    AudibleApi, AuthenticationCoordinator, CredentialStore, ExportQueue, LibraryService,
    LocalDownloads, SystemClock,
};
use loudible_ffmpeg::FfmpegConverter;
use loudible_presentation::{AppDependencies, start_app};

use crate::model::LiveApp;
use crate::shell::{app_view::AppView, covers::CoverStore, window::Wake};

/// Not `local.loudible.Loudible`, whose records have another format.
const SUPPORT_FOLDER: &str = "Loudible";

/// Downloads and exports stay where users can see them.
fn downloads_folder() -> Result<PathBuf, String> {
    let directories = UserDirs::new().ok_or("Could not find the home folder.")?;
    let documents = directories
        .document_dir()
        .map(PathBuf::from)
        .unwrap_or_else(|| directories.home_dir().join("Documents"));
    Ok(documents.join("Loudible"))
}

/// Manifests hold key material, so they stay out of Documents (synced, shared).
fn support_folder() -> Result<PathBuf, String> {
    let directories = BaseDirs::new().ok_or("Could not find the home folder.")?;
    Ok(directories.data_dir().join(SUPPORT_FOLDER))
}

pub fn make_app_view(wake: Wake) -> Result<AppView<LiveApp>, String> {
    let text = |error: loudible_core::ExporterError| error.to_string();
    let downloads_folder = downloads_folder()?;
    let support = support_folder()?;
    let clock = Arc::new(SystemClock);
    let transport = Arc::new(ReqwestTransport::new().map_err(text)?);
    let api: Arc<dyn AudibleApi> = Arc::new(AudibleHttpApi::new(
        transport.clone(),
        Arc::new(SystemRandomSource),
        clock.clone(),
    ));
    let store: Arc<dyn CredentialStore> = Arc::new(default_credential_store(
        support.join("session.bin"),
        support.join("master.key"),
    ));
    let auth = AuthenticationCoordinator::new(api.clone(), store, clock);
    let downloads = LocalDownloads::new(downloads_folder.clone(), support.join("Downloads"));
    let queue = ExportQueue::new(
        auth.clone(),
        api.clone(),
        Arc::new(ReqwestDownloader::new().map_err(text)?),
        Arc::new(FfmpegConverter::new().map_err(text)?),
        downloads.clone(),
    );
    let library = LibraryService::new(api, auth.clone(), LibraryService::DEFAULT_PAGE_SIZE);
    let waker = wake.clone();
    let handle = start_app(
        AppDependencies {
            auth,
            library,
            queue,
            downloads,
            export_folder: downloads_folder,
        },
        move || waker(),
    )
    .map_err(|error| error.to_string())?;
    let covers = CoverStore::start(transport, wake).map_err(|error| error.to_string())?;
    Ok(AppView::with_app(LiveApp::new(handle)).with_covers(covers))
}

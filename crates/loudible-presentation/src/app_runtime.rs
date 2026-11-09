use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use loudible_core::{
    Audiobook, AuthenticationCoordinator, BookAction, ExportQueue, LibraryService, LocalDownloads,
    Marketplace,
};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{LocalSet, spawn_local};
use uuid::Uuid;

use crate::{
    AppSnapshot, AppViewModel, Changes, ExportViewModel, LibraryViewModel, SessionViewModel,
};

/// What the models need. Everything here can cross to the model thread.
pub struct AppDependencies {
    pub auth: AuthenticationCoordinator,
    pub library: LibraryService,
    pub queue: ExportQueue,
    pub downloads: LocalDownloads,
    pub export_folder: PathBuf,
}

/// Everything the UI can ask of the models.
#[derive(Debug)]
pub enum Command {
    Refresh,
    Rescan,
    SetSearchText(String),
    ToggleSelection(String),
    SetSelection(Vec<String>),
    SelectAllVisible,
    DeselectAllVisible,
    ClearSelection,
    SetMarketplace(Marketplace),
    SignIn,
    CompleteSignIn(String),
    SetPastedCallbackUrl(String),
    SubmitPastedUrl,
    CancelSignIn,
    SignOut,
    BrowserFailed(String),
    DismissNotice(Uuid),
    DismissExportMessage,
    Perform(BookAction, Box<Audiobook>),
    PerformOnSelection(BookAction),
    CancelAll,
    /// Stops every job, keeping download progress; replies once they have stopped.
    Suspend(oneshot::Sender<()>),
}

/// The UI thread's end of the running models. Dropping it stops them.
pub struct AppHandle {
    commands: mpsc::UnboundedSender<Command>,
    snapshot: Arc<Mutex<Arc<AppSnapshot>>>,
}

impl AppHandle {
    pub fn send(&self, command: Command) {
        // Fails only once the model thread has stopped, when there's nothing left to ask.
        let _ = self.commands.send(command);
    }

    pub fn snapshot(&self) -> Arc<AppSnapshot> {
        self.snapshot
            .lock()
            .expect("no panic while publishing")
            .clone()
    }
}

/// How often at most the window is told to redraw for model changes.
const PUBLISH_INTERVAL: Duration = Duration::from_millis(16);

/// Runs the models on their own thread, single-threaded like the Swift main actor. `wake` is
/// called from that thread after each new snapshot, typically to post an event to the UI loop.
pub fn start_app(
    dependencies: AppDependencies,
    wake: impl Fn() + Send + 'static,
) -> io::Result<AppHandle> {
    let (commands, mut receiver) = mpsc::unbounded_channel();
    let (first_snapshot, snapshot_ready) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    std::thread::Builder::new()
        .name("Loudible.Models".into())
        .spawn(move || {
            let local = LocalSet::new();
            local.block_on(&runtime, async move {
                let changes = Changes::default();
                let app = make_app(dependencies, changes.clone());
                let latest = Arc::new(Mutex::new(Arc::new(AppSnapshot::capture(&app))));
                if first_snapshot.send(latest.clone()).is_err() {
                    return;
                }
                let publisher = app.clone();
                spawn_local(async move {
                    loop {
                        changes.ui_changed().await;
                        let snapshot = Arc::new(AppSnapshot::capture(&publisher));
                        *latest.lock().expect("no panic while publishing") = snapshot;
                        wake();
                        tokio::time::sleep(PUBLISH_INTERVAL).await;
                    }
                });
                let starting = app.clone();
                spawn_local(async move { starting.start().await });
                while let Some(command) = receiver.recv().await {
                    let app = app.clone();
                    spawn_local(async move { perform(&app, command).await });
                }
            });
        })?;
    let snapshot = snapshot_ready
        .recv()
        .map_err(|_| io::Error::other("The model thread stopped while starting."))?;
    Ok(AppHandle { commands, snapshot })
}

fn make_app(dependencies: AppDependencies, changes: Changes) -> AppViewModel {
    AppViewModel::new(
        SessionViewModel::new(dependencies.auth, changes.clone()),
        LibraryViewModel::new(dependencies.library, changes.clone()),
        ExportViewModel::new(
            dependencies.queue,
            dependencies.export_folder,
            changes.clone(),
        ),
        dependencies.downloads,
        changes,
    )
}

async fn perform(app: &AppViewModel, command: Command) {
    let (session, library, export) = (app.session(), app.library(), app.export());
    match command {
        Command::Refresh => app.refresh_library().await,
        Command::Rescan => app.rescan().await,
        Command::SetSearchText(text) => library.set_search_text(text),
        Command::ToggleSelection(asin) => library.toggle_selection(&asin),
        Command::SetSelection(asins) => library.set_selection(asins),
        Command::SelectAllVisible => library.select_all_visible(),
        Command::DeselectAllVisible => library.deselect_all_visible(),
        Command::ClearSelection => library.clear_selection(),
        Command::SetMarketplace(marketplace) => session.set_marketplace(marketplace),
        Command::SignIn => session.sign_in(),
        Command::CompleteSignIn(url) => session.complete_sign_in(&url).await,
        Command::SetPastedCallbackUrl(text) => session.set_pasted_callback_url(text),
        Command::SubmitPastedUrl => session.submit_pasted_url().await,
        Command::CancelSignIn => session.cancel_sign_in(),
        Command::SignOut => app.sign_out().await,
        Command::BrowserFailed(detail) => session.browser_failed(detail),
        Command::DismissNotice(id) => app.errors().dismiss(id),
        Command::DismissExportMessage => export.set_error_message(None),
        Command::Perform(action, book) => app.perform(action, &book).await,
        Command::PerformOnSelection(action) => app.perform_on_selection(action),
        Command::CancelAll => export.cancel_all().await,
        Command::Suspend(done) => {
            export.suspend().await;
            let _ = done.send(());
        }
    }
}

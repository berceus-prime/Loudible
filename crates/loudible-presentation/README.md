# loudible-presentation

The view models of Loudible, ported from the Swift target `LoudiblePresentation`. They hold what
the window shows and turn user actions into work for `loudible-core`. Nothing here draws.

Layering: `loudible-scheduling` → `loudible-core` → `loudible-adapters` / `loudible-ffmpeg` →
**`loudible-presentation`** → the `skia-ui` root crate. This crate depends only on
`loudible-core`; the adapters are picked by the root crate (`src/composition.rs`) and arrive
already built, inside `AppDependencies`. `src/model/live_app.rs` is the only consumer, through
`AppHandle`, `AppSnapshot` and `Command`.

```sh
cargo test -p loudible-presentation
```

## Threading

The Swift models are `@MainActor @Observable`. Here they are `Rc` and `RefCell`, not `Send`, and
live on one thread of their own.

* `start_app` spawns the thread `Loudible.Models`, with a current-thread Tokio runtime and a
  `LocalSet`. All models are built and run there. The UI thread never touches them.
* The UI sends `Command`s over an unbounded channel. Each command runs as its own local task, so
  a long one (a refresh, `SignOut`, `Suspend`) doesn't hold up the next. They interleave at
  `await` points.
* Models call `Changes::mark()` after every state change. `Changes` wraps `tokio::sync::Notify`:
  a signal is kept until awaited, and several marks collapse into one wake-up.
  `mark_session()` also wakes `AppViewModel`'s session observer; it's used when the sign-in
  state changes, or whether jobs wait for authentication.
* A publisher task waits for a change, captures a new `AppSnapshot`, stores it as the latest,
  calls `wake`, then sleeps 16 ms. Changes during the sleep go into the next snapshot, so the
  window is told to redraw at most about 60 times a second.
* `wake` runs on the model thread. It should only post an event to the UI loop.
* `AppSnapshot` is immutable and `Send + Sync`. A snapshot already handed out never changes;
  later commands only show up in later snapshots.
* File work (scans, trashing, sweeping partial downloads) runs in `spawn_blocking`. If it
  panics, the previous state stays.
* Dropping the `AppHandle` closes the command channel, which ends the thread and the tasks on it.

## Using it from a UI

```rust
let handle = start_app(
    AppDependencies { auth, library, queue, downloads, export_folder },
    move || post_redraw_event(),
)?;

handle.send(Command::Refresh);
let snapshot = handle.snapshot(); // Arc<AppSnapshot>, cheap to clone
```

* `start_app` returns once the first snapshot exists (captured before `start()` has restored
  the sign-in), so `snapshot()` always has something. Check `session.has_restored` before
  showing a signed-out state.
* Read state only from snapshots. `snapshot.library.statuses` lines up with
  `snapshot.library.visible_books`.
* `AppSnapshot::menu_items(book)` is worked out on demand, when a menu opens, not for every row.
  Also on the snapshot: `is_in_library`, `marketplace_badge`, `downloaded_file`,
  `exported_file`.
* `Command::Suspend` carries a `oneshot::Sender<()>` and replies once every job has stopped.
  `live_app.rs` polls the receiver with `try_recv` on each tick, before quitting.
* `Command::Perform` runs the queue-backed actions. `ShowDownload` and `ShowExport` do nothing
  here; revealing files and confirming a delete are up to the view.
* `send` ignores a closed channel: that only happens after the model thread has stopped.

## View models

| Type | Responsibility |
| --- | --- |
| `AppViewModel` | Owns the other three and the `ErrorToastQueue`. Signing in loads the library; signing out (not re-authentication) resets it; a new sign-in resumes jobs paused for authentication. A token refresh keeps the device and doesn't reload. Keeps `DiskState` (downloaded, exported and partly downloaded books, manifests) from rescans of the folders, and rescans after each finished job. Continues the signed-in account's unfinished downloads and deletes ones that can't be continued. Resolves per-book status and menus, and runs selection actions. |
| `SessionViewModel` | Mirrors `AuthenticationCoordinator`: restore at start, marketplace, pending login, pasted callback URL, sign in and out, re-authentication. Derives `can_sign_in`, `can_sign_out`, `can_change_marketplace`, `can_submit_pasted_url` and `account_description`. Credential storage failures and browser failures become `AppError`s. |
| `LibraryViewModel` | The purchased library with search and multi-selection. A refresh loads the home marketplace and shows it, then checks every other marketplace and merges what it finds, with a count per store. Stores that fail become `marketplace_notice`, not an error. Downloaded books no longer in the library are appended, sorted by title, while signed in (or while a re-authentication keeps the last library visible). Hidden books are filtered out. |
| `ExportViewModel` | Mirrors `ExportQueue` snapshots: jobs, progress, waiting for authentication. Enqueues downloads and exports, cancels one job or all, suspends. Keeps each book's latest failure until its next attempt, and toasts each failed job once. `has_work_suspend_loses` says whether stopping now would lose an export or a job that hadn't started. |

## Errors and text

* `AppError` covers download, export, enqueue, sign-in browser and credential storage failures.
  Each has a short `title()`, readable `detail()` and the full `technical_error()`.
* `ErrorToastQueue` keeps notices in arrival order until each is dismissed. A `Notice` pairs an
  `AppError` with a `Uuid`; dismissal is by id, so a stale dismiss can't remove a newer notice.
  Session and export models report through callbacks that `AppViewModel` points at the queue.
* `ExportViewModel::error_message` and `LibraryViewModel::error_message` are inline messages,
  not toasts (e.g. "Select at least one book to export.", or a failed home-library load).
* `book_status_text`: `accessibility_text(status)` is the screen-reader value and badge tooltip
  for a `BookStatus` (`None` when there is no badge); `transfer_text(progress)` gives
  "132 MB / 288 MB". Sizes match English `ByteCountFormatter` output: three significant digits,
  decimal megabytes, "Zero KB" for nothing.

## Invariants

* Models are not `Send`. Call them only on the model thread, or in tests inside a `LocalSet`
  (`tests/support::local`).
* Every state change goes through the models' `update` helpers, which call `changes.mark()`.
  A change that skips it won't be published until something else marks.
* No `RefCell` borrow is held across an `await` or while calling out. Error sinks and
  `on_job_finished` are cloned out of the state before being called.
* Long-lived local tasks (session, queue and auth observers, the job-finished rescan) hold `Weak`
  references, so they end with the models.
* `LibraryViewModel` bumps a generation on every refresh and reset. A home-library answer for an
  older generation is dropped, and a reset or new refresh aborts the marketplace check.
* Selection is reconciled with the books only once marketplace checks finish, so a selected
  title from another store survives a refresh.
* `reconcile_partial_downloads` acts only when the library loaded completely and no marketplace
  is being checked, so a failed refresh deletes nothing. It skips books with a job, books that
  failed this session (they wait for Resume Download), and doesn't discard a book missing from
  the library while some store couldn't be checked.
* `rescan` keeps the manifests of books with an unfinished job: a download writes its manifest
  before the file gets its final name.
* `sign_out` suspends jobs first. Downloads keep their bytes and continue after signing in to
  the same account; exports and jobs that hadn't started are dropped.
* `delete_download` trashes the audio file, manifest and any partial download, but keeps the M4B.
* `AppSnapshot::capture` groups jobs by book once, so a large library doesn't cost
  books × jobs on every progress update. `visible_books` is cached until the books or the search
  change.

## Modules

| File | Contents |
| --- | --- |
| `src/lib.rs` | Module list and re-exports. |
| `src/app_runtime.rs` | `AppDependencies`, `Command`, `AppHandle`, `start_app`, the publisher loop. |
| `src/app_snapshot.rs` | `AppSnapshot` with `SessionSnapshot`, `LibrarySnapshot`, `ExportSnapshot`. |
| `src/changes.rs` | `Changes`, the replacement for `@Observable`. |
| `src/app_view_model.rs` | `AppViewModel`, `DiskState`, menu facts. |
| `src/session_view_model.rs` | `SessionViewModel`. |
| `src/library_view_model.rs` | `LibraryViewModel`, `LoadState`, `MarketplaceCount`. |
| `src/export_view_model.rs` | `ExportViewModel`. |
| `src/app_error.rs` | `AppError`, `Notice`, `ErrorToastQueue`. |
| `src/book_status_text.rs` | `accessibility_text`, `transfer_text`. |

## Tests

| File | Covers |
| --- | --- |
| `tests/app_runtime.rs` | The model thread as the UI sees it: snapshots, wake, commands, suspend, dropping the handle. |
| `tests/session_view_model.rs` | Restore, sign-in by browser and by pasted URL, sign-out, re-authentication, credential toasts. |
| `tests/library_view_model.rs` | Loading, search, selection, other marketplaces, local-only books, reset. |
| `tests/export_view_model.rs` | `AppViewModel` with real `ExportQueue`: exports, cancelling, sign-out and resume, rescans, partial downloads, failures. |
| `tests/error_toasts.rs` | `ErrorToastQueue` order and dismissal, `AppError` text. |
| `tests/book_status_text.rs` | Every status's text. |
| `tests/support/mod.rs` | `local`, and `Fixture` over the doubles in `loudible-test-support`. All data is synthetic. |

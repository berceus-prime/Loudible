# loudible-core

The application core: domain models, the
sign-in lifecycle, library rules, the kept downloads and the download/export queue. It does no
HTTP, no keychain and no FFmpeg. Those sit behind the traits in `dependencies.rs`.

## Layering

```
loudible-core → loudible-adapters, loudible-ffmpeg → loudible-presentation → skia-ui
```

* Used by `loudible-adapters`, `loudible-ffmpeg`, `loudible-presentation`, `loudible-test-support`
  and the root crate.
* Tests use the doubles in `loudible-test-support`.

## Ports

Adapters implement these traits. Tests replace them.

| Trait | Does | Notes |
| --- | --- | --- |
| `AudibleApi` | `make_login`, `register`, `refresh`, `library_page`, `license` | `library_page` takes any marketplace; one device registration works in every store. |
| `CredentialStore` | `load`, `save`, `delete` the `AudibleSession` | Errors are plain strings. |
| `FileDownloader` | Downloads a URL to a path | Starts from byte zero and removes failed or cancelled staging bytes. |
| `AudioConverter` | Decrypts and remuxes to M4B at `request.output` | Gets a `ConversionRequest`: input, output, `Decryption`, chapters, `BookMetadata`. |
| `WallClock` | `now()` | `SystemClock` is the real one. |

## Main types

### `AuthenticationCoordinator`

Owns sign-in. States: `SignedOut`, `AwaitingCallback`, `Registering`, `SignedIn`,
`ReauthenticationRequired`.

* `begin_login` makes a `PendingLogin` (device identity, PKCE verifier, login URL, cookies). A new
  login supersedes the old one. Logins expire after 30 minutes.
* `complete_login` checks the callback URL (`https`, `www.amazon.<tld>`, `/ap/maplanding`, exactly
  one `openid.oa2.authorization_code`). A malformed URL leaves the login pending. Once accepted,
  further callbacks are rejected. A failed registration needs a new login. A result that arrives
  after cancel or a newer login is discarded.
* `valid_session` refreshes the token within 5 minutes of expiry. Concurrent callers share one
  refresh, which runs in its own task so one cancelled caller can't abort it.
* `with_valid_session` runs a call; on `AuthenticationRequired` it refreshes and retries once. A
  second rejection moves to `ReauthenticationRequired` and deletes the stored credentials.
* `sign_out` is local only. The device is not deregistered with Audible.
* A failed save or delete keeps the in-memory session and sets a `CredentialStorageFailure`
  warning (`storage_warning`, `storage_warnings`).
* `state_updates` emits the current state, then each change.

### `LibraryService`

Fetches library pages and keeps only purchased audiobooks.

* `fetch_home_library` loads the session's marketplace. Rejections go through
  `with_valid_session`, so they can end the session.
* `fetch_other_marketplaces` checks every other store in `Marketplace::ALL` order, one at a time,
  and streams `MarketplaceScan` (`Titles`, `Empty`, `Failed`). A failing store never ends the
  session or deletes credentials. Stops when the receiver is dropped.
* Paging stops on a short page, on reaching `total_count`, on a page with nothing new, or after
  1000 pages. ASINs are deduplicated; the first copy wins.
* `exclusion` drops Audible Plus titles (`is_ayce`), podcasts and episodes, and
  series/podcast/season containers. `is_hidden` drops titles containing "Morning".
* `merge` appends another store's books without replacing ones already present.
* `search_books`: every whitespace-separated term must match title, subtitle, series, an author,
  a narrator or the ASIN, ignoring case and diacritics.
* `LibrarySelection` is a set of ASINs; `reconcile` drops ones no longer in the library.

### `LocalDownloads`

The kept encrypted downloads. Two folders:

| Folder | Holds |
| --- | --- |
| `folder` | Audio files `Title [ASIN].aax` / `.aaxc`, and hidden `.partial` staging files. |
| `manifest_folder` | `<ASIN>.json` manifests, `<ASIN>.cover` images. Created with mode 0700; files 0600. |

* A book is downloaded when a non-hidden `.aax`/`.aaxc` with `[ASIN]` in its name exists in
  `folder`. The audio file is the source of truth, not the manifest. The same scan finds exports
  (`scan_exports`). With several matches the first in name order wins.
* Manifests are written to a temporary file, then renamed into place.
* `remove_orphaned_manifests` deletes manifests whose ASIN isn't in the given set.
* `trash_download` moves the audio file to the Trash (Recycle Bin on Windows) and deletes its
  manifest and staging bytes. Exports are not touched.
* `sweep_partials` removes leftover staging files and legacy recovery records.

### `ExportQueue`

Schedules download and export jobs. Must run inside a Tokio runtime. Cheap to clone.

* `enqueue(books, kind, destination)` adds one `ExportJob` per book; `start` is the export
  shortcut. A book that already has an unfinished job of that kind is skipped, and so is a
  download of a book with an unfinished export. Returns `StartError` when there is nothing to
  queue or a folder isn't writable.
* `cancel(id)`, `cancel_all`, `suspend` (cancels jobs on quit or sign-out), `clear`, `resume_after_authentication`, `wait_until_finished`.
* `updates()` is a `watch` of `Snapshot` (jobs, `is_running`, `is_waiting_for_authentication`).

Two workflows do the work:

| Workflow | Steps |
| --- | --- |
| `DownloadWorkflow` | Already downloaded: done, no license. Otherwise: license, download to `.<ASIN>.download.partial`, fetch the cover (failure ignored), write the manifest, move to `Title [ASIN].<ext>`, delete staging bytes. |
| `BookExportWorkflow` | Runs `DownloadWorkflow` first. Keys come from the fresh license, else the manifest, else a new license (without downloading again). Converts to a hidden partial in the destination, then moves it to `Title [ASIN].m4b`. |

### Domain models

| Type | Is |
| --- | --- |
| `Audiobook` | A library title, with the `Marketplace` whose library holds it. |
| `LibraryEntry`, `LibraryPage`, `ProductRelationship` | Raw library items before filtering. |
| `Marketplace` | One of ten stores, US first. Serialized as its id only. |
| `AudibleSession`, `DeviceIdentity` | Tokens, customer id, device serial and private key. |
| `PendingLogin`, `LoginCallback` | An in-progress sign-in and the redirect check. |
| `ContentLicense`, `Decryption`, `Chapter` | A license. ADRM with a 4-byte key and no IV is AAX; 16-byte key and IV is AAXC; anything else is unsupported. |
| `DownloadManifest` | Book, keys and chapters kept for offline export. |
| `TransferProgress` | Progress while a download is active. |
| `ExportJob`, `JobKind`, `JobState`, `Phase` | One job and its checked state machine. |
| `BookStatus`, `book_menu_items` | The status shown on a cover and the menu a book offers. |
| `ExporterError` | Every error, with user-facing `Display` text. |

## Behaviour to keep

**Ordering and concurrency.** Download and export jobs run one at a time in FIFO order.
Jobs can be added while another runs. A failed job does not stop the rest. Enqueuing when idle
clears the previous run's finished jobs.

**Job states.** `ExportJob::transition` rejects illegal moves. Finished states (`Completed`,
`Failed`, `Cancelled`) are final. Phases (`Licensing` → `Downloading` → `Converting` →
`Finalizing`) only move forward and progress within a phase never decreases. A job waiting for
authentication can restart from licensing. A job may start in any phase; its `first_phase` is the
origin of `fraction_completed`.

**Re-authentication.** A job needing sign-in holds the only active slot. On
`resume_after_authentication`, it restarts from licensing. Cancelling it admits the next job.

**Incomplete downloads.** Each attempt gets a fresh license and starts at byte zero. Failures,
cancellation and suspension discard staging bytes. Startup removes old staging files and legacy
recovery records.

**File naming** (`export_file_naming`).

* `Title [ASIN].<ext>`, then `Title [ASIN] (2).<ext>` and so on. Finished files are never
  overwritten: the move refuses to clobber and bumps the suffix.
* Titles are NFC-normalized; `:` becomes ` - `; `/ \ ? * " < > |` become `_`; control characters
  become spaces; whitespace is collapsed; leading and trailing dots and spaces are trimmed. An empty
  title becomes `Untitled`. A Windows device name before the first dot gets a `_` prefix.
* Names fit 255 UTF-8 bytes with room for a ` (9999)` suffix.
* The ASIN is read back from the last `[ALNUM]` group, so renamed titles still match.
* Files in progress are hidden: `.<base>.<uuid>.partial` for conversions,
  `.<ASIN>.download.partial` for downloads.

**Key material.** Manifests hold decryption keys, so they live in
`manifest_folder`, outside the user-visible download folder, readable by the owner only.
Binary fields are base64 in the JSON. The session (tokens, device private key) is not written by
this crate; persisting it is `CredentialStore`'s job.

**Conversion failures.** A failed conversion of a kept download adds "delete the download and
download it again" to the message. The partial output is always removed; the download is kept.

**Text.** `text.rs` holds the text rules: grapheme clusters as characters,
case- and diacritic-insensitive matching, and `localizedStandardCompare` ordering
(`standard_compare`).

## Modules

| Module | Contents |
| --- | --- |
| `dependencies` | Port traits, `TransferProgress`, `ConversionRequest`, `BookMetadata`. |
| `authentication_coordinator` | `AuthenticationCoordinator`, `AuthState`. |
| `library` | `LibraryService`, ownership filtering, search, merge, `LibrarySelection`, `MarketplaceScan`. |
| `local_downloads` | `LocalDownloads`: scanning, manifests, Trash, staging cleanup. |
| `export_queue/mod.rs` | `ExportQueue`, `Snapshot`, `StartError`. |
| `export_queue/state.rs` | Snapshot updates and progress events; ignores stale download callbacks. |
| `export_queue/download_workflow.rs` | `DownloadWorkflow`. |
| `export_queue/book_export_workflow.rs` | `BookExportWorkflow`. |
| `export_queue/license_provider.rs` | License through `with_valid_session`. |
| `export_queue/file_finalizer.rs` | No-overwrite move into the first free name. |
| `export_queue/execution_context.rs` | Progress callback and cancellation token for a workflow. |
| `export_job` | `ExportJob` state machine and progress weighting. |
| `export_file_naming` | File names, sanitizing, ASIN tags, partial names. |
| `book_status` | `BookStatus`, `BookAction`, `book_menu_items`. |
| `audiobook`, `marketplace`, `audible_session`, `pending_login`, `content_license`, `download_manifest` | Domain models. |
| `exporter_error` | `ExporterError`. |
| `text` | Text rules. |
| `codec`, `hex_encoding` | Base64 fields in JSON, lowercase hex. |

## Tests

```sh
cargo test -p loudible-core
```

| File | Covers |
| --- | --- |
| `tests/authentication_coordinator.rs` | Sign-in, callbacks, refresh, re-authentication, storage warnings. |
| `tests/library.rs` | Paging, ownership rules, other marketplaces, search, selection. |
| `tests/local_downloads.rs` | Scanning, manifests, orphans, Trash, sweeping, legacy record cleanup. |
| `tests/export_queue.rs` | FIFO ordering, cancellation, auth pause, fresh downloads, naming, workflows. |
| `tests/export_job.rs` | State transitions and progress. |
| `tests/export_file_naming.rs` | Sanitizing, length limits, collisions, ASIN tags. |
| `tests/book_status.rs` | Cover status priority and menu items. |
| `tests/models.rs` | Marketplaces, sessions, logins, hex, DRM selection. |

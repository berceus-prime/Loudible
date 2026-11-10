# loudible-test-support

Test doubles and fixtures shared by the Rust tests. It is only ever a dev-dependency; no shipped crate depends on it.

All data is synthetic. Sessions, tokens, keys, cookies and URLs are made up, and nothing here
belongs to a real account.

It depends on `loudible-core` and implements its traits (`WallClock`, `CredentialStore`,
`AudibleApi`, `FileDownloader`, `AudioConverter`).

## Doubles

| Item | Purpose |
| --- | --- |
| `Locked<T>` | A mutex-protected value for state shared between a test and its doubles. `with` runs a closure on it, `set` replaces it, `current` clones it out. |
| `TestClock` | A `WallClock` that starts at `reference_date()` and moves only through `advance(seconds)`. |
| `InMemoryCredentialStore` | A `CredentialStore` over `stored`. `fail_load`, `fail_save` and `fail_delete` make each call fail; `save_count` counts saves. `with(session)` starts signed in. |
| `MockAudibleApi` | A scriptable `AudibleApi`. See below. |
| `MockDownloader` | A `FileDownloader` that records `downloads`. By default it writes `encrypted` to the destination. Reports progress at 50 and 100 of 100. When the handler fails it removes the file. Replace the handler with `on_download`. |
| `MockConverter` | An `AudioConverter` that records `requests` and by default writes `m4b:<asin>` to the output. Reports progress 0.5 and 1.0. Replace the handler with `on_convert`. |
| `AsyncGate` | Holds callers in `wait()` until `open()`. Clones share the gate. Used to stop a mocked download or conversion mid-flight. |
| `TemporaryDirectory` | A unique directory removed on drop, with `path`, `join` and `contents`. |

### MockAudibleApi

Each call goes to a handler set with `on_make_login`, `on_register`, `on_refresh`, `on_library`
or `on_license`. Handlers return a future, so they can wait on a gate or check cancellation.

Unscripted handlers fail with `ExporterError::Unexpected("unscripted")`, so a test has to script
every call it expects. The exception is `make_login`, which by default returns
`pending_login(marketplace, login_id(0xAA), reference_date())`.

The mock records what it was asked for:

| Field | Contents |
| --- | --- |
| `register_count`, `refresh_count` | Calls to `register` and `refresh`. |
| `requested_pages`, `requested_marketplaces` | Page number and store of each library request. |
| `requested_licenses`, `requested_license_marketplaces` | ASIN and store of each license request. |

`on_library` handlers receive a `LibraryRequest` with the page number, page size, marketplace
and session.

## Helpers

| Item | Purpose |
| --- | --- |
| `wait_until(condition)` | Polls every 2 ms until `condition` holds. Panics after five seconds, naming the caller. |
| `check(&cancel)` | `Err(ExporterError::Cancelled)` once the token is cancelled. |
| `write(path, contents)` | Writes a file, mapping errors to `ExporterError::FileSystem`. |
| `contents_of(directory)` | Sorted names in a directory, hidden files included; empty if it doesn't exist. |

## Fixtures

| Function | Returns |
| --- | --- |
| `reference_date()`, `seconds_after_reference(s)` | A fixed date (Unix time 1,800,000,000) and offsets from it. |
| `device()` | A fixed `DeviceIdentity`. |
| `session()`, `session_in(marketplace)` | A synthetic `AudibleSession`, valid until an hour after the reference date. |
| `session_with_token(token)`, `expired_session()` | The same, with another access token, or already expired. |
| `login_id(n)`, `pending_login(marketplace, id, created_at)` | A `PendingLogin` with a synthetic verifier, URL and cookie. |
| `callback_url(marketplace, code)`, `us_callback()` | A sign-in callback URL carrying an authorization code. |
| `book(asin)`, `book_in(asin, title, marketplace)` | An `Audiobook` with one author and one narrator. |
| `entry(asin)`, `entry_in(asin, marketplace)`, `entry_of(...)` | A `LibraryEntry`; `entry_of` sets delivery type, content type, Plus Catalog and relationships. |
| `license(asin)` | An AAXC `ContentLicense` with one 60-second chapter and a `cdn.example.invalid` URL. |

## Conventions

* Tests state what they use. Script every `MockAudibleApi` call a test relies on; an unscripted
  one fails.
* Synthetic data only. New fixtures must not contain anything from a real account.
* Doubles are shared through `Arc`; read what they recorded with `current()`.

## Usage

```rust
use std::sync::Arc;

use loudible_core::*;
use loudible_test_support::*;

#[tokio::test]
async fn completes_login_and_persists_session() {
    let api = Arc::new(MockAudibleApi::default());
    let store = Arc::new(InMemoryCredentialStore::default());
    api.on_register(|_, _| async { Ok(session()) });
    let auth = AuthenticationCoordinator::new(api.clone(), store.clone(), Arc::new(TestClock::default()));

    auth.begin_login(Marketplace::US).unwrap();
    let result = auth.complete_login(&us_callback()).await.unwrap();

    assert_eq!(store.stored.current(), Some(result));
    assert_eq!(api.register_count.current(), 1);
}
```

Holding a conversion mid-flight:

```rust
let gate = AsyncGate::default();
let held = gate.clone();
converter.on_convert(move |request, cancel| {
    let held = held.clone();
    async move {
        held.wait().await;
        check(&cancel)?;
        write(&request.output, "m4b")
    }
});
// start the export, then
wait_until(|| converter.requests.current().len() == 1).await;
gate.open();
```

## Used by

Dev-dependency of `loudible-core`, `loudible-adapters`, `loudible-ffmpeg`,
`loudible-presentation` and the root `skia-ui` crate (`tests/live_app.rs`).
`loudible-scheduling` does not use it.

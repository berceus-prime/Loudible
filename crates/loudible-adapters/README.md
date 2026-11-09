# loudible-adapters

The real implementations of the ports `loudible-core` defines: the Audible HTTP API, request
signing, sign-in, saved credentials and file downloads. Ported from the Swift `LoudibleAdapters`
target.

It depends only on `loudible-core`. The root crate's `src/composition.rs` picks these adapters;
tests and `--mock` use doubles instead.

```text
loudible-scheduling → loudible-core → loudible-adapters / loudible-ffmpeg → loudible-presentation → skia-ui
```

## Core traits

| Type | Implements |
| --- | --- |
| `AudibleHttpApi` | `AudibleApi` |
| `EncryptedFileCredentialStore` | `CredentialStore` |
| `ReqwestDownloader` | `FileDownloader` |

The crate also defines its own small seams, so tests can replace the network and randomness:

| Trait | Real implementation | Purpose |
| --- | --- | --- |
| `HttpTransport` | `ReqwestTransport` | Sends one `HttpRequest`, returns an `HttpResponse`. `AudibleHttpApi` uses nothing else. |
| `RandomSource` | `SystemRandomSource` | Bytes and UUIDs for sign-in identities. |
| `SecretStore` | `KeyringSecretStore`, `KeyFileSecretStore`, `TimeoutSecretStore` | Holds one secret string. |

## Audible API

`AudibleHttpApi::new(transport, random, clock)` emulates the Audible for iPhone app. Its version
strings and user agents are in `audible_login_factory::app`.

| Method | Request |
| --- | --- |
| `make_login` | None. Builds a `PendingLogin` through `AudibleLoginFactory`. |
| `register` | `POST /auth/register` on the marketplace's Amazon API host, JSON body with the PKCE verifier. |
| `refresh` | `POST /auth/token`, form-encoded, with the refresh token. |
| `library_page` | Signed `GET /1.0/library`, newest purchases first. |
| `license` | Signed `POST /1.0/content/{asin}/licenserequest`, then an unsigned `GET /1.0/content/{asin}/metadata` for chapters. |

* Library and license requests go to the requested marketplace's host, not the session's own.
  Signing does not cover the host, so one device key signs requests to any store.
* A device key that cannot be loaded or used gives `AuthenticationRequired`.
* Registration fails if Audible registered a different device than the one that signed in.
* `refresh` maps 401, and 400/403 with `invalid_grant`, `InvalidToken` or `unauthorized_client`,
  to `AuthenticationRequired`. Other failures become `ExporterError::Http`.

### Request signing

`RequestSigner` adds `x-adp-token`, `x-adp-alg` and `x-adp-signature`. The signature is
RSA PKCS#1 v1.5 SHA-256 over method, path and query, a UTC timestamp with microseconds, the body
and the ADP token. Keys are PEM, either `RSA PRIVATE KEY` (PKCS#1) or `PRIVATE KEY` (PKCS#8);
anything else is a `SigningError`.

### Sign-in

`AudibleLoginFactory::make_login` creates:

* a 40-character alphanumeric device serial, drawn without modulo bias;
* a PKCE pair (`pkce::code_verifier`, `pkce::code_challenge`, RFC 7636, S256);
* the marketplace's `/ap/signin` URL, which returns to `LoginCallback::PATH`;
* the cookies (`frc`, `map-md`, `amzn-app-id`) and web user agent Amazon expects from the app's
  web view.

Query items are percent-encoded the way Audible's signed requests expect; `+` is always escaped.

### Response parsing

| Module | Parses |
| --- | --- |
| `library_response` | Library pages. Items without an ASIN are skipped; a missing title falls back to the ASIN. `Total-Count` gives the total. `check_status` maps errors for all API responses. |
| `license_response` | Licenses. 403/404 and non-`Granted` statuses become `LicenseDenied` with Audible's messages and denial reasons. Only `Adrm` is accepted; anything else is `UnsupportedDrm`. The download URL must be `https`. |
| `license_response::voucher` | The AES-128-CBC voucher holding the content key and IV. Its key and IV are the halves of SHA-256(device type, serial, customer id, ASIN). NUL padding and trailing bytes are tolerated. |
| `json` | `Json`, a lenient read-only view. Missing keys and type mismatches are `None`; numbers and numeric strings both read as integers. |

Chapter trees are flattened in playback order. A parent with its own segment stays a chapter;
zero-length entries are dropped.

Licensing uses the chapters included in the license response and makes no second metadata request.

## Credentials

`EncryptedFileCredentialStore` writes the session as AES-256-GCM encrypted JSON to a file. Only
the random 32-byte key goes into the `SecretStore`, because Windows Credential Manager cannot hold
a whole session.

* The file starts with a format tag, used as associated data, then the nonce and ciphertext.
* Files are written atomically with mode 0600, in a folder created with mode 0700.
* `load` with no session file does not touch the secret store, so a first launch prompts for
  nothing.
* `delete` removes the file first, so signing out holds even if the key cannot be deleted.
* A damaged file, missing key or wrong key is reported as unreadable, not as no session.

`default_credential_store(session_file, key_file)` picks the secret store:

| Condition | Store |
| --- | --- |
| `key_file` exists | `KeyFileSecretStore`: the key in an owner-only file, unencrypted. Used only after `opt_in` creates the file. Signing out empties it, so the opt-in survives. |
| Linux | `KeyringSecretStore` (Secret Service) inside `TimeoutSecretStore`, limit 5 seconds. |
| Otherwise | `KeyringSecretStore` (macOS Keychain, Windows Credential Manager), no limit, since a Keychain prompt waits for the user. |

`TimeoutSecretStore` abandons a hung call on its thread. After one timeout every later call fails
at once.

## HTTP and downloads

`ReqwestTransport` and `ReqwestDownloader` share one client setup:

* rustls with the `ring` provider, installed as the process default if none is. `aws-lc-rs` is
  avoided because it would add CMake and NASM to Windows builds.
* 60-second connect and read timeouts, matching `URLSession`'s stall limit. There is no overall
  request timeout.
* No referer, no cookie store, system proxy settings.
* Error messages drop the URL, because download URLs carry access tokens.

`ReqwestDownloader` downloads from byte zero, sends only the Audible player's user agent,
reports progress at most every 100 ms, and removes the destination on failure or cancellation.
A later attempt gets a fresh URL and starts over.

## Modules

| Module | Contents |
| --- | --- |
| `audible_http_api` | `AudibleHttpApi`, `device_registration`, `token_refresh`, `form_encode`. |
| `audible_login_factory` | `AudibleLoginFactory`, `pkce`, the emulated app's constants. |
| `request_signer` | `RequestSigner`, `SigningError`. |
| `library_response` | Library request, parsing, `check_status`. |
| `license_response` | License request and parsing, chapter flattening, `voucher`. |
| `credentials` | `SecretStore` and its stores, `EncryptedFileCredentialStore`, `default_credential_store`. |
| `http_transport` | `HttpRequest`, `HttpResponse`, `HttpTransport`, `ReqwestTransport`, shared client setup. |
| `reqwest_downloader` | `ReqwestDownloader` for fresh transfers. |
| `random` | `RandomSource`, `SystemRandomSource`. |
| `json` | The lenient `Json` view. |

## Tests

```sh
cargo test -p loudible-adapters
```

| File | Covers |
| --- | --- |
| `tests/authentication.rs` | PKCE, login URLs and cookies, serials, signing against OpenSSL vectors, registration and refresh. |
| `tests/library.rs` | Signed library requests, parsing, error mapping. |
| `tests/license.rs` | Voucher decryption, AAXC and AAX licenses, denials, unsupported DRM. |
| `tests/chapter_correction.rs` | License chapters with exactly one HTTP request. |
| `tests/credentials.rs` | Encryption at rest, file permissions, timeouts, key file opt-in. Uses an in-memory secret store, never the real keyring. |
| `tests/download_recovery.rs` | Fresh transfer and cleanup against a loopback HTTP server. |
| `tests/support/mod.rs` | `MockTransport`, a predictable `RandomSource`, fixture helpers. |

Nothing contacts Audible. All fixtures in `tests/fixtures/` are synthetic, including the device
keys, signing vectors and voucher vector; none come from a real account. The tests also use the
shared doubles in `loudible-test-support`.

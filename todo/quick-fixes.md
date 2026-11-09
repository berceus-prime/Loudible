# Quick fixes

These are localized findings from the review of `crates/`. The redirect issue is the most urgent of the three despite its small implementation scope.

## Signed headers can follow a redirect to another host

**Context:** `crates/loudible-adapters/src/request_signer.rs` adds `x-adp-token` and `x-adp-signature` to API requests. `ReqwestTransport` in `crates/loudible-adapters/src/http_transport.rs` uses reqwest's default redirect policy.

**Bug:** A cross-host redirect can forward these custom headers to the destination host, exposing the device token. Standard authorization-header stripping does not cover `x-adp-token`.

**Done when:** Cross-host redirects cannot receive signed credentials. Cover the redirect behavior with a regression test.

## Missing book fields leave old FFmpeg tags

**Context:** `metadata_tags` in `crates/loudible-ffmpeg/src/ffmpeg_converter.rs` omits empty authors, narrators, subtitle, publisher, and release date. `apply_metadata` in `crates/loudible-ffmpeg/bridge/ffmpeg_bridge.c` copies source tags before applying replacements; an empty replacement removes a tag.

**Bug:** An exported M4B can retain incorrect source `artist`, `album_artist`, `composer`, `description`, `copyright`, or `date` when the corresponding book field is absent.

**Done when:** Absent book fields clear the corresponding inherited tags. Update the metadata test and verify the tags in a remuxed file.

## Test helper hides directory read errors

**Context:** `contents_of` in `crates/loudible-test-support/src/lib.rs` returns sorted directory entries and an empty list for a missing directory. It currently uses `unwrap_or_default()` on `read_dir` and flattens entry errors.

**Bug:** A directory that exists but cannot be read appears empty, so cleanup assertions can pass while files remain.

**Done when:** Only a genuinely missing directory counts as empty; other read and entry errors fail the test clearly.

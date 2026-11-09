# Validate ASINs before using them in paths

**Context:** `crates/loudible-adapters/src/library_response.rs` accepts any nonempty ASIN from the library response. `LocalDownloads` in `crates/loudible-core/src/local_downloads.rs` interpolates ASINs into manifest, cover, and partial-record paths. `crates/loudible-core/src/export_file_naming.rs` sanitizes an ASIN for output names but assumes it fits the filename budget.

**Problems:** An ASIN containing `../` can make manifest or cleanup operations reach outside the manifest folder. An excessively long accepted ASIN can underflow the filename budget and panic a job, potentially leaving its scheduler reservation uncleared.

**Done when:** Incoming ASINs have a bounded, path-safe representation before any file operation. Existing on-disk records and filenames are handled deliberately. Test traversal, long values, and valid ASINs through the affected paths.

# Keep manifests for downloads started during a rescan

**Context:** `AppViewModel::rescan` in `crates/loudible-presentation/src/app_view_model.rs` snapshots busy ASINs before moving a disk scan to a blocking thread. The download workflow in `crates/loudible-core/src/export_queue/download_workflow.rs` writes a manifest before moving the audio file to its final name. `LocalDownloads::remove_orphaned_manifests` removes manifests absent from the scan's keep set.

**Problem:** A download can start after the busy-ASIN snapshot, write its manifest, and still be moving its audio file while the scan runs. The scan then treats that manifest as orphaned and deletes it. The completed audio lacks the manifest needed for export.

**Done when:** Cleanup and download finalization agree on which ASINs are active at deletion time. Test the interleaving in which a download starts after rescan begins and writes its manifest before its final audio file appears.

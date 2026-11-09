# Keep valid download recovery state across restarts and retries

**Context:** `DownloadWorkflow::prepare_recovery` in `crates/loudible-core/src/export_queue/download_workflow.rs` reads a compatible saved resume token. `ReqwestDownloader::download` in `crates/loudible-adapters/src/reqwest_downloader.rs` resumes from that token and may retry from byte zero.

**Problems:** `prepare_recovery` overwrites the saved record with default recovery before the downloader starts. A process exit in that window loses resumable progress. Separately, a network error during a valid resumed transfer causes the downloader to delete the partial file and retry from zero, even when the transfer has yielded a usable updated token. If the fresh retry also fails, accumulated bytes are lost.

**Done when:** Compatible recovery data survives until it is safely superseded. Retry from zero only when the saved position or URL is unusable. Exercise process interruption and transient network failure at each recovery transition.

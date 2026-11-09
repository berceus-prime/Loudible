use loudible_core::*;
use loudible_presentation::*;

#[test]
fn describes_every_status() {
    let text = |status: BookStatus| accessibility_text(&status);
    let kept = TransferProgress::new(132_000_000, 288_000_000);
    assert_eq!(text(BookStatus::None), None);
    assert_eq!(
        text(BookStatus::Queued(JobKind::Download)).unwrap(),
        "Queued for download"
    );
    assert_eq!(
        text(BookStatus::Queued(JobKind::Export)).unwrap(),
        "Queued for export"
    );
    assert_eq!(
        text(BookStatus::Running {
            kind: JobKind::Download,
            progress: 0.459,
            transferred: Some(kept)
        })
        .unwrap(),
        "Downloading, 45 percent, 132 MB of 288 MB"
    );
    assert_eq!(
        text(BookStatus::Running {
            kind: JobKind::Export,
            progress: 1.0,
            transferred: None
        })
        .unwrap(),
        "Exporting, 100 percent"
    );
    assert_eq!(
        text(BookStatus::Failed {
            kind: JobKind::Export,
            message: "disk full".into(),
            kept: None
        })
        .unwrap(),
        "Export failed: disk full"
    );
    assert_eq!(
        text(BookStatus::Failed {
            kind: JobKind::Download,
            message: "Download failed: HTTP 500".into(),
            kept: Some(kept)
        })
        .unwrap(),
        "Download failed: HTTP 500, 132 MB of 288 MB downloaded"
    );
    assert_eq!(
        text(BookStatus::Paused(kept)).unwrap(),
        "Download paused, 132 MB of 288 MB"
    );
    assert_eq!(text(BookStatus::Downloaded).unwrap(), "Downloaded");
    assert_eq!(text(BookStatus::Exported).unwrap(), "Exported to M4B");
    assert_eq!(transfer_text(&kept), "132 MB / 288 MB");
}

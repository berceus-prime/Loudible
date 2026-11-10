//! Error toasts. The web-view case belongs to the shell.

use loudible_core::JobKind;
use loudible_presentation::*;

#[test]
fn queued_errors_keep_full_diagnostics_and_dismiss_by_identity() {
    let queue = ErrorToastQueue::default();
    let diagnostic = format!(
        "NSURLErrorDomain -1009\nUnderlying error: offline\n{}",
        "details ".repeat(200)
    );
    queue.present(AppError::Download {
        book_title: "Tea".into(),
        technical_error: diagnostic.clone(),
    });
    queue.present(AppError::Export {
        book_title: "Coffee".into(),
        technical_error: "Conversion failed".into(),
    });
    let first = queue.notices()[0].clone();
    assert_eq!(first.error.title(), "Download failed");
    assert!(first.error.detail().contains("Tea"));
    assert_eq!(first.error.technical_error(), diagnostic);
    queue.dismiss(first.id);
    assert_eq!(queue.notices()[0].error.title(), "Export failed");
    queue.dismiss(first.id);
    assert_eq!(queue.notices().len(), 1);
}

#[test]
fn batch_failure_summarizes_the_selection() {
    let error = AppError::Enqueue {
        kind: JobKind::Download,
        book_count: 1000,
        technical_error: "Folder is not writable".into(),
    };
    assert!(error.detail().contains("1000 selected books"));
    assert!(error.detail().chars().count() < 200);
    assert_eq!(error.technical_error(), "Folder is not writable");
}

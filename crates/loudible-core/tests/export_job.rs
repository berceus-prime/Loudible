use std::path::PathBuf;

use loudible_core::JobState::*;
use loudible_core::Phase::*;
use loudible_core::*;
use loudible_test_support::*;

fn job() -> ExportJob {
    ExportJob::new(book("B000000001"), JobKind::Export)
}

fn output() -> PathBuf {
    PathBuf::from("/tmp/out.m4b")
}

#[test]
fn happy_path_advances_through_phases() {
    let mut job = job();
    assert_eq!(*job.state(), Queued);
    for state in [
        Running(Licensing, 0.0),
        Running(Downloading, 0.0),
        Running(Downloading, 0.5),
        Running(Converting, 0.2),
        Running(Finalizing, 0.0),
    ] {
        job.transition(state).unwrap();
    }
    job.transition(Completed(output())).unwrap();
    assert_eq!(*job.state(), Completed(output()));
    assert_eq!(job.fraction_completed(), 1.0);
}

#[test]
fn phases_and_progress_never_go_backwards() {
    let mut job = job();
    job.transition(Running(Licensing, 0.0)).unwrap();
    job.transition(Running(Converting, 0.5)).unwrap();
    assert!(job.transition(Running(Downloading, 1.0)).is_err());
    assert!(job.transition(Running(Converting, 0.4)).is_err());
}

#[test]
fn finished_states_are_final() {
    for finished in [
        Completed(PathBuf::from("/x")),
        Failed("e".into()),
        Cancelled,
    ] {
        let mut job = job();
        job.transition(Running(Licensing, 0.0)).unwrap();
        job.transition(finished).unwrap();
        assert!(job.state().is_finished());
        assert!(job.transition(Queued).is_err());
        assert!(job.transition(Running(Licensing, 0.0)).is_err());
        assert!(job.transition(Cancelled).is_err());
    }
}

#[test]
fn authentication_pause_restarts_from_licensing() {
    let mut job = job();
    job.transition(Running(Licensing, 0.0)).unwrap();
    job.transition(Running(Downloading, 0.3)).unwrap();
    job.transition(WaitingForAuthentication).unwrap();
    assert!(job.transition(Running(Downloading, 0.0)).is_err());
    job.transition(Running(Licensing, 0.0)).unwrap();
    assert_eq!(*job.state(), Running(Licensing, 0.0));
}

#[test]
fn byte_count_is_kept_only_while_downloading() {
    let mut job = job();
    let bytes = TransferProgress::new(132, 288);
    job.transition_with(Running(Licensing, 0.0), Some(bytes))
        .unwrap();
    assert_eq!(job.transferred(), None);
    job.transition_with(Running(Downloading, bytes.fraction()), Some(bytes))
        .unwrap();
    assert_eq!(job.transferred(), Some(bytes));
    job.transition(Running(Converting, 0.0)).unwrap();
    assert_eq!(job.transferred(), None);
}

#[test]
fn queued_job_can_start_in_any_phase_but_not_complete() {
    let mut job = job();
    assert!(job.transition(Completed(PathBuf::from("/x"))).is_err());
    // An export of a kept download starts at converting.
    job.transition(Running(Converting, 0.0)).unwrap();
    assert_eq!(job.first_phase(), Some(Converting));
    assert_eq!(job.fraction_completed(), 0.0);
    job.transition(Running(Converting, 0.5)).unwrap();
    assert!((job.fraction_completed() - 0.475).abs() < 0.0001); // 0.38 * 0.5 of the remaining 0.40
}

#[test]
fn queued_job_can_be_cancelled_or_paused() {
    let mut cancelled = job();
    cancelled.transition(Cancelled).unwrap();
    let mut paused = job();
    paused.transition(WaitingForAuthentication).unwrap();
    paused.transition(Cancelled).unwrap();
    assert_eq!(*paused.state(), Cancelled);
}

#[test]
fn fraction_completed_is_monotonic_across_phases() {
    let mut job = job();
    let mut last = job.fraction_completed();
    for state in [
        Running(Licensing, 0.0),
        Running(Licensing, 1.0),
        Running(Downloading, 0.5),
        Running(Downloading, 1.0),
        Running(Converting, 0.5),
        Running(Finalizing, 1.0),
    ] {
        job.transition(state).unwrap();
        assert!(job.fraction_completed() >= last);
        last = job.fraction_completed();
    }
    assert!(last <= 1.0);
}

#[test]
fn download_weights_downloading_most() {
    let mut job = ExportJob::new(book("B000000001"), JobKind::Download);
    job.transition(Running(Licensing, 1.0)).unwrap();
    assert!((job.fraction_completed() - 0.02).abs() < 0.0001);
    job.transition(Running(Downloading, 0.5)).unwrap();
    assert!((job.fraction_completed() - 0.50).abs() < 0.0001);
    job.transition(Running(Finalizing, 0.0)).unwrap();
    assert!((job.fraction_completed() - 0.98).abs() < 0.0001);
}

#[test]
fn jobs_of_one_book_have_distinct_ids() {
    let book = book("B000000001");
    assert_ne!(
        ExportJob::new(book.clone(), JobKind::Download).id,
        ExportJob::new(book, JobKind::Export).id
    );
}

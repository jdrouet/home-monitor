use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use domain::{ImportStatus, Timestamp, import_report};
use garmin_connect::GarminConnect;
use state_file::{ActivityCursorFile, BackfillFile};
use tokio::sync::watch;
use use_cases::ImportCursors;

/// The recorded job status joined with the progress read from the cursors.
pub struct ImportView {
    pub jobs: watch::Receiver<ImportStatus>,
    pub cursors: ImportCursors<BackfillFile, ActivityCursorFile>,
}

impl ImportView {
    /// The encoded report at `now`. The next pull is the one the pull loop
    /// recorded, never recomputed here.
    pub fn report(&self, now: Timestamp) -> Vec<u8> {
        let jobs = self.jobs.borrow().clone();
        let status = self.cursors.status::<GarminConnect>(jobs, now);
        import_report::encode(&status)
    }
}

pub fn router(view: ImportView) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/imports", get(imports))
        .with_state(Arc::new(view))
}

async fn imports(State(view): State<Arc<ImportView>>) -> Response {
    let body = view.report(Timestamp::now());
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::http::{StatusCode, header};
    use domain::{
        AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportEta, ImportFailure,
        ImportJob, ImportProgress, ImportStatus, Timestamp, import_report,
    };
    use state_file::{ActivityCursorFile, BackfillFile};
    use tokio::sync::watch;
    use use_cases::ImportCursors;

    use super::{ImportView, imports};

    fn missing(name: &str) -> PathBuf {
        PathBuf::from("/nonexistent-health-collector-dir").join(name)
    }

    fn view(status: ImportStatus) -> (watch::Sender<ImportStatus>, ImportView) {
        let (sender, receiver) = watch::channel(status);
        let view = ImportView {
            jobs: receiver,
            cursors: ImportCursors {
                gap: BackfillFile::new(missing("gap.json")),
                yearly: BackfillFile::new(missing("yearly.json")),
                activities: ActivityCursorFile::new(missing("activities.json")),
                archive: BackfillFile::new(missing("archive.json")),
            },
        };
        (sender, view)
    }

    fn recorded() -> ImportStatus {
        let mut status = ImportStatus {
            next_pull: Some(Timestamp::from_micros(100)),
            ..ImportStatus::default()
        };
        let attempt = ImportAttempt {
            started: Timestamp::from_micros(1),
            ended: Some(AttemptEnd {
                at: Timestamp::from_micros(2),
                outcome: AttemptOutcome::Failed(ImportFailure::GarminDown),
                counts: ImportCounts::default(),
            }),
        };
        status.record(
            ImportJob::Workouts,
            attempt,
            Timestamp::from_micros(100),
            None,
        );
        status
    }

    #[test]
    fn the_report_holds_the_recorded_job_and_the_next_pull() {
        let (_sender, view) = view(recorded());
        let status = import_report::decode(&view.report(Timestamp::from_micros(50)))
            .expect("a valid report");
        assert_eq!(status.next_pull, Some(Timestamp::from_micros(100)));
        let workouts = &status.jobs[&ImportJob::Workouts];
        assert_eq!(workouts.retry_at, Some(Timestamp::from_micros(100)));
        assert_eq!(workouts.attempts.len(), 1);
    }

    #[test]
    fn missing_cursors_give_no_progress_and_an_unknown_finish_for_a_running_job() {
        let mut status = ImportStatus {
            next_pull: Some(Timestamp::from_micros(100)),
            ..ImportStatus::default()
        };
        let running = ImportAttempt {
            started: Timestamp::from_micros(1),
            ended: None,
        };
        status.record(
            ImportJob::HealthHistory,
            running,
            Timestamp::from_micros(100),
            None,
        );
        let (_sender, view) = view(status);
        let report = import_report::decode(&view.report(Timestamp::from_micros(50)))
            .expect("a valid report");
        let job = &report.jobs[&ImportJob::HealthHistory];
        assert_eq!(job.progress, None::<ImportProgress>);
        assert_eq!(job.eta, Some(ImportEta::Unknown));
    }

    #[test]
    fn a_new_record_shows_in_the_next_report() {
        let (sender, view) = view(ImportStatus::default());
        let before = import_report::decode(&view.report(Timestamp::from_micros(0))).expect("ok");
        assert!(before.jobs.is_empty() || before.jobs.values().all(|job| job.attempts.is_empty()));
        sender.send_replace(recorded());
        let after = import_report::decode(&view.report(Timestamp::from_micros(0))).expect("ok");
        assert_eq!(after.jobs[&ImportJob::Workouts].attempts.len(), 1);
    }

    #[tokio::test]
    async fn the_endpoint_answers_json_with_the_report() {
        let (_sender, view) = view(recorded());
        let response = imports(State(std::sync::Arc::new(view))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let status = import_report::decode(&body).expect("a valid report");
        assert_eq!(status.next_pull, Some(Timestamp::from_micros(100)));
    }
}

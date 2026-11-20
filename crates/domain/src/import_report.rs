use serde::{Deserialize, Serialize};

use crate::{ImportReportError, ImportStatus};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct WireOut<'a> {
    version: u32,
    #[serde(flatten)]
    status: &'a ImportStatus,
}

#[derive(Deserialize)]
struct WireIn {
    version: u32,
    #[serde(flatten)]
    status: ImportStatus,
}

pub fn encode(status: &ImportStatus) -> Vec<u8> {
    serde_json::to_vec(&WireOut {
        version: SCHEMA_VERSION,
        status,
    })
    .expect("an import report always serializes")
}

pub fn decode(payload: &[u8]) -> Result<ImportStatus, ImportReportError> {
    let wire: WireIn = serde_json::from_slice(payload)?;
    if wire.version != SCHEMA_VERSION {
        return Err(ImportReportError::UnsupportedVersion(wire.version));
    }
    Ok(wire.status)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::NaiveDate;
    use serde::Deserialize;

    use crate::{
        AttemptEnd, AttemptOutcome, ImportAttempt, ImportCounts, ImportEta, ImportFailure,
        ImportJob, ImportJobStatus, ImportProgress, ImportReportError, ImportStatus, Timestamp,
    };

    use super::{decode, encode};

    fn at(micros: i64) -> Timestamp {
        Timestamp::from_micros(micros)
    }

    fn sample() -> ImportStatus {
        let mut status = ImportStatus {
            next_pull: Some(at(100)),
            ..ImportStatus::default()
        };
        status.jobs.insert(
            ImportJob::DailyPull,
            ImportJobStatus {
                attempts: vec![ImportAttempt {
                    started: at(1),
                    ended: Some(AttemptEnd {
                        at: at(2),
                        outcome: AttemptOutcome::Done,
                        counts: ImportCounts {
                            stored: 2,
                            ..ImportCounts::default()
                        },
                    }),
                }],
                ..ImportJobStatus::default()
            },
        );
        status.jobs.insert(
            ImportJob::Workouts,
            ImportJobStatus {
                attempts: vec![ImportAttempt {
                    started: at(3),
                    ended: Some(AttemptEnd {
                        at: at(4),
                        outcome: AttemptOutcome::Failed(ImportFailure::GarminDown),
                        counts: ImportCounts {
                            stored: 5,
                            failed: 1,
                            ..ImportCounts::default()
                        },
                    }),
                }],
                retry_at: Some(at(100)),
                next_retry: None,
                progress: Some(ImportProgress::Workouts { complete: false }),
                eta: Some(ImportEta::Unknown),
            },
        );
        status.jobs.insert(
            ImportJob::MissedDays,
            ImportJobStatus {
                eta: Some(ImportEta::ImportDays { days: 1 }),
                ..ImportJobStatus::default()
            },
        );
        status.jobs.insert(
            ImportJob::HealthHistory,
            ImportJobStatus {
                attempts: vec![ImportAttempt {
                    started: at(5),
                    ended: None,
                }],
                progress: Some(ImportProgress::Days {
                    done_through: NaiveDate::from_ymd_opt(2026, 3, 10).unwrap(),
                    left: 245,
                }),
                eta: Some(ImportEta::At {
                    at: at(15_190_000_000),
                }),
                ..ImportJobStatus::default()
            },
        );
        status.jobs.insert(
            ImportJob::DailyArchive,
            ImportJobStatus {
                eta: Some(ImportEta::WaitsFor {
                    job: ImportJob::HealthHistory,
                }),
                ..ImportJobStatus::default()
            },
        );
        status
    }

    const GOLDEN: &str = concat!(
        r#"{"version":1,"next_pull":100,"jobs":{"#,
        r#""daily_pull":{"attempts":[{"started":1,"ended":{"at":2,"outcome":"done","counts":{"stored":2,"empty":0,"failed":0}}}]},"#,
        r#""missed_days":{"attempts":[],"eta":{"kind":"import_days","days":1}},"#,
        r#""health_history":{"attempts":[{"started":5}],"progress":{"kind":"days","done_through":"2026-03-10","left":245},"eta":{"kind":"at","at":15190000000}},"#,
        r#""workouts":{"attempts":[{"started":3,"ended":{"at":4,"outcome":{"failed":"garmin_down"},"counts":{"stored":5,"empty":0,"failed":1}}}],"retry_at":100,"progress":{"kind":"workouts","complete":false},"eta":{"kind":"unknown"}},"#,
        r#""daily_archive":{"attempts":[],"eta":{"kind":"waits_for","job":"health_history"}}}}"#,
    );

    #[test]
    fn encoding_matches_the_wire_format() {
        assert_eq!(String::from_utf8(encode(&sample())).unwrap(), GOLDEN);
    }

    #[test]
    fn the_golden_payload_decodes_with_no_next_retry() {
        let status = decode(GOLDEN.as_bytes()).expect("decodes");
        assert_eq!(status, sample());
        assert!(status.jobs.values().all(|job| job.next_retry.is_none()));
    }

    #[test]
    fn a_planned_retry_is_on_the_wire_and_round_trips() {
        let mut status = sample();
        let job = status.jobs.get_mut(&ImportJob::Workouts).unwrap();
        job.retry_at = None;
        job.next_retry = Some(at(50));
        let payload = encode(&status);
        assert!(
            String::from_utf8(payload.clone())
                .unwrap()
                .contains(r#""next_retry":50"#)
        );
        assert_eq!(decode(&payload).expect("decodes"), status);
    }

    /// The reader shape before `next_retry` existed: it must still accept
    /// reports that carry a planned retry.
    #[derive(Deserialize)]
    struct OldReader {
        jobs: BTreeMap<ImportJob, OldJob>,
    }

    #[derive(Deserialize)]
    struct OldJob {
        attempts: Vec<ImportAttempt>,
        #[serde(default)]
        retry_at: Option<Timestamp>,
    }

    #[test]
    fn a_reader_without_next_retry_accepts_a_pending_retry() {
        let mut status = ImportStatus::default();
        let failed = ImportAttempt {
            started: at(1),
            ended: Some(AttemptEnd {
                at: at(2),
                outcome: AttemptOutcome::Failed(ImportFailure::GarminDown),
                counts: ImportCounts::default(),
            }),
        };
        status.record(ImportJob::Workouts, failed, at(100), Some(at(50)));
        let old: OldReader = serde_json::from_slice(&encode(&status)).expect("old reader accepts");
        let job = &old.jobs[&ImportJob::Workouts];
        assert_eq!(job.attempts, vec![failed]);
        assert_eq!(job.retry_at, None);

        status.record(ImportJob::Workouts, failed, at(100), None);
        let old: OldReader = serde_json::from_slice(&encode(&status)).expect("old reader accepts");
        assert_eq!(old.jobs[&ImportJob::Workouts].retry_at, Some(at(100)));
    }

    #[test]
    fn round_trips() {
        assert_eq!(decode(&encode(&sample())).expect("decodes"), sample());
    }

    #[test]
    fn job_map_keys_round_trip() {
        let payload = br#"{"version":1,"jobs":{"daily_pull":{"attempts":[]}}}"#;
        let status = decode(payload).expect("decodes");
        assert!(status.jobs.contains_key(&ImportJob::DailyPull));
        assert_eq!(status.next_pull, None);
    }

    #[test]
    fn other_versions_are_rejected() {
        assert!(matches!(
            decode(br#"{"version":2,"jobs":{}}"#),
            Err(ImportReportError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn garbage_is_a_json_error() {
        assert!(matches!(
            decode(b"not json"),
            Err(ImportReportError::Json(_))
        ));
    }
}

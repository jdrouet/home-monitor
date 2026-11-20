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
    use chrono::NaiveDate;

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
                progress: Some(ImportProgress::Workouts { complete: false }),
                eta: Some(ImportEta::Unknown),
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
                eta: Some(ImportEta::ImportDays { days: 2 }),
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

    #[test]
    fn encoding_matches_the_wire_format() {
        let expected = concat!(
            r#"{"version":1,"next_pull":100,"jobs":{"#,
            r#""daily_pull":{"attempts":[{"started":1,"ended":{"at":2,"outcome":"done","counts":{"stored":2}}}]},"#,
            r#""health_history":{"attempts":[{"started":5}],"progress":{"kind":"days","done_through":"2026-03-10","left":245},"eta":{"kind":"import_days","days":2}},"#,
            r#""workouts":{"attempts":[{"started":3,"ended":{"at":4,"outcome":{"failed":"garmin_down"},"counts":{"stored":5,"failed":1}}}],"retry_at":100,"progress":{"kind":"workouts","complete":false},"eta":{"kind":"unknown"}},"#,
            r#""daily_archive":{"attempts":[],"eta":{"kind":"waits_for","job":"health_history"}}}}"#,
        );
        assert_eq!(String::from_utf8(encode(&sample())).unwrap(), expected);
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

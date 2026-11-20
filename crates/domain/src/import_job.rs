use serde::{Deserialize, Serialize};

/// A Garmin import, in the order the chain runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportJob {
    DailyPull,
    MissedDays,
    HealthHistory,
    Workouts,
    DailyArchive,
}

impl ImportJob {
    pub const ALL: [Self; 5] = [
        Self::DailyPull,
        Self::MissedDays,
        Self::HealthHistory,
        Self::Workouts,
        Self::DailyArchive,
    ];
}

#[cfg(test)]
mod tests {
    use super::ImportJob;

    #[test]
    fn all_lists_every_job_in_run_order() {
        let mut sorted = ImportJob::ALL;
        sorted.sort();
        assert_eq!(sorted, ImportJob::ALL);
        assert_eq!(ImportJob::ALL[0], ImportJob::DailyPull);
    }
}

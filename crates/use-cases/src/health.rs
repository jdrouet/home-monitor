use chrono::NaiveDate;
use domain::{Measurement, Metric, SourceKind};

use crate::CalendarDay;

/// Metrics the health page shows, one store query each.
pub const HEALTH_METRICS: [Metric; 6] = [
    Metric::SleepDuration,
    Metric::SleepScore,
    Metric::RestingHeartRate,
    Metric::Steps,
    Metric::BodyBatteryMax,
    Metric::BodyBatteryMin,
];

/// One day of the health table; a metric with no reading that day is `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthRow {
    pub date: NaiveDate,
    pub sleep_seconds: Option<f64>,
    pub sleep_score: Option<f64>,
    pub resting_heart_rate: Option<f64>,
    pub steps: Option<f64>,
    pub body_battery_max: Option<f64>,
    pub body_battery_min: Option<f64>,
}

/// One row per day, in the order of `days`. Per day and metric the latest
/// health reading inside the day wins: a completed day's final value replaces
/// the partial ones pulled earlier.
pub fn build_health(days: &[CalendarDay], measurements: &[Measurement]) -> Vec<HealthRow> {
    days.iter()
        .map(|day| {
            let latest = |metric| latest_reading(day, measurements, metric);
            HealthRow {
                date: day.date,
                sleep_seconds: latest(Metric::SleepDuration),
                sleep_score: latest(Metric::SleepScore),
                resting_heart_rate: latest(Metric::RestingHeartRate),
                steps: latest(Metric::Steps),
                body_battery_max: latest(Metric::BodyBatteryMax),
                body_battery_min: latest(Metric::BodyBatteryMin),
            }
        })
        .collect()
}

fn latest_reading(day: &CalendarDay, measurements: &[Measurement], metric: Metric) -> Option<f64> {
    measurements
        .iter()
        .filter(|reading| {
            reading.source_kind == SourceKind::Health
                && reading.metric == metric
                && (day.start..day.end).contains(&reading.timestamp)
        })
        .max_by_key(|reading| reading.timestamp)
        .map(|reading| reading.value)
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use domain::{Measurement, Metric, SourceId, SourceKind, Timestamp};

    use crate::CalendarDay;

    use super::build_health;

    fn day(day: u32, start: i64, end: i64) -> CalendarDay {
        CalendarDay {
            date: NaiveDate::from_ymd_opt(2026, 7, day).expect("valid date"),
            start: Timestamp::from_micros(start),
            end: Timestamp::from_micros(end),
        }
    }

    fn reading(kind: SourceKind, metric: Metric, at: i64, value: f64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(at),
            source_kind: kind,
            source_id: SourceId::from("garmin".to_owned()),
            metric,
            value,
        }
    }

    fn steps(at: i64, value: f64) -> Measurement {
        reading(SourceKind::Health, Metric::Steps, at, value)
    }

    #[test]
    fn a_completed_days_final_reading_beats_its_partial_ones() {
        let days = [day(14, 0, 100)];
        let readings = [steps(40, 3000.0), steps(99, 8431.0), steps(10, 500.0)];
        let rows = build_health(&days, &readings);
        assert_eq!(rows[0].steps, Some(8431.0));
    }

    #[test]
    fn todays_latest_partial_reading_is_shown() {
        let days = [day(15, 100, 200)];
        let readings = [steps(120, 1000.0), steps(150, 2500.0)];
        assert_eq!(build_health(&days, &readings)[0].steps, Some(2500.0));
    }

    #[test]
    fn readings_land_in_their_own_day_only() {
        let days = [day(15, 100, 200), day(14, 0, 100)];
        let readings = [steps(99, 8431.0), steps(100, 10.0), steps(200, 99.0)];
        let rows = build_health(&days, &readings);
        assert_eq!(rows[0].steps, Some(10.0));
        assert_eq!(rows[1].steps, Some(8431.0));
        assert_eq!(rows[0].date, days[0].date);
    }

    #[test]
    fn other_kinds_are_ignored() {
        let days = [day(14, 0, 100)];
        let readings = [reading(SourceKind::Ble, Metric::Steps, 50, 7.0)];
        assert_eq!(build_health(&days, &readings)[0].steps, None);
    }

    #[test]
    fn every_metric_fills_its_own_column() {
        let days = [day(14, 0, 100)];
        let readings = [
            (Metric::SleepDuration, 1.0),
            (Metric::SleepScore, 2.0),
            (Metric::RestingHeartRate, 3.0),
            (Metric::Steps, 4.0),
            (Metric::BodyBatteryMax, 5.0),
            (Metric::BodyBatteryMin, 6.0),
        ]
        .map(|(metric, value)| reading(SourceKind::Health, metric, 10, value));
        let row = &build_health(&days, &readings)[0];
        assert_eq!(
            [
                row.sleep_seconds,
                row.sleep_score,
                row.resting_heart_rate,
                row.steps,
                row.body_battery_max,
                row.body_battery_min,
            ],
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0].map(Some)
        );
    }

    #[test]
    fn no_readings_gives_empty_rows() {
        let rows = build_health(&[day(14, 0, 100)], &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].steps, None);
        assert_eq!(rows[0].sleep_seconds, None);
    }
}

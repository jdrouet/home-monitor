use reqwest::Url;

use crate::JSON;
use crate::endpoints::build_url;

const ANY: &str = "*/*";

/// One of Garmin's documents kept in the archive. The template is the API
/// path and query, where `{name}` is the user's display name and `{d}` or
/// `{id}` the day or activity the document is about.
#[derive(Debug, Clone, Copy)]
pub struct GarminDocument {
    file: &'static str,
    template: &'static str,
}

impl GarminDocument {
    const fn new(file: &'static str, template: &'static str) -> Self {
        Self { file, template }
    }

    pub(crate) const fn file_name(&self) -> &'static str {
        self.file
    }

    /// Zip files take any content type, everything else is JSON.
    pub(crate) fn accept(&self) -> &'static str {
        if self.file.ends_with(".zip") {
            ANY
        } else {
            JSON
        }
    }

    pub(crate) fn url(&self, api: &Url, name: &str, key: &str) -> Url {
        let fill = |text: &str| {
            text.replace("{name}", name)
                .replace("{d}", key)
                .replace("{id}", key)
        };
        let (path, query) = self.template.split_once('?').unwrap_or((self.template, ""));
        let segments: Vec<String> = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(fill)
            .collect();
        let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
        let pairs: Vec<(&str, String)> = query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(name, value)| (name, fill(value)))
            .collect();
        let pairs: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        build_url(api, &segments, &pairs)
    }
}

pub(crate) const DAILY: &[GarminDocument] = &[
    GarminDocument::new(
        "user_summary.json",
        "/usersummary-service/usersummary/daily/{name}?calendarDate={d}",
    ),
    GarminDocument::new(
        "sleep.json",
        "/wellness-service/wellness/dailySleepData/{name}?date={d}&nonSleepBufferMinutes=60",
    ),
    GarminDocument::new(
        "naps.json",
        "/sleep-service/sleep/naps/{d}?includeOverlaps=true",
    ),
    GarminDocument::new(
        "heart_rate.json",
        "/wellness-service/wellness/dailyHeartRate/{name}?date={d}",
    ),
    GarminDocument::new("stress.json", "/wellness-service/wellness/dailyStress/{d}"),
    GarminDocument::new("hrv.json", "/hrv-service/hrv/{d}"),
    GarminDocument::new(
        "body_battery.json",
        "/wellness-service/wellness/bodyBattery/reports/daily?startDate={d}&endDate={d}",
    ),
    GarminDocument::new(
        "body_battery_events.json",
        "/wellness-service/wellness/bodyBattery/events/{d}",
    ),
    GarminDocument::new(
        "respiration.json",
        "/wellness-service/wellness/daily/respiration/{d}",
    ),
    GarminDocument::new(
        "spo2_acclimation.json",
        "/wellness-service/wellness/daily/spo2acclimation/{d}",
    ),
    GarminDocument::new(
        "intensity_minutes.json",
        "/wellness-service/wellness/daily/im/{d}",
    ),
    GarminDocument::new(
        "floors.json",
        "/wellness-service/wellness/floorsChartData/daily/{d}",
    ),
    GarminDocument::new(
        "steps.json",
        "/wellness-service/wellness/dailySummaryChart/{name}?date={d}",
    ),
    GarminDocument::new(
        "movement.json",
        "/wellness-service/wellness/dailyMovement?calendarDate={d}",
    ),
    GarminDocument::new(
        "events.json",
        "/wellness-service/wellness/dailyEvents?calendarDate={d}",
    ),
    GarminDocument::new(
        "activities_for_date.json",
        "/mobile-gateway/heartRate/forDate/{d}",
    ),
    GarminDocument::new(
        "hydration.json",
        "/usersummary-service/usersummary/hydration/allData/{d}",
    ),
    GarminDocument::new(
        "weight.json",
        "/weight-service/weight/dayview/{d}?includeAll=True",
    ),
    GarminDocument::new(
        "body_composition.json",
        "/weight-service/weight/dateRange?startDate={d}&endDate={d}",
    ),
    GarminDocument::new(
        "health_snapshots.json",
        "/wellnessactivity-service/activity/summary/{d}",
    ),
    GarminDocument::new(
        "max_metrics.json",
        "/metrics-service/metrics/maxmet/daily/{d}/{d}",
    ),
    GarminDocument::new(
        "training_status.json",
        "/metrics-service/metrics/trainingstatus/aggregated/{d}",
    ),
    GarminDocument::new(
        "training_status_daily.json",
        "/metrics-service/metrics/trainingstatus/daily/{d}",
    ),
    GarminDocument::new(
        "training_load_balance.json",
        "/metrics-service/metrics/trainingloadbalance/latest/{d}",
    ),
    GarminDocument::new(
        "training_readiness.json",
        "/metrics-service/metrics/trainingreadiness/{d}",
    ),
    GarminDocument::new(
        "endurance_score.json",
        "/metrics-service/metrics/endurancescore?calendarDate={d}",
    ),
    GarminDocument::new(
        "hill_score.json",
        "/metrics-service/metrics/hillscore?calendarDate={d}",
    ),
    GarminDocument::new(
        "race_predictions.json",
        "/metrics-service/metrics/racepredictions/daily/{name}?fromCalendarDate={d}&toCalendarDate={d}",
    ),
    GarminDocument::new(
        "lactate_threshold_speed.json",
        "/biometric-service/stats/lactateThresholdSpeed/range/{d}/{d}?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST",
    ),
    GarminDocument::new(
        "lactate_threshold_heart_rate.json",
        "/biometric-service/stats/lactateThresholdHeartRate/range/{d}/{d}?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST",
    ),
    GarminDocument::new(
        "ftp_running.json",
        "/biometric-service/stats/functionalThresholdPower/range/{d}/{d}?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST",
    ),
    GarminDocument::new(
        "ftp_cycling.json",
        "/biometric-service/stats/functionalThresholdPower/range/{d}/{d}?sport=CYCLING&aggregation=daily&aggregationStrategy=LATEST",
    ),
    GarminDocument::new(
        "power_to_weight.json",
        "/biometric-service/biometric/powerToWeight/latest/{d}?sport=Running",
    ),
    GarminDocument::new(
        "running_tolerance.json",
        "/metrics-service/metrics/runningtolerance/stats?startDate={d}&endDate={d}&aggregation=daily",
    ),
    GarminDocument::new(
        "running_economy.json",
        "/metrics-service/metrics/runningeconomy/latest/{d}",
    ),
    GarminDocument::new(
        "heat_altitude.json",
        "/metrics-service/metrics/heataltitudeacclimation/latest/{d}",
    ),
    GarminDocument::new(
        "cycling_ability.json",
        "/metrics-service/metrics/cycling-ability/latest?calendarDate={d}",
    ),
    GarminDocument::new(
        "critical_swim_speed.json",
        "/biometric-service/criticalSwimSpeed/latest/{d}",
    ),
    GarminDocument::new(
        "stamina_running.json",
        "/metrics-service/metrics/stamina-curve/latest?calendarDate={d}&sportType=running",
    ),
    GarminDocument::new(
        "stamina_cycling.json",
        "/metrics-service/metrics/stamina-curve/latest?calendarDate={d}&sportType=cycling",
    ),
    GarminDocument::new("fitness_age.json", "/fitnessage-service/fitnessage/v2/{d}"),
    GarminDocument::new(
        "one_rep_max.json",
        "/fitnessstats-service/vbt-stats/one-rep-max/current/{d}",
    ),
    GarminDocument::new("wellness.zip", "/download-service/files/wellness/{d}"),
];

pub(crate) const ACTIVITY: &[GarminDocument] = &[
    GarminDocument::new("summary.json", "/activity-service/activity/{id}"),
    GarminDocument::new(
        "details.json",
        "/activity-service/activity/{id}/details?maxChartSize=2000&maxPolylineSize=4000",
    ),
    GarminDocument::new("original.zip", "/download-service/files/activity/{id}"),
    GarminDocument::new("splits.json", "/activity-service/activity/{id}/splits"),
    GarminDocument::new(
        "typed_splits.json",
        "/activity-service/activity/{id}/typedsplits",
    ),
    GarminDocument::new(
        "split_summaries.json",
        "/activity-service/activity/{id}/split_summaries",
    ),
    GarminDocument::new(
        "hr_zones.json",
        "/activity-service/activity/{id}/hrTimeInZones",
    ),
    GarminDocument::new(
        "power_zones.json",
        "/activity-service/activity/{id}/powerTimeInZones",
    ),
    GarminDocument::new("weather.json", "/activity-service/activity/{id}/weather"),
    GarminDocument::new(
        "exercise_sets.json",
        "/activity-service/activity/{id}/exerciseSets",
    ),
    GarminDocument::new("rounds.json", "/activity-service/activity/{id}/rounds"),
    GarminDocument::new(
        "step_details.json",
        "/activity-service/activity/{id}/stepdetails",
    ),
    GarminDocument::new(
        "jump_details.json",
        "/activity-service/activity/{id}/jumpDetails",
    ),
    GarminDocument::new(
        "subsecond_details.json",
        "/activity-service/activity/{id}/subseconddetails?maxChartSize=2000",
    ),
    GarminDocument::new(
        "swim_details.json",
        "/activity-service/activity/{id}/swimDetails?maxChartSize=2000&maxPolylineSize=4000",
    ),
    GarminDocument::new(
        "power_curve.json",
        "/activity-service/activity/{id}/powerCurve",
    ),
    GarminDocument::new("segments.json", "/segment-service/segment/list/{id}"),
];

#[cfg(test)]
mod tests {
    use reqwest::Url;

    use super::{ACTIVITY, ANY, DAILY, GarminDocument};
    use crate::JSON;

    fn rendered(documents: &[GarminDocument], file: &str, key: &str) -> String {
        let document = documents
            .iter()
            .find(|document| document.file_name() == file)
            .expect("document");
        let api = Url::parse("http://api.test").expect("url");
        let url = document.url(&api, "Jane Doe", key);
        match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        }
    }

    #[test]
    fn the_catalogs_have_their_documents() {
        assert_eq!(DAILY.len(), 43);
        assert_eq!(ACTIVITY.len(), 17);
    }

    #[test]
    fn the_name_is_percent_encoded() {
        assert_eq!(
            rendered(DAILY, "sleep.json", "2026-07-14"),
            "/wellness-service/wellness/dailySleepData/Jane%20Doe?date=2026-07-14&nonSleepBufferMinutes=60"
        );
    }

    #[test]
    fn the_day_fills_every_placeholder() {
        assert_eq!(
            rendered(DAILY, "body_battery.json", "2026-07-14"),
            "/wellness-service/wellness/bodyBattery/reports/daily?startDate=2026-07-14&endDate=2026-07-14"
        );
    }

    #[test]
    fn the_activity_id_fills_the_path() {
        assert_eq!(
            rendered(ACTIVITY, "details.json", "123"),
            "/activity-service/activity/123/details?maxChartSize=2000&maxPolylineSize=4000"
        );
    }

    #[test]
    fn a_zip_renders_its_download_path_and_accepts_anything() {
        assert_eq!(
            rendered(ACTIVITY, "original.zip", "123"),
            "/download-service/files/activity/123"
        );
        assert_eq!(ACTIVITY[2].accept(), ANY);
        assert_eq!(ACTIVITY[0].accept(), JSON);
    }

    #[test]
    fn file_names_are_unique_per_catalog() {
        for documents in [DAILY, ACTIVITY] {
            let mut names: Vec<_> = documents.iter().map(GarminDocument::file_name).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), documents.len());
        }
    }
}

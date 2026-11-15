use reqwest::Url;

use crate::endpoints::build_url;

const JSON: &str = "application/json";
const ANY: &str = "*/*";

/// One of Garmin's documents kept in the archive. The template is the API
/// path and query, where `{name}` is the user's display name and `{d}` or
/// `{id}` the day or activity the document is about.
#[derive(Debug, Clone, Copy)]
pub struct GarminDocument {
    file: &'static str,
    template: &'static str,
    accept: &'static str,
}

impl GarminDocument {
    const fn json(file: &'static str, template: &'static str) -> Self {
        Self {
            file,
            template,
            accept: JSON,
        }
    }

    /// A binary document, such as a zip.
    const fn file(file: &'static str, template: &'static str) -> Self {
        Self {
            file,
            template,
            accept: ANY,
        }
    }

    pub(crate) const fn file_name(&self) -> &'static str {
        self.file
    }

    pub(crate) const fn accept(&self) -> &'static str {
        self.accept
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

const DAILY_DOCUMENTS: &[GarminDocument] = &[
    GarminDocument::json(
        "user_summary.json",
        "/usersummary-service/usersummary/daily/{name}?calendarDate={d}",
    ),
    GarminDocument::json(
        "sleep.json",
        "/wellness-service/wellness/dailySleepData/{name}?date={d}&nonSleepBufferMinutes=60",
    ),
    GarminDocument::json(
        "naps.json",
        "/sleep-service/sleep/naps/{d}?includeOverlaps=true",
    ),
    GarminDocument::json(
        "heart_rate.json",
        "/wellness-service/wellness/dailyHeartRate/{name}?date={d}",
    ),
    GarminDocument::json("stress.json", "/wellness-service/wellness/dailyStress/{d}"),
    GarminDocument::json("hrv.json", "/hrv-service/hrv/{d}"),
    GarminDocument::json(
        "body_battery.json",
        "/wellness-service/wellness/bodyBattery/reports/daily?startDate={d}&endDate={d}",
    ),
    GarminDocument::json(
        "body_battery_events.json",
        "/wellness-service/wellness/bodyBattery/events/{d}",
    ),
    GarminDocument::json(
        "respiration.json",
        "/wellness-service/wellness/daily/respiration/{d}",
    ),
    GarminDocument::json(
        "spo2_acclimation.json",
        "/wellness-service/wellness/daily/spo2acclimation/{d}",
    ),
    GarminDocument::json(
        "intensity_minutes.json",
        "/wellness-service/wellness/daily/im/{d}",
    ),
    GarminDocument::json(
        "floors.json",
        "/wellness-service/wellness/floorsChartData/daily/{d}",
    ),
    GarminDocument::json(
        "steps.json",
        "/wellness-service/wellness/dailySummaryChart/{name}?date={d}",
    ),
    GarminDocument::json(
        "movement.json",
        "/wellness-service/wellness/dailyMovement?calendarDate={d}",
    ),
    GarminDocument::json(
        "events.json",
        "/wellness-service/wellness/dailyEvents?calendarDate={d}",
    ),
    GarminDocument::json(
        "activities_for_date.json",
        "/mobile-gateway/heartRate/forDate/{d}",
    ),
    GarminDocument::json(
        "hydration.json",
        "/usersummary-service/usersummary/hydration/allData/{d}",
    ),
    GarminDocument::json(
        "weight.json",
        "/weight-service/weight/dayview/{d}?includeAll=True",
    ),
    GarminDocument::json(
        "body_composition.json",
        "/weight-service/weight/dateRange?startDate={d}&endDate={d}",
    ),
    GarminDocument::json(
        "health_snapshots.json",
        "/wellnessactivity-service/activity/summary/{d}",
    ),
    GarminDocument::json(
        "max_metrics.json",
        "/metrics-service/metrics/maxmet/daily/{d}/{d}",
    ),
    GarminDocument::json(
        "training_status.json",
        "/metrics-service/metrics/trainingstatus/aggregated/{d}",
    ),
    GarminDocument::json(
        "training_status_daily.json",
        "/metrics-service/metrics/trainingstatus/daily/{d}",
    ),
    GarminDocument::json(
        "training_load_balance.json",
        "/metrics-service/metrics/trainingloadbalance/latest/{d}",
    ),
    GarminDocument::json(
        "training_readiness.json",
        "/metrics-service/metrics/trainingreadiness/{d}",
    ),
    GarminDocument::json(
        "endurance_score.json",
        "/metrics-service/metrics/endurancescore?calendarDate={d}",
    ),
    GarminDocument::json(
        "hill_score.json",
        "/metrics-service/metrics/hillscore?calendarDate={d}",
    ),
    GarminDocument::json(
        "race_predictions.json",
        "/metrics-service/metrics/racepredictions/daily/{name}?fromCalendarDate={d}&toCalendarDate={d}",
    ),
    GarminDocument::json(
        "lactate_threshold_speed.json",
        "/biometric-service/stats/lactateThresholdSpeed/range/{d}/{d}?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST",
    ),
    GarminDocument::json(
        "lactate_threshold_heart_rate.json",
        "/biometric-service/stats/lactateThresholdHeartRate/range/{d}/{d}?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST",
    ),
    GarminDocument::json(
        "ftp_running.json",
        "/biometric-service/stats/functionalThresholdPower/range/{d}/{d}?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST",
    ),
    GarminDocument::json(
        "ftp_cycling.json",
        "/biometric-service/stats/functionalThresholdPower/range/{d}/{d}?sport=CYCLING&aggregation=daily&aggregationStrategy=LATEST",
    ),
    GarminDocument::json(
        "power_to_weight.json",
        "/biometric-service/biometric/powerToWeight/latest/{d}?sport=Running",
    ),
    GarminDocument::json(
        "running_tolerance.json",
        "/metrics-service/metrics/runningtolerance/stats?startDate={d}&endDate={d}&aggregation=daily",
    ),
    GarminDocument::json(
        "running_economy.json",
        "/metrics-service/metrics/runningeconomy/latest/{d}",
    ),
    GarminDocument::json(
        "heat_altitude.json",
        "/metrics-service/metrics/heataltitudeacclimation/latest/{d}",
    ),
    GarminDocument::json(
        "cycling_ability.json",
        "/metrics-service/metrics/cycling-ability/latest?calendarDate={d}",
    ),
    GarminDocument::json(
        "critical_swim_speed.json",
        "/biometric-service/criticalSwimSpeed/latest/{d}",
    ),
    GarminDocument::json(
        "stamina_running.json",
        "/metrics-service/metrics/stamina-curve/latest?calendarDate={d}&sportType=running",
    ),
    GarminDocument::json(
        "stamina_cycling.json",
        "/metrics-service/metrics/stamina-curve/latest?calendarDate={d}&sportType=cycling",
    ),
    GarminDocument::json("fitness_age.json", "/fitnessage-service/fitnessage/v2/{d}"),
    GarminDocument::json(
        "one_rep_max.json",
        "/fitnessstats-service/vbt-stats/one-rep-max/current/{d}",
    ),
    GarminDocument::file("wellness.zip", "/download-service/files/wellness/{d}"),
];

const ACTIVITY_DOCUMENTS: &[GarminDocument] = &[
    GarminDocument::json("summary.json", "/activity-service/activity/{id}"),
    GarminDocument::json(
        "details.json",
        "/activity-service/activity/{id}/details?maxChartSize=2000&maxPolylineSize=4000",
    ),
    GarminDocument::file("original.zip", "/download-service/files/activity/{id}"),
    GarminDocument::json("splits.json", "/activity-service/activity/{id}/splits"),
    GarminDocument::json(
        "typed_splits.json",
        "/activity-service/activity/{id}/typedsplits",
    ),
    GarminDocument::json(
        "split_summaries.json",
        "/activity-service/activity/{id}/split_summaries",
    ),
    GarminDocument::json(
        "hr_zones.json",
        "/activity-service/activity/{id}/hrTimeInZones",
    ),
    GarminDocument::json(
        "power_zones.json",
        "/activity-service/activity/{id}/powerTimeInZones",
    ),
    GarminDocument::json("weather.json", "/activity-service/activity/{id}/weather"),
    GarminDocument::json(
        "exercise_sets.json",
        "/activity-service/activity/{id}/exerciseSets",
    ),
    GarminDocument::json("rounds.json", "/activity-service/activity/{id}/rounds"),
    GarminDocument::json(
        "step_details.json",
        "/activity-service/activity/{id}/stepdetails",
    ),
    GarminDocument::json(
        "jump_details.json",
        "/activity-service/activity/{id}/jumpDetails",
    ),
    GarminDocument::json(
        "subsecond_details.json",
        "/activity-service/activity/{id}/subseconddetails?maxChartSize=2000",
    ),
    GarminDocument::json(
        "swim_details.json",
        "/activity-service/activity/{id}/swimDetails?maxChartSize=2000&maxPolylineSize=4000",
    ),
    GarminDocument::json(
        "power_curve.json",
        "/activity-service/activity/{id}/powerCurve",
    ),
    GarminDocument::json("segments.json", "/segment-service/segment/list/{id}"),
];

pub(crate) const DAILY: &[GarminDocument] = DAILY_DOCUMENTS;
pub(crate) const ACTIVITY: &[GarminDocument] = ACTIVITY_DOCUMENTS;

#[cfg(test)]
mod tests {
    use reqwest::Url;

    use super::{ACTIVITY, DAILY, GarminDocument};

    const DAILY_LINES: &[(&str, &str)] = &[
        (
            "user_summary.json",
            "/usersummary-service/usersummary/daily/Jane%20Doe?calendarDate=2026-07-14",
        ),
        (
            "sleep.json",
            "/wellness-service/wellness/dailySleepData/Jane%20Doe?date=2026-07-14&nonSleepBufferMinutes=60",
        ),
        (
            "naps.json",
            "/sleep-service/sleep/naps/2026-07-14?includeOverlaps=true",
        ),
        (
            "heart_rate.json",
            "/wellness-service/wellness/dailyHeartRate/Jane%20Doe?date=2026-07-14",
        ),
        (
            "stress.json",
            "/wellness-service/wellness/dailyStress/2026-07-14",
        ),
        ("hrv.json", "/hrv-service/hrv/2026-07-14"),
        (
            "body_battery.json",
            "/wellness-service/wellness/bodyBattery/reports/daily?startDate=2026-07-14&endDate=2026-07-14",
        ),
        (
            "body_battery_events.json",
            "/wellness-service/wellness/bodyBattery/events/2026-07-14",
        ),
        (
            "respiration.json",
            "/wellness-service/wellness/daily/respiration/2026-07-14",
        ),
        (
            "spo2_acclimation.json",
            "/wellness-service/wellness/daily/spo2acclimation/2026-07-14",
        ),
        (
            "intensity_minutes.json",
            "/wellness-service/wellness/daily/im/2026-07-14",
        ),
        (
            "floors.json",
            "/wellness-service/wellness/floorsChartData/daily/2026-07-14",
        ),
        (
            "steps.json",
            "/wellness-service/wellness/dailySummaryChart/Jane%20Doe?date=2026-07-14",
        ),
        (
            "movement.json",
            "/wellness-service/wellness/dailyMovement?calendarDate=2026-07-14",
        ),
        (
            "events.json",
            "/wellness-service/wellness/dailyEvents?calendarDate=2026-07-14",
        ),
        (
            "activities_for_date.json",
            "/mobile-gateway/heartRate/forDate/2026-07-14",
        ),
        (
            "hydration.json",
            "/usersummary-service/usersummary/hydration/allData/2026-07-14",
        ),
        (
            "weight.json",
            "/weight-service/weight/dayview/2026-07-14?includeAll=True",
        ),
        (
            "body_composition.json",
            "/weight-service/weight/dateRange?startDate=2026-07-14&endDate=2026-07-14",
        ),
        (
            "health_snapshots.json",
            "/wellnessactivity-service/activity/summary/2026-07-14",
        ),
        (
            "max_metrics.json",
            "/metrics-service/metrics/maxmet/daily/2026-07-14/2026-07-14",
        ),
        (
            "training_status.json",
            "/metrics-service/metrics/trainingstatus/aggregated/2026-07-14",
        ),
        (
            "training_status_daily.json",
            "/metrics-service/metrics/trainingstatus/daily/2026-07-14",
        ),
        (
            "training_load_balance.json",
            "/metrics-service/metrics/trainingloadbalance/latest/2026-07-14",
        ),
        (
            "training_readiness.json",
            "/metrics-service/metrics/trainingreadiness/2026-07-14",
        ),
        (
            "endurance_score.json",
            "/metrics-service/metrics/endurancescore?calendarDate=2026-07-14",
        ),
        (
            "hill_score.json",
            "/metrics-service/metrics/hillscore?calendarDate=2026-07-14",
        ),
        (
            "race_predictions.json",
            "/metrics-service/metrics/racepredictions/daily/Jane%20Doe?fromCalendarDate=2026-07-14&toCalendarDate=2026-07-14",
        ),
        (
            "lactate_threshold_speed.json",
            "/biometric-service/stats/lactateThresholdSpeed/range/2026-07-14/2026-07-14?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST",
        ),
        (
            "lactate_threshold_heart_rate.json",
            "/biometric-service/stats/lactateThresholdHeartRate/range/2026-07-14/2026-07-14?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST",
        ),
        (
            "ftp_running.json",
            "/biometric-service/stats/functionalThresholdPower/range/2026-07-14/2026-07-14?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST",
        ),
        (
            "ftp_cycling.json",
            "/biometric-service/stats/functionalThresholdPower/range/2026-07-14/2026-07-14?sport=CYCLING&aggregation=daily&aggregationStrategy=LATEST",
        ),
        (
            "power_to_weight.json",
            "/biometric-service/biometric/powerToWeight/latest/2026-07-14?sport=Running",
        ),
        (
            "running_tolerance.json",
            "/metrics-service/metrics/runningtolerance/stats?startDate=2026-07-14&endDate=2026-07-14&aggregation=daily",
        ),
        (
            "running_economy.json",
            "/metrics-service/metrics/runningeconomy/latest/2026-07-14",
        ),
        (
            "heat_altitude.json",
            "/metrics-service/metrics/heataltitudeacclimation/latest/2026-07-14",
        ),
        (
            "cycling_ability.json",
            "/metrics-service/metrics/cycling-ability/latest?calendarDate=2026-07-14",
        ),
        (
            "critical_swim_speed.json",
            "/biometric-service/criticalSwimSpeed/latest/2026-07-14",
        ),
        (
            "stamina_running.json",
            "/metrics-service/metrics/stamina-curve/latest?calendarDate=2026-07-14&sportType=running",
        ),
        (
            "stamina_cycling.json",
            "/metrics-service/metrics/stamina-curve/latest?calendarDate=2026-07-14&sportType=cycling",
        ),
        (
            "fitness_age.json",
            "/fitnessage-service/fitnessage/v2/2026-07-14",
        ),
        (
            "one_rep_max.json",
            "/fitnessstats-service/vbt-stats/one-rep-max/current/2026-07-14",
        ),
        (
            "wellness.zip",
            "/download-service/files/wellness/2026-07-14",
        ),
    ];

    const ACTIVITY_LINES: &[(&str, &str)] = &[
        ("summary.json", "/activity-service/activity/123"),
        (
            "details.json",
            "/activity-service/activity/123/details?maxChartSize=2000&maxPolylineSize=4000",
        ),
        ("original.zip", "/download-service/files/activity/123"),
        ("splits.json", "/activity-service/activity/123/splits"),
        (
            "typed_splits.json",
            "/activity-service/activity/123/typedsplits",
        ),
        (
            "split_summaries.json",
            "/activity-service/activity/123/split_summaries",
        ),
        (
            "hr_zones.json",
            "/activity-service/activity/123/hrTimeInZones",
        ),
        (
            "power_zones.json",
            "/activity-service/activity/123/powerTimeInZones",
        ),
        ("weather.json", "/activity-service/activity/123/weather"),
        (
            "exercise_sets.json",
            "/activity-service/activity/123/exerciseSets",
        ),
        ("rounds.json", "/activity-service/activity/123/rounds"),
        (
            "step_details.json",
            "/activity-service/activity/123/stepdetails",
        ),
        (
            "jump_details.json",
            "/activity-service/activity/123/jumpDetails",
        ),
        (
            "subsecond_details.json",
            "/activity-service/activity/123/subseconddetails?maxChartSize=2000",
        ),
        (
            "swim_details.json",
            "/activity-service/activity/123/swimDetails?maxChartSize=2000&maxPolylineSize=4000",
        ),
        (
            "power_curve.json",
            "/activity-service/activity/123/powerCurve",
        ),
        ("segments.json", "/segment-service/segment/list/123"),
    ];

    fn api() -> Url {
        Url::parse("http://api.test").expect("url")
    }

    fn assert_catalog(documents: &[GarminDocument], lines: &[(&str, &str)], key: &str) {
        assert_eq!(documents.len(), lines.len());
        for (document, (file, target)) in documents.iter().zip(lines) {
            assert_eq!(document.file_name(), *file);
            let url = document.url(&api(), "Jane Doe", key);
            let rendered = match url.query() {
                Some(query) => format!("{}?{query}", url.path()),
                None => url.path().to_owned(),
            };
            assert_eq!(rendered, *target, "{file}");
        }
    }

    #[test]
    fn every_daily_document_has_its_exact_path_and_query() {
        assert_catalog(DAILY, DAILY_LINES, "2026-07-14");
    }

    #[test]
    fn every_activity_document_has_its_exact_path_and_query() {
        assert_catalog(ACTIVITY, ACTIVITY_LINES, "123");
    }

    #[test]
    fn only_the_zips_accept_anything() {
        for document in DAILY.iter().chain(ACTIVITY) {
            let zip = document.file_name().ends_with(".zip");
            assert_eq!(
                document.accept(),
                if zip { "*/*" } else { "application/json" }
            );
        }
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

# ADR-0025: Garmin raw archive and request pacing

Status: accepted

## Context
Issue #32 asks to keep, at home and in Garmin's original form, everything
Garmin offers: every workout with its original file, one year of daily data and
all past workouts. The copy must survive Garmin access stopping, and every
request must be paced with a random delay. ADR-0022 paced data calls 3 s
apart; ADR-0023 bounded one step (about 70 s) inside a 5-minute margin;
ADR-0024 runs the gap fill before the yearly backfill.

## Decision
- Pacing: one process-wide pacer in `garmin-connect` spaces every request
  (login, refresh, data, downloads) by a uniformly random 3 to 6 s between
  dispatches. The gap is drawn from std `RandomState`. The pacer enforces it at
  dispatch: after sleeping it re-checks under its lock that the gap has passed,
  then records the send. Replaces ADR-0022's fixed 3 s pause.
- Redirects: neither HTTP client follows redirects automatically. A shared
  helper follows `Location` hops by hand, and each hop waits on the pacer, so
  every request is paced. The API follows at most 3 hops and the sign-in page
  at most 5; a redirect past the limit ends as a status error. The bearer
  token goes only to the API origin. SSO cookies are sent to, and absorbed
  from, only the SSO origin.
- Catalog: 43 per-day and 17 per-activity documents from python-garminconnect
  0.3.20, kept as data in the adapter, one template line each (`{name}`, `{d}`,
  `{id}` stand for the display name, the day and the activity id). The
  inventory below gives the reason for every per-day and per-activity GET.
  Excluded: duplicates, range aggregates, undated "latest" reads with dated
  variants, accessories and features not used, user-entered logs, settings and
  gamification, documents keyed by nested ids, write endpoints.
- Layout: under `[health_collector] archive_dir` (optional, default
  `/archive/garmin`, volume `garmin-archive`): `daily/<YYYY>/<YYYY-MM-DD>/<file>`
  and `activities/<YYYY>/<activityId>/<file>`. Bodies are stored as Garmin sent
  them and the original activity zip is not extracted. Files are written as in
  ADR-0019, with new directories created one level at a time and each new
  directory's parent fsynced. The adapter never creates the root's parent (the
  mount point): without it archive access fails and the archive runners stop,
  so nothing reaches the container layer.
- Document outcomes: a 2xx body is stored; a 204 or 404 is confirmed absence
  and stored as an empty file; a 403 is stored as an empty `<file>.denied`
  marker and no document. A day or activity counts as stored when each document
  or its marker exists, so a permanently forbidden endpoint (a premium feature,
  for example) does not block the runner. A temporary 403 is recorded the same
  way; deleting `*.denied` markers re-fetches those documents, and a successful
  later write removes its marker.
- Completed days only: rolling cursor `garmin_archive.json` (ADR-0023 format).
  The first range starts at today-365. After each completed pull `last` moves
  to the newest final day (yesterday once a pull from 10:00 fetched it, else
  the day before). Each day is kept missing-only and passed only when every
  document is present. The daily pull and the gap fill archive nothing and are
  unchanged.
- Activities: `garmin_activities.json`
  `{"version":1,"start":N,"done":bool,"catch_up":M|null}`. The history walks
  the list newest first, 5 per step, and resumes one page early to absorb
  deletions. After a page it saves `start` as the offset the page was read at
  plus its length. Once `done`, each run catches up from the top: it resumes
  one page before `catch_up` when set, saves `catch_up` after each complete
  page, and ends when it reaches a page with nothing new at or past the saved
  offset, or a short page, which clears `catch_up`. A run reads at most 10
  pages; hitting the cap keeps `catch_up` so the next run continues, and the
  run reports `OutOfTime`.
- Order and isolation: after a completed pull: gap fill, yearly metrics,
  activities, archive days. A run that stopped ends the chain, including a
  stopped yearly run. Metrics runners never touch storage, so a storage
  failure stops only the archive work.
- Stops: in archive paths only a 401 is a session error and 427 or 429 is
  throttling. Any other failure (5xx, transport, timeout, undecodable list,
  failed archive read or write) stops the run without advancing, and it
  resumes next window. None of this publishes `session_valid`; the #16 alert
  still comes only from the daily pull.
- Timing (amends ADR-0023): 60 s per step and the 30-minute session age stay.
  Because one step now spans many requests, the window and the session age are
  checked before each fetch. A step that would cross the 5-minute margin ends
  without advancing; one reaching the session age reopens and resumes
  missing-only.
- Logs and errors carry dates, activity ids and file names, never bodies,
  tokens or URLs.
- Volume: about 72 requests per day in steady state; the history is about
  33,000 requests (about 15,700 for the days, about 17,200 for 1,000
  activities), about 51 h of windows (2 to 3 days).
- Rollout: `git pull` (compose volume) first, then
  `docker compose pull health-collector && docker compose up -d health-collector`;
  set `archive_dir` only once the new image runs.
- Rejected: archiving provisional "today" documents; extracting the activity
  zip; per-day activity listing; the `rand` crate; dropping the step spacing;
  activity catch-up inside the daily pull (it would mix storage into the pull
  and #16); config keys for the cursors.

## Consequences
- About 11 to 13 requests per minute during import and a higher block risk,
  accepted by the issue.
- A persistently failing endpoint blocks its runner until it recovers; the log
  names the file. A 403 or 404 does not block.
- Endpoints that do not apply cost a request per day or activity and store
  empty files.
- Disk use is about 0.2 GB per year of days and 1.5 to 3.5 GB per 1,000
  activities (estimates).
- Deleting a cursor re-runs that import; stored files are skipped. Activity
  edits after archiving are not re-fetched.
- Residual risk: deleting more than one page (5) of already traversed
  activities during an interrupted history walk can shift unseen activities
  below the resume offset, and they would be missed. Deleting
  `garmin_activities.json` re-walks everything.

## Catalog inventory (verbatim from 0.3.20; `{name}` displayName, `{d}` date, `{id}` activity id)
### Per-day — included (43)
user_summary.json `/usersummary-service/usersummary/daily/{name}?calendarDate={d}`
sleep.json `/wellness-service/wellness/dailySleepData/{name}?date={d}&nonSleepBufferMinutes=60`
naps.json `/sleep-service/sleep/naps/{d}?includeOverlaps=true`
heart_rate.json `/wellness-service/wellness/dailyHeartRate/{name}?date={d}`
stress.json `/wellness-service/wellness/dailyStress/{d}`
hrv.json `/hrv-service/hrv/{d}`
body_battery.json `/wellness-service/wellness/bodyBattery/reports/daily?startDate={d}&endDate={d}`
body_battery_events.json `/wellness-service/wellness/bodyBattery/events/{d}`
respiration.json `/wellness-service/wellness/daily/respiration/{d}`
spo2_acclimation.json `/wellness-service/wellness/daily/spo2acclimation/{d}`
intensity_minutes.json `/wellness-service/wellness/daily/im/{d}`
floors.json `/wellness-service/wellness/floorsChartData/daily/{d}`
steps.json `/wellness-service/wellness/dailySummaryChart/{name}?date={d}`
movement.json `/wellness-service/wellness/dailyMovement?calendarDate={d}`
events.json `/wellness-service/wellness/dailyEvents?calendarDate={d}`
activities_for_date.json `/mobile-gateway/heartRate/forDate/{d}` (content UNVERIFIED)
hydration.json `/usersummary-service/usersummary/hydration/allData/{d}`
weight.json `/weight-service/weight/dayview/{d}?includeAll=True`
body_composition.json `/weight-service/weight/dateRange?startDate={d}&endDate={d}`
health_snapshots.json `/wellnessactivity-service/activity/summary/{d}`
max_metrics.json `/metrics-service/metrics/maxmet/daily/{d}/{d}`
training_status.json `/metrics-service/metrics/trainingstatus/aggregated/{d}`
training_status_daily.json `/metrics-service/metrics/trainingstatus/daily/{d}`
training_load_balance.json `/metrics-service/metrics/trainingloadbalance/latest/{d}`
training_readiness.json `/metrics-service/metrics/trainingreadiness/{d}`
endurance_score.json `/metrics-service/metrics/endurancescore?calendarDate={d}`
hill_score.json `/metrics-service/metrics/hillscore?calendarDate={d}`
race_predictions.json `/metrics-service/metrics/racepredictions/daily/{name}?fromCalendarDate={d}&toCalendarDate={d}`
lactate_threshold_speed.json `/biometric-service/stats/lactateThresholdSpeed/range/{d}/{d}?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST`
lactate_threshold_heart_rate.json `/biometric-service/stats/lactateThresholdHeartRate/range/{d}/{d}?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST`
ftp_running.json `/biometric-service/stats/functionalThresholdPower/range/{d}/{d}?sport=RUNNING&aggregation=daily&aggregationStrategy=LATEST`
ftp_cycling.json `/biometric-service/stats/functionalThresholdPower/range/{d}/{d}?sport=CYCLING&aggregation=daily&aggregationStrategy=LATEST`
power_to_weight.json `/biometric-service/biometric/powerToWeight/latest/{d}?sport=Running`
running_tolerance.json `/metrics-service/metrics/runningtolerance/stats?startDate={d}&endDate={d}&aggregation=daily`
running_economy.json `/metrics-service/metrics/runningeconomy/latest/{d}`
heat_altitude.json `/metrics-service/metrics/heataltitudeacclimation/latest/{d}`
cycling_ability.json `/metrics-service/metrics/cycling-ability/latest?calendarDate={d}`
critical_swim_speed.json `/biometric-service/criticalSwimSpeed/latest/{d}`
stamina_running.json `/metrics-service/metrics/stamina-curve/latest?calendarDate={d}&sportType=running`
stamina_cycling.json `/metrics-service/metrics/stamina-curve/latest?calendarDate={d}&sportType=cycling`
fitness_age.json `/fitnessage-service/fitnessage/v2/{d}`
one_rep_max.json `/fitnessstats-service/vbt-stats/one-rep-max/current/{d}`
wellness.zip `/download-service/files/wellness/{d}` (Accept `*/*`; content UNVERIFIED)
### Per-day — excluded (reason)
`/usersummary-service/usersummary/hydration/daily/{d}` lighter duplicate of allData; `/wellness-service/wellness/daily/spo2/{d}` subset of spo2acclimation; `/fitnessage-service/fitnessage/{d}` older duplicate of v2; `/sleep-service/sleep/dailySleepData?date&nonSleepBufferMinutes` same payload as wellness sleep; `/metrics-service/metrics/maxmet/latest/{d}` as-of copy of maxmet daily; `/userstats-service/wellness/daily/{name}?fromDate&untilDate&metricId=60` (and 22/23) in user summary; `/usersummary-service/usersummary/weekAverageRestingHR/{name}?calendarDate=` recomputable; `/weight-service/weight/latest?date&ignorePriority=true` as-of copy of dayview; range aggregates (`/usersummary-service/stats/...`, `/usersummary-service/v2/dailysummaries`, `/hrv-service/hrv/daily/{s}/{e}`, `/sleep-service/stats/sleep/...`, `.../trainingreadiness/{s}/{e}`, `.../trainingstatus/weekly/{name}`, `/wellness-service/stats/...`, `/userstats-service/wellness|activities/weekly|monthly/...`, `/fitnessage-service/stats/...`, `/wellnessactivity-service/activity/summary/range/...`, weekly/monthly weight/BP, `.../endurancescore/stats`, `.../hillscore/stats`, `.../heataltitudeacclimation/{agg}/{s}/{e}`, `.../runningeconomy/{agg}`, `.../cycling-ability?…aggregation`, `/biometric-service/stats?…`, `.../criticalSwimSpeed/range/...`, `/fitnessstats-service/powerCurve`, one-rep-max history); undated "latest" with dated variants included (`/biometric-service/biometric/latestLactateThreshold`, `.../latestFunctionalThresholdPower/CYCLING`, `.../racepredictions/latest/{name}`); per-snapshot docs needing ids (`/wellnessactivity-service/activity/summary/{d}/{uuid}`, `.../epoch/{uuid}`, `/download-service/files/wellness/activity/{d}/{uuid}`); accessories not used (`/bloodpressure-service/bloodpressure/dayview/{d}`, `/biometric-service/biometric/glucose/{d}`, ECG, `/periodichealth-service/...`); user-entered (`/lifestylelogging-service/dailyLog/{d}`, nutrition, `/jetlag-service/jetlag/status/daily/{d}`, `/trainingplan-service/trainingplan/active-plans?startDate=`); settings/gamification (`/wellness-service/wellness/wellness-goals/consolidated/steps/{d}`, `/goal-service/goal/user/effective/{d}`, `/badge-service/badge/{name}/earned/dailysummary/{d}`); device-specific (`/web-gateway/solar/{deviceId}/{s}/{e}`).
### Per-activity — included (17)
summary.json `/activity-service/activity/{id}`; details.json `/activity-service/activity/{id}/details?maxChartSize=2000&maxPolylineSize=4000`; original.zip `/download-service/files/activity/{id}` (Accept `*/*`); splits.json `/activity-service/activity/{id}/splits`; typed_splits.json `/activity-service/activity/{id}/typedsplits`; split_summaries.json `/activity-service/activity/{id}/split_summaries`; hr_zones.json `/activity-service/activity/{id}/hrTimeInZones`; power_zones.json `/activity-service/activity/{id}/powerTimeInZones`; weather.json `/activity-service/activity/{id}/weather`; exercise_sets.json `/activity-service/activity/{id}/exerciseSets`; rounds.json `/activity-service/activity/{id}/rounds`; step_details.json `/activity-service/activity/{id}/stepdetails`; jump_details.json `/activity-service/activity/{id}/jumpDetails`; subsecond_details.json `/activity-service/activity/{id}/subseconddetails?maxChartSize=2000`; swim_details.json `/activity-service/activity/{id}/swimDetails?maxChartSize=2000&maxPolylineSize=4000`; power_curve.json `/activity-service/activity/{id}/powerCurve`; segments.json `/segment-service/segment/list/{id}`.
### Per-activity — excluded
`/activity-service/activity/{id}/laps` duplicate of splits' lapDTOs (UNVERIFIED); `.../{id}/mapdetails`, `/activity-service/activity/polyline/{id}` subset of details; `.../{id}/prev`, `.../{id}/next` navigation; `/download-service/export/{tcx,gpx,kml,csv}/activity/{id}` derived from original; `/gear-service/gear/filterGear?activityId={id}`, `/gear-service/activity/v2/{id}` user-linked gear; `/badge-service/badge/{name}/earned/activity/{id}` gamification; photos (presigned links expiring after 24 h).
List: `/activitylist-service/activities/search/activities?start={n}&limit={k}` (`activityId` verified; `startTimeLocal` UNVERIFIED). All write/upload/delete endpoints excluded.

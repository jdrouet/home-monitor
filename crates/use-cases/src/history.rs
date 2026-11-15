use std::num::NonZeroU32;

use domain::{
    Measurement, Metric, RoomName, SeriesQuery, SourceId, SourceKind, Thermometer, Timestamp,
};

const MICROS: i64 = 1_000_000;
const OUTSIDE: &str = "Outside";

/// Metrics the history page can chart.
pub const HISTORY_METRICS: [Metric; 2] = [Metric::Temperature, Metric::Humidity];

/// How far back a chart reaches. Each period buckets its readings at a width
/// that is a multiple of the weather's 15-minute interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Month,
    Year,
}

impl Period {
    pub const ALL: [Self; 4] = [Self::Day, Self::Week, Self::Month, Self::Year];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
            Self::Year => "1y",
        }
    }

    pub const fn span_secs(self) -> i64 {
        match self {
            Self::Day => 86_400,
            Self::Week => 7 * 86_400,
            Self::Month => 30 * 86_400,
            Self::Year => 365 * 86_400,
        }
    }

    pub const fn bucket_secs(self) -> NonZeroU32 {
        let secs = match self {
            Self::Day => 900,
            Self::Week => 1_800,
            Self::Month => 7_200,
            Self::Year => 86_400,
        };
        NonZeroU32::new(secs).expect("bucket widths are not zero")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistorySelection {
    pub metric: Metric,
    pub period: Period,
    pub rooms: Vec<RoomName>,
    pub include_outside: bool,
}

impl HistorySelection {
    pub fn shows_room(&self, room: &RoomName) -> bool {
        self.rooms.contains(room)
    }
}

/// Lines aligned on `timestamps` (Unix seconds): `lines[i][j]` is the mean of
/// `labels[i]` in the bucket starting at `timestamps[j]`, `None` for a gap.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryChart {
    pub labels: Vec<String>,
    pub timestamps: Vec<i64>,
    pub lines: Vec<Vec<Option<f64>>>,
}

struct Grid {
    from: i64,
    width: i64,
    count: usize,
}

impl Grid {
    fn new(period: Period, now: Timestamp) -> Self {
        let width = i64::from(period.bucket_secs().get()) * MICROS;
        let now = now.as_micros();
        let from = (now - period.span_secs() * MICROS).div_euclid(width) * width;
        let count = usize::try_from((now - from + width - 1) / width).unwrap_or(0);
        Self { from, width, count }
    }

    fn timestamps(&self) -> Vec<i64> {
        (0..self.count)
            .map(|index| (self.from + index as i64 * self.width) / MICROS)
            .collect()
    }

    fn line<'a>(&self, rows: impl Iterator<Item = &'a Measurement>) -> Vec<Option<f64>> {
        let mut line = vec![None; self.count];
        for row in rows {
            let index = (row.timestamp.as_micros() - self.from).div_euclid(self.width);
            if let Some(slot) = usize::try_from(index)
                .ok()
                .and_then(|index| line.get_mut(index))
            {
                *slot = Some(row.value);
            }
        }
        line
    }
}

/// The store query behind a chart: every source of the metric, bucketed.
pub fn history_query(selection: &HistorySelection, now: Timestamp) -> SeriesQuery {
    let grid = Grid::new(selection.period, now);
    SeriesQuery {
        metric: selection.metric,
        source_id: None,
        from: Timestamp::from_micros(grid.from),
        to: now,
        bucket_secs: Some(selection.period.bucket_secs()),
    }
}

/// Selected configured rooms in configuration order, then "Outside". Sources
/// that are neither are dropped.
pub fn build_history(
    thermometers: &[Thermometer],
    selection: &HistorySelection,
    now: Timestamp,
    measurements: &[Measurement],
) -> HistoryChart {
    let grid = Grid::new(selection.period, now);
    let mut labels = Vec::new();
    let mut lines = Vec::new();
    for thermometer in thermometers
        .iter()
        .filter(|thermometer| selection.shows_room(&thermometer.room))
    {
        let source_id = SourceId::from(thermometer.mac);
        labels.push(thermometer.room.to_string());
        lines.push(
            grid.line(
                measurements
                    .iter()
                    .filter(|row| row.source_kind == SourceKind::Ble && row.source_id == source_id),
            ),
        );
    }
    if selection.include_outside {
        labels.push(OUTSIDE.to_owned());
        lines.push(
            grid.line(
                measurements
                    .iter()
                    .filter(|row| row.source_kind == SourceKind::Weather),
            ),
        );
    }
    HistoryChart {
        labels,
        timestamps: grid.timestamps(),
        lines,
    }
}

#[cfg(test)]
mod tests {
    use domain::{
        MacAddress, Measurement, Metric, RoomName, SourceId, SourceKind, Thermometer, Timestamp,
    };

    use super::{HISTORY_METRICS, HistorySelection, Period, build_history, history_query};

    const MAC_BEDROOM: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 1]);
    const MAC_OFFICE: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 2]);
    const MAC_STRANGER: MacAddress = MacAddress::new([0xA4, 0xC1, 0x38, 0, 0, 9]);
    const ALIGNED_NOW: i64 = 1_000_000 * 900 * 10_000;

    fn room(name: &str) -> RoomName {
        RoomName::try_from(name.to_owned()).expect("valid room")
    }

    fn thermometers() -> Vec<Thermometer> {
        [(MAC_BEDROOM, "bedroom"), (MAC_OFFICE, "office")]
            .into_iter()
            .map(|(mac, name)| Thermometer {
                mac,
                room: room(name),
                temperature_range: None,
            })
            .collect()
    }

    fn selection(rooms: &[&str], include_outside: bool) -> HistorySelection {
        HistorySelection {
            metric: Metric::Temperature,
            period: Period::Day,
            rooms: rooms.iter().map(|name| room(name)).collect(),
            include_outside,
        }
    }

    fn ble(mac: MacAddress, micros: i64, value: f64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(micros),
            source_kind: SourceKind::Ble,
            source_id: mac.into(),
            metric: Metric::Temperature,
            value,
        }
    }

    fn outside(micros: i64, value: f64) -> Measurement {
        Measurement {
            source_kind: SourceKind::Weather,
            source_id: SourceId::from("open-meteo".to_owned()),
            ..ble(MAC_BEDROOM, micros, value)
        }
    }

    fn chart(selection: &HistorySelection, now: i64, rows: &[Measurement]) -> super::HistoryChart {
        build_history(
            &thermometers(),
            selection,
            Timestamp::from_micros(now),
            rows,
        )
    }

    #[test]
    fn an_aligned_day_has_96_buckets_and_an_unaligned_one_97() {
        let aligned = chart(&selection(&[], false), ALIGNED_NOW, &[]);
        assert_eq!(aligned.timestamps.len(), 96);
        assert_eq!(aligned.timestamps[0], ALIGNED_NOW / 1_000_000 - 86_400);
        let unaligned = chart(&selection(&[], false), ALIGNED_NOW + 1, &[]);
        assert_eq!(unaligned.timestamps.len(), 97);
        assert!(unaligned.timestamps.iter().all(|ts| ts % 900 == 0));
    }

    #[test]
    fn rooms_follow_the_configuration_then_outside() {
        let selected = selection(&["office", "bedroom"], true);
        let chart = chart(&selected, ALIGNED_NOW, &[]);
        assert_eq!(chart.labels, ["bedroom", "office", "Outside"]);
        assert_eq!(chart.lines.len(), 3);
    }

    #[test]
    fn unselected_and_unknown_rooms_are_absent() {
        let selected = selection(&["office", "garage"], false);
        assert_eq!(chart(&selected, ALIGNED_NOW, &[]).labels, ["office"]);
    }

    #[test]
    fn outside_is_absent_unless_selected() {
        let chart = chart(&selection(&["bedroom"], false), ALIGNED_NOW, &[]);
        assert_eq!(chart.labels, ["bedroom"]);
    }

    #[test]
    fn a_value_lands_in_its_bucket_and_other_buckets_are_gaps() {
        let first = ALIGNED_NOW - 86_400 * 1_000_000;
        let rows = [
            ble(MAC_BEDROOM, first, 20.0),
            ble(MAC_BEDROOM, first + 900_000_000 * 5, 21.5),
            ble(MAC_STRANGER, first, 99.0),
            ble(MAC_OFFICE, first, 18.0),
            outside(first + 900_000_000, 4.0),
        ];
        let chart = chart(&selection(&["bedroom"], true), ALIGNED_NOW, &rows);
        let bedroom = &chart.lines[0];
        assert_eq!(bedroom[0], Some(20.0));
        assert_eq!(bedroom[5], Some(21.5));
        assert_eq!(bedroom.iter().flatten().count(), 2);
        let outside_line = &chart.lines[1];
        assert_eq!(outside_line[1], Some(4.0));
        assert_eq!(outside_line.iter().flatten().count(), 1);
    }

    #[test]
    fn rows_outside_the_grid_are_ignored() {
        let rows = [
            ble(MAC_BEDROOM, ALIGNED_NOW - 90_000_000_000, 1.0),
            ble(MAC_BEDROOM, ALIGNED_NOW, 2.0),
        ];
        let chart = chart(&selection(&["bedroom"], false), ALIGNED_NOW, &rows);
        assert_eq!(chart.lines[0].iter().flatten().count(), 0);
    }

    #[test]
    fn every_period_has_a_small_grid_and_a_matching_query() {
        assert_eq!(HISTORY_METRICS, [Metric::Temperature, Metric::Humidity]);
        let now = Timestamp::from_micros(ALIGNED_NOW + 12_345);
        let expected = [
            ("24h", 86_400, 900),
            ("7d", 7 * 86_400, 1_800),
            ("30d", 30 * 86_400, 7_200),
            ("1y", 365 * 86_400, 86_400),
        ];
        for (period, (name, span, bucket)) in Period::ALL.into_iter().zip(expected) {
            assert_eq!(period.as_str(), name);
            assert_eq!(period.span_secs(), span);
            assert_eq!(period.bucket_secs().get(), bucket);
            let selected = HistorySelection {
                period,
                ..selection(&[], false)
            };
            let grid = build_history(&thermometers(), &selected, now, &[]).timestamps;
            assert!(grid.len() <= 400, "{name}: {}", grid.len());
            let query = history_query(&selected, now);
            assert_eq!(query.metric, Metric::Temperature);
            assert_eq!(query.source_id, None);
            assert_eq!(query.to, now);
            assert_eq!(query.bucket_secs, Some(period.bucket_secs()));
            assert_eq!(query.from.as_micros(), grid[0] * 1_000_000);
        }
    }
}

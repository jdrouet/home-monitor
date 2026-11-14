use serde::{Deserialize, Serialize};

/// A measured quantity. Units are implied: degrees Celsius, percent relative
/// humidity and percent battery charge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Temperature,
    Humidity,
    Battery,
}

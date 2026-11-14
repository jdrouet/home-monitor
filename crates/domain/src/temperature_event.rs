use crate::Deviation;

/// A temperature notification a room is owed; `celsius` is the latest known.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TemperatureEvent {
    Onset { deviation: Deviation, celsius: f64 },
    Reminder { deviation: Deviation, celsius: f64 },
    Resolved { celsius: f64 },
}

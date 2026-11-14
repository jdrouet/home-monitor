use std::path::PathBuf;

use domain::{MacAddress, RoomName};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config {}: {source}", path.display())]
    InFile {
        path: PathBuf,
        source: Box<ConfigError>,
    },
    #[error(transparent)]
    Parse(#[from] toml::de::Error),
    #[error("thermometer MAC {0} is configured more than once")]
    DuplicateMac(MacAddress),
    #[error("room {0} has more than one thermometer")]
    DuplicateRoom(RoomName),
    #[error("weather latitude {0} is outside -90 to 90")]
    InvalidLatitude(f64),
    #[error("weather longitude {0} is outside -180 to 180")]
    InvalidLongitude(f64),
}

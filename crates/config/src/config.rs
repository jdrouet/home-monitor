use std::collections::HashSet;
use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use domain::Thermometer;
use serde::Deserialize;

use crate::ConfigError;

const PATH_ENV: &str = "HOME_MONITOR_CONFIG";
const DEFAULT_PATH: &str = "config.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub ble_collector: ServiceConfig,
    pub store: StoreConfig,
    pub dashboard: DashboardConfig,
    pub alerter: AlerterConfig,
    pub thermometers: Vec<Thermometer>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    pub http_listen: SocketAddr,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreConfig {
    pub http_listen: SocketAddr,
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardConfig {
    pub http_listen: SocketAddr,
    pub store_url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlerterConfig {
    pub http_listen: SocketAddr,
    pub ntfy_url: String,
}

impl FromStr for Config {
    type Err = ConfigError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }
}

impl Config {
    fn validate(&self) -> Result<(), ConfigError> {
        let mut macs = HashSet::new();
        let mut rooms = HashSet::new();
        for thermometer in &self.thermometers {
            if !macs.insert(thermometer.mac) {
                return Err(ConfigError::DuplicateMac(thermometer.mac));
            }
            if !rooms.insert(&thermometer.room) {
                return Err(ConfigError::DuplicateRoom(thermometer.room.clone()));
            }
        }
        Ok(())
    }
}

/// Loads the file named by `HOME_MONITOR_CONFIG`, or `config.toml` when unset.
pub fn load() -> Result<Config, ConfigError> {
    let path = env::var_os(PATH_ENV).map_or_else(|| PathBuf::from(DEFAULT_PATH), PathBuf::from);
    load_from(&path)
}

fn load_from(path: &Path) -> Result<Config, ConfigError> {
    let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })?;
    text.parse().map_err(|source| ConfigError::InFile {
        path: path.to_owned(),
        source: Box::new(source),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::ConfigError;

    use super::{Config, load_from};

    const EXAMPLE: &str = include_str!("../../../config.example.toml");

    fn example_without(section: &str) -> String {
        EXAMPLE.replace(section, "")
    }

    #[test]
    fn example_parses() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        let rooms: Vec<_> = config
            .thermometers
            .iter()
            .map(|thermometer| thermometer.room.as_str())
            .collect();
        assert_eq!(rooms, ["bedroom", "office", "living room"]);
        assert_eq!(config.mqtt.host, "mosquitto");
        assert_eq!(config.mqtt.port, 1883);
        assert_eq!(config.dashboard.store_url, "http://store:8080");
        assert_eq!(config.store.data_dir, Path::new("/data"));
        assert_eq!(config.alerter.ntfy_url, "http://ntfy");
    }

    #[test]
    fn duplicate_mac_is_rejected() {
        let text = EXAMPLE.replace("A4:C1:38:E1:6F:B2", "A4:C1:38:1C:02:76");
        assert!(matches!(
            text.parse::<Config>(),
            Err(ConfigError::DuplicateMac(_))
        ));
    }

    #[test]
    fn duplicate_room_is_rejected() {
        let text = EXAMPLE.replace("\"office\"", "\"bedroom\"");
        assert!(matches!(
            text.parse::<Config>(),
            Err(ConfigError::DuplicateRoom(_))
        ));
    }

    #[test]
    fn malformed_values_are_rejected() {
        for text in [
            EXAMPLE.replace("A4:C1:38:E1:6F:B2", "not-a-mac"),
            EXAMPLE.replace("\"office\"", "\"\""),
            format!("unknown = 1\n{EXAMPLE}"),
            example_without("data_dir = \"/data\"\n"),
            example_without("[mqtt]\nhost = \"mosquitto\"\nport = 1883\n"),
            example_without(
                "[alerter]\n# The ntfy topic is a secret: set NTFY_TOPIC in .env.\nhttp_listen = \"0.0.0.0:8080\"\nntfy_url = \"http://ntfy\"\n\n",
            ),
        ] {
            assert!(matches!(text.parse::<Config>(), Err(ConfigError::Parse(_))));
        }
    }

    #[test]
    fn load_reports_the_path() {
        let missing = Path::new("/nonexistent/config.toml");
        let error = load_from(missing).expect_err("file is missing");
        assert!(matches!(error, ConfigError::Read { .. }));
        assert!(error.to_string().contains("/nonexistent/config.toml"));
    }

    #[test]
    fn load_reads_a_file() {
        let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config.example.toml");
        assert_eq!(load_from(&example).expect("loads").thermometers.len(), 3);
    }

    #[test]
    fn invalid_file_names_the_path() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let error = load_from(&manifest).expect_err("not a config");
        assert!(matches!(error, ConfigError::InFile { .. }));
        assert!(error.to_string().contains("Cargo.toml"));
    }
}

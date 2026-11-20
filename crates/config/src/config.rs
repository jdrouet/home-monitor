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
const DEFAULT_STATE_FILE: &str = "/data/alerts.json";
const DEFAULT_TOKEN_FILE: &str = "/tokens/garmin_tokens.json";
const DEFAULT_ARCHIVE_DIR: &str = "/archive/garmin";
const DEFAULT_ALERTER_URL: &str = "http://alerter:8080";
const DEFAULT_HEALTH_COLLECTOR_URL: &str = "http://health-collector:8080";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub ble_collector: ServiceConfig,
    pub store: StoreConfig,
    pub dashboard: DashboardConfig,
    pub alerter: AlerterConfig,
    pub weather_collector: Option<WeatherCollectorConfig>,
    pub health_collector: Option<HealthCollectorConfig>,
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
    #[serde(default = "default_alerter_url")]
    pub alerter_url: String,
    #[serde(default = "default_health_collector_url")]
    pub health_collector_url: String,
}

fn default_alerter_url() -> String {
    DEFAULT_ALERTER_URL.to_owned()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlerterConfig {
    pub http_listen: SocketAddr,
    pub ntfy_url: String,
    #[serde(default = "default_state_file")]
    pub state_file: PathBuf,
    #[serde(default = "default_health_collector_url")]
    pub health_collector_url: String,
}

fn default_health_collector_url() -> String {
    DEFAULT_HEALTH_COLLECTOR_URL.to_owned()
}

fn default_state_file() -> PathBuf {
    PathBuf::from(DEFAULT_STATE_FILE)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherCollectorConfig {
    pub http_listen: SocketAddr,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthCollectorConfig {
    pub http_listen: SocketAddr,
    #[serde(default = "default_token_file")]
    pub token_file: PathBuf,
    #[serde(default = "default_archive_dir")]
    pub archive_dir: PathBuf,
}

fn default_token_file() -> PathBuf {
    PathBuf::from(DEFAULT_TOKEN_FILE)
}

fn default_archive_dir() -> PathBuf {
    PathBuf::from(DEFAULT_ARCHIVE_DIR)
}

impl WeatherCollectorConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if !(-90.0..=90.0).contains(&self.latitude) {
            return Err(ConfigError::InvalidLatitude(self.latitude));
        }
        if !(-180.0..=180.0).contains(&self.longitude) {
            return Err(ConfigError::InvalidLongitude(self.longitude));
        }
        Ok(())
    }
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
        if let Some(weather) = &self.weather_collector {
            weather.validate()?;
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

    const WEATHER_SECTION: &str = "[weather_collector]\nhttp_listen = \"0.0.0.0:8080\"\n# Outdoor weather location (Open-Meteo), in decimal degrees.\nlatitude = 48.8566\nlongitude = 2.3522\n\n";

    const HEALTH_SECTION: &str = "[health_collector]\n# Garmin tokens come from `health-collector login`.\nhttp_listen = \"0.0.0.0:8080\"\n# archive_dir = \"/archive/garmin\"\n\n";

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
    fn alerter_url_defaults_to_the_alerter_service() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        assert_eq!(config.dashboard.alerter_url, "http://alerter:8080");
    }

    #[test]
    fn alerter_url_can_be_set() {
        let text = EXAMPLE.replace(
            "store_url = \"http://store:8080\"\n",
            "store_url = \"http://store:8080\"\nalerter_url = \"http://other:9000\"\n",
        );
        let config: Config = text.parse().expect("config is valid");
        assert_eq!(config.dashboard.alerter_url, "http://other:9000");
    }

    #[test]
    fn dashboard_health_collector_url_defaults_and_can_be_set() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        assert_eq!(
            config.dashboard.health_collector_url,
            "http://health-collector:8080"
        );
        let text = EXAMPLE.replace(
            "store_url = \"http://store:8080\"\n",
            "store_url = \"http://store:8080\"\nhealth_collector_url = \"http://other:9000\"\n",
        );
        let config: Config = text.parse().expect("config is valid");
        assert_eq!(config.dashboard.health_collector_url, "http://other:9000");
    }

    #[test]
    fn health_collector_url_defaults_to_the_collector_service() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        assert_eq!(
            config.alerter.health_collector_url,
            "http://health-collector:8080"
        );
    }

    #[test]
    fn health_collector_url_can_be_set() {
        let text = EXAMPLE.replace(
            "ntfy_url = \"http://ntfy\"\n",
            "ntfy_url = \"http://ntfy\"\nhealth_collector_url = \"http://other:9000\"\n",
        );
        let config: Config = text.parse().expect("config is valid");
        assert_eq!(config.alerter.health_collector_url, "http://other:9000");
    }

    #[test]
    fn example_has_a_weather_location() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        let weather = config.weather_collector.expect("section is present");
        assert_eq!(weather.http_listen.port(), 8080);
        assert_eq!(weather.latitude, 48.8566);
        assert_eq!(weather.longitude, 2.3522);
    }

    #[test]
    fn example_has_a_health_collector_with_the_default_token_file() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        let health = config.health_collector.expect("section is present");
        assert_eq!(health.http_listen.port(), 8080);
        assert_eq!(health.token_file, Path::new("/tokens/garmin_tokens.json"));
    }

    #[test]
    fn token_file_can_be_set() {
        let text = EXAMPLE.replace(
            "[health_collector]\n",
            "[health_collector]\ntoken_file = \"/var/garmin.json\"\n",
        );
        let config: Config = text.parse().expect("config is valid");
        let health = config.health_collector.expect("section is present");
        assert_eq!(health.token_file, Path::new("/var/garmin.json"));
    }

    #[test]
    fn example_has_the_default_archive_dir() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        let health = config.health_collector.expect("section is present");
        assert_eq!(health.archive_dir, Path::new("/archive/garmin"));
    }

    #[test]
    fn archive_dir_can_be_set() {
        let text = EXAMPLE.replace(
            "[health_collector]\n",
            "[health_collector]\narchive_dir = \"/mnt/garmin\"\n",
        );
        let config: Config = text.parse().expect("config is valid");
        let health = config.health_collector.expect("section is present");
        assert_eq!(health.archive_dir, Path::new("/mnt/garmin"));
    }

    #[test]
    fn health_section_is_optional() {
        let text = example_without(HEALTH_SECTION);
        let config: Config = text.parse().expect("config is valid");
        assert!(config.health_collector.is_none());
    }

    #[test]
    fn unknown_health_key_is_rejected() {
        let text = EXAMPLE.replace(
            "[health_collector]\n",
            "[health_collector]\nemail = \"a\"\n",
        );
        assert!(matches!(text.parse::<Config>(), Err(ConfigError::Parse(_))));
    }

    #[test]
    fn state_file_defaults_to_the_data_volume() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        assert_eq!(config.alerter.state_file, Path::new("/data/alerts.json"));
    }

    #[test]
    fn state_file_can_be_set() {
        let text = EXAMPLE.replace(
            "ntfy_url = \"http://ntfy\"\n",
            "ntfy_url = \"http://ntfy\"\nstate_file = \"/var/alerts.json\"\n",
        );
        let config: Config = text.parse().expect("config is valid");
        assert_eq!(config.alerter.state_file, Path::new("/var/alerts.json"));
    }

    #[test]
    fn example_sets_a_range_per_room() {
        let config: Config = EXAMPLE.parse().expect("example is valid");
        let ranges: Vec<_> = config
            .thermometers
            .iter()
            .map(|thermometer| {
                let range = thermometer.temperature_range.expect("range is set");
                (range.min(), range.max())
            })
            .collect();
        assert_eq!(ranges, [(18.0, 20.0), (19.0, 22.0), (19.0, 22.0)]);
    }

    #[test]
    fn range_is_optional() {
        let text = EXAMPLE.replace("min_temperature = 18.0\nmax_temperature = 20.0\n", "");
        let config: Config = text.parse().expect("config is valid");
        assert!(config.thermometers[0].temperature_range.is_none());
        assert!(config.thermometers[1].temperature_range.is_some());
    }

    #[test]
    fn invalid_ranges_are_rejected() {
        for text in [
            EXAMPLE.replace("min_temperature = 18.0\n", ""),
            EXAMPLE.replace("max_temperature = 20.0\n", ""),
            EXAMPLE.replace("min_temperature = 18.0", "min_temperature = 20.0"),
            EXAMPLE.replace("min_temperature = 18.0", "min_temperature = 25.0"),
            EXAMPLE.replace("min_temperature = 18.0", "min_temperature = nan"),
            EXAMPLE.replace("max_temperature = 20.0", "max_temperature = inf"),
            EXAMPLE.replace("min_temperature = 18.0", "minimum = 18.0"),
        ] {
            assert!(matches!(text.parse::<Config>(), Err(ConfigError::Parse(_))));
        }
    }

    #[test]
    fn weather_section_is_optional() {
        let text = example_without(WEATHER_SECTION);
        let config: Config = text.parse().expect("config is valid");
        assert!(config.weather_collector.is_none());
    }

    #[test]
    fn out_of_range_latitude_is_rejected() {
        for latitude in ["91", "-90.5", "nan"] {
            let text = EXAMPLE.replace("48.8566", latitude);
            assert!(matches!(
                text.parse::<Config>(),
                Err(ConfigError::InvalidLatitude(_))
            ));
        }
    }

    #[test]
    fn out_of_range_longitude_is_rejected() {
        for longitude in ["180.5", "-181", "nan"] {
            let text = EXAMPLE.replace("2.3522", longitude);
            assert!(matches!(
                text.parse::<Config>(),
                Err(ConfigError::InvalidLongitude(_))
            ));
        }
    }

    #[test]
    fn boundary_coordinates_are_accepted() {
        let text = EXAMPLE.replace("48.8566", "90").replace("2.3522", "-180");
        assert!(text.parse::<Config>().is_ok());
    }

    #[test]
    fn unknown_weather_key_is_rejected() {
        let text = EXAMPLE.replace("latitude = ", "altitude = 1\nlatitude = ");
        assert!(matches!(text.parse::<Config>(), Err(ConfigError::Parse(_))));
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

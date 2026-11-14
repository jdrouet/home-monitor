use crate::{SourceId, SourceKind};

pub const MEASUREMENT_TOPIC_FILTER: &str = "measurements/#";

pub fn measurement_topic(source_kind: SourceKind, source_id: &SourceId) -> String {
    format!(
        "measurements/{}/{}",
        source_kind.as_str(),
        source_id.as_str()
    )
}

#[cfg(test)]
mod tests {
    use crate::{MacAddress, SourceKind};

    use super::measurement_topic;

    #[test]
    fn topic_names_kind_and_source() {
        let address: MacAddress = "A4:C1:38:1C:02:76".parse().expect("valid address");
        assert_eq!(
            measurement_topic(SourceKind::Ble, &address.into()),
            "measurements/ble/A4:C1:38:1C:02:76"
        );
    }
}

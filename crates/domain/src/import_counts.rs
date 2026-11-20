use serde::{Deserialize, Serialize};

/// What one attempt did, in the job's unit (days or workouts).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportCounts {
    pub stored: u32,
    pub empty: u32,
    pub failed: u32,
}

#[cfg(test)]
mod tests {
    use super::ImportCounts;

    #[test]
    fn missing_fields_read_as_zero() {
        let counts: ImportCounts = serde_json::from_str(r#"{"failed":1}"#).unwrap();
        assert_eq!(
            counts,
            ImportCounts {
                failed: 1,
                ..ImportCounts::default()
            }
        );
    }
}

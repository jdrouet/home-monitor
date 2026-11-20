use serde::{Deserialize, Serialize};

/// What one attempt did, in the job's unit (days or workouts).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCounts {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub stored: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub empty: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub failed: u32,
}

fn is_zero(count: &u32) -> bool {
    *count == 0
}

impl ImportCounts {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::ImportCounts;

    #[test]
    fn zero_fields_are_not_written() {
        let counts = ImportCounts {
            stored: 2,
            ..ImportCounts::default()
        };
        assert_eq!(serde_json::to_string(&counts).unwrap(), r#"{"stored":2}"#);
        assert_eq!(
            serde_json::to_string(&ImportCounts::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn missing_fields_read_as_zero() {
        let counts: ImportCounts = serde_json::from_str(r#"{"failed":1}"#).unwrap();
        assert_eq!(counts.failed, 1);
        assert!(!counts.is_empty());
        assert!(ImportCounts::default().is_empty());
    }
}

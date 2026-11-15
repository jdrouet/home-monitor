use std::fmt;

/// Garmin's number for one recorded activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActivityId(pub u64);

impl fmt::Display for ActivityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::ActivityId;

    #[test]
    fn it_displays_as_the_bare_number() {
        assert_eq!(ActivityId(9_876_543_210).to_string(), "9876543210");
    }
}

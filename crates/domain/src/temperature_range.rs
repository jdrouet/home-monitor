use crate::{Deviation, DomainError};

/// The temperatures, in °C, a room may stay within; both limits are allowed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemperatureRange {
    min: f64,
    max: f64,
}

impl TemperatureRange {
    pub fn new(min: f64, max: f64) -> Result<Self, DomainError> {
        if min.is_finite() && max.is_finite() && min < max {
            Ok(Self { min, max })
        } else {
            Err(DomainError::InvalidTemperatureRange)
        }
    }

    pub const fn min(&self) -> f64 {
        self.min
    }

    pub const fn max(&self) -> f64 {
        self.max
    }

    /// The side of the range `celsius` is outside of, if any.
    pub fn deviation(&self, celsius: f64) -> Option<Deviation> {
        if celsius < self.min {
            Some(Deviation::TooCold)
        } else if celsius > self.max {
            Some(Deviation::TooHot)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Deviation, DomainError};

    use super::TemperatureRange;

    fn range() -> TemperatureRange {
        TemperatureRange::new(18.0, 20.0).expect("valid range")
    }

    #[test]
    fn limits_are_inside_the_range() {
        assert_eq!(range().deviation(18.0), None);
        assert_eq!(range().deviation(19.0), None);
        assert_eq!(range().deviation(20.0), None);
    }

    #[test]
    fn beyond_a_limit_names_the_side() {
        assert_eq!(range().deviation(17.9), Some(Deviation::TooCold));
        assert_eq!(range().deviation(20.1), Some(Deviation::TooHot));
    }

    #[test]
    fn exposes_the_limits() {
        assert_eq!((range().min(), range().max()), (18.0, 20.0));
    }

    #[test]
    fn invalid_ranges_are_rejected() {
        for (min, max) in [
            (20.0, 18.0),
            (20.0, 20.0),
            (f64::NAN, 20.0),
            (18.0, f64::INFINITY),
        ] {
            assert_eq!(
                TemperatureRange::new(min, max),
                Err(DomainError::InvalidTemperatureRange)
            );
        }
    }
}

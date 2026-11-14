use domain::Metric;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub metric: Metric,
    pub value: f64,
}

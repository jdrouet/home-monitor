use std::future::Future;

use domain::{Measurement, SourceId, SourceKind};

pub trait Publisher {
    type Error: std::error::Error + Send + Sync + 'static;

    fn publish(
        &self,
        source_kind: SourceKind,
        source_id: &SourceId,
        measurements: &[Measurement],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

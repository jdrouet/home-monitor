use std::future::Future;

use crate::Advertisement;

pub trait AdvertisementSource {
    /// Resolves to `None` when no more advertisements will arrive. The
    /// future must be cancel-safe: dropping it must not lose an advertisement.
    fn next_advertisement(&mut self) -> impl Future<Output = Option<Advertisement>> + Send;
}

use std::future::Future;

use crate::Advertisement;

pub trait AdvertisementSource {
    /// Resolves to `None` when no more advertisements will arrive. Dropping
    /// the future may lose at most the advertisement being read.
    fn next_advertisement(&mut self) -> impl Future<Output = Option<Advertisement>> + Send;
}

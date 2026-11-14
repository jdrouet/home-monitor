use std::time::Duration;

/// A notification an offline watch asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfflineEvent {
    Offline { silent_for: Duration },
    StillOffline { silent_for: Duration },
    Back,
}

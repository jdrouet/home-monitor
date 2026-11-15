/// A notification a login watch asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginEvent {
    Expired,
    StillExpired,
    Back,
}

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Millis(pub u32);

impl Millis {
    pub const fn mins(n: u32) -> Self {
        Self(n * 60_000)
    }

    pub const fn hours(n: u32) -> Self {
        Self::mins(n * 60)
    }

    pub const fn days(n: u32) -> Self {
        Self::hours(n * 24)
    }

    pub fn from_expire_timeout(ms: i32) -> Option<Self> {
        // TODO: Handle default expiry
        // -1 -> default expiry
        // 0 -> never expires
        (ms > 0).then(|| Self(ms as u32))
    }

    pub fn duration(self) -> Duration {
        Duration::from_millis(self.0 as u64)
    }
}

use std::time::Duration;

/// Every gesture and timing number used by nav.
#[derive(Clone, Debug, PartialEq)]
pub struct NavConfig {
    /// Minimum travel for quick/held swipes (bottom edge).
    pub min_travel_px: f32,
    pub hold_time: Duration,
    pub flick_px_per_ms: f32,
    /// Travel at which `Timing::Any` swipes commit.
    pub swipe_commit_px: f32,
    pub axis_lock_px: f32,
    pub velocity_window: Duration,
    pub velocity_stale: Duration,
    pub velocity_min_span: Duration,
    pub fade: Duration,
    pub bottom_strip_height: f32,
    /// Width of each corner [`Span`](super::Span), as a fraction of the
    /// area's width.
    pub corner_fraction: f32,
}

impl Default for NavConfig {
    fn default() -> Self {
        Self {
            min_travel_px: 56.0,
            hold_time: Duration::from_millis(220),
            flick_px_per_ms: 0.9,
            swipe_commit_px: 90.0,
            axis_lock_px: 10.0,
            velocity_window: Duration::from_millis(80),
            velocity_stale: Duration::from_millis(80),
            velocity_min_span: Duration::from_millis(8),
            fade: Duration::from_millis(160),
            bottom_strip_height: 5.0,
            corner_fraction: 0.25,
        }
    }
}

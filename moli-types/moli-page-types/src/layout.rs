/// Controls when Moli updates layout and publishes screen geometry.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LayoutPolicy {
    /// Preserve the pre-layout compatibility behavior and reject operations
    /// such as renderer screenshots that require a real layout backend.
    #[default]
    Mock,
    /// Geometry queries and input initialize a missing screen snapshot, then
    /// keep reading it until an explicit refresh. Screenshots and screencasts
    /// publish layout. Print projections are temporary.
    Lazy,
    /// Keep observable screen geometry current and advance the rendering
    /// lifecycle independently of pixel consumers. Uses the same layout
    /// backend as `Lazy`. Image and font resource policies are separate.
    Realtime,
}

impl LayoutPolicy {
    pub const fn uses_real_layout(self) -> bool {
        matches!(self, Self::Lazy | Self::Realtime)
    }

    pub const fn is_realtime(self) -> bool {
        matches!(self, Self::Realtime)
    }
}

/// Default timer cadence for realtime pages without a display clock.
pub const DEFAULT_REALTIME_MAX_FPS: u32 = 60;

/// Immutable browser layout configuration, sealed before its first Page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayoutConfiguration {
    pub policy: LayoutPolicy,
    pub scrollbars_hidden: bool,
    /// Maximum timer cadence for realtime rendering. Zero removes the limit.
    pub realtime_max_fps: u32,
}

impl Default for LayoutConfiguration {
    fn default() -> Self {
        Self {
            policy: LayoutPolicy::default(),
            scrollbars_hidden: false,
            realtime_max_fps: DEFAULT_REALTIME_MAX_FPS,
        }
    }
}

impl LayoutConfiguration {
    pub const fn scrollbars_hidden_for(self, emulated_hidden: bool) -> bool {
        self.scrollbars_hidden || emulated_hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_policy_defaults_to_mock() {
        assert_eq!(LayoutPolicy::default(), LayoutPolicy::Mock);
        assert!(!LayoutPolicy::default().uses_real_layout());
        assert!(LayoutPolicy::Lazy.uses_real_layout());
        assert!(LayoutPolicy::Realtime.uses_real_layout());
        assert!(!LayoutPolicy::Mock.uses_real_layout());
        assert!(!LayoutPolicy::Mock.is_realtime());
        assert!(!LayoutPolicy::Lazy.is_realtime());
        assert!(LayoutPolicy::Realtime.is_realtime());
    }
}

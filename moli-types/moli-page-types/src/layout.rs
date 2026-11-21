/// Selects whether layout-dependent operations use Moli's deterministic
/// compatibility geometry or construct a real one-shot layout pass.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LayoutPolicy {
    /// Preserve the pre-layout compatibility behavior and reject operations
    /// such as renderer screenshots that require a real layout backend.
    #[default]
    Mock,
    /// Geometry queries and input initialize a missing screen snapshot, then
    /// keep reading it until an explicit refresh. Screenshots and screencasts
    /// publish layout; print projections are temporary.
    OnDemand,
    /// Keep observable screen geometry current and advance the rendering
    /// lifecycle independently of pixel consumers. Uses the same layout
    /// backend as OnDemand; image and font resource policies remain separate.
    Realtime,
}

impl LayoutPolicy {
    pub const fn uses_real_layout(self) -> bool {
        matches!(self, Self::OnDemand | Self::Realtime)
    }

    pub const fn is_realtime(self) -> bool {
        matches!(self, Self::Realtime)
    }
}

/// Immutable browser layout configuration, sealed before its first Page.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LayoutConfiguration {
    pub policy: LayoutPolicy,
    pub scrollbars_hidden: bool,
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
        assert!(LayoutPolicy::OnDemand.uses_real_layout());
        assert!(LayoutPolicy::Realtime.uses_real_layout());
        assert!(!LayoutPolicy::Mock.uses_real_layout());
        assert!(!LayoutPolicy::Mock.is_realtime());
        assert!(!LayoutPolicy::OnDemand.is_realtime());
        assert!(LayoutPolicy::Realtime.is_realtime());
    }
}

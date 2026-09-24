/// Selects whether layout-dependent operations use Moli's deterministic
/// compatibility geometry or construct a real one-shot layout pass.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LayoutPolicy {
    /// Preserve the pre-layout compatibility behavior and reject operations
    /// such as renderer screenshots that require a real layout backend.
    #[default]
    Mock,
    /// Only screenshot, screencast, and PDF output build layout.
    /// Screenshots and screencasts publish layout; print projections are temporary.
    /// Geometry queries and input read the last published layout, if any.
    OnDemand,
}

impl LayoutPolicy {
    pub const fn uses_real_layout(self) -> bool {
        matches!(self, Self::OnDemand)
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
        assert!(!LayoutPolicy::Mock.uses_real_layout());
    }
}

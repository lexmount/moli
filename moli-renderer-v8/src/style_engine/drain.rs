#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::style_engine) enum StyleInvalidationDrainBoundary {
    ComputedStyleRead,
    RuntimeEvaluate,
    RuntimePendingWorkFlush,
    SelectedPageTask,
    NonScriptPageTask,
    #[cfg(test)]
    TestExplicit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StyleInvalidationTurnExitBoundary {
    RuntimeEvaluate,
    RuntimePendingWorkFlush,
    SelectedPageTask,
    NonScriptPageTask,
}

impl From<StyleInvalidationTurnExitBoundary> for StyleInvalidationDrainBoundary {
    fn from(boundary: StyleInvalidationTurnExitBoundary) -> Self {
        match boundary {
            StyleInvalidationTurnExitBoundary::RuntimeEvaluate => Self::RuntimeEvaluate,
            StyleInvalidationTurnExitBoundary::RuntimePendingWorkFlush => {
                Self::RuntimePendingWorkFlush
            }
            StyleInvalidationTurnExitBoundary::SelectedPageTask => Self::SelectedPageTask,
            StyleInvalidationTurnExitBoundary::NonScriptPageTask => Self::NonScriptPageTask,
        }
    }
}

use std::sync::Arc;

/// Original, one-based source coordinates, independent of the current DOM owner.
/// Zero denotes a coordinate unavailable to the parser or embedding runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    pub resource_name: Arc<str>,
    pub line: u64,
    pub column: u64,
}

impl SourceLocation {
    pub fn new(resource_name: impl Into<Arc<str>>, line: u64, column: u64) -> Self {
        Self {
            resource_name: resource_name.into(),
            line,
            column,
        }
    }
}

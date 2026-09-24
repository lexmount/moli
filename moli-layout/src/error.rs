use thiserror::Error;

/// A structured failure while constructing or evaluating one layout pass.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum LayoutError {
    /// Geometry and input consume published layout and cannot create it.
    #[error(
        "no published layout is available for the current document; call Page.captureScreenshot or Page.printToPDF, or start Page.startScreencast and wait for a frame, then retry. Geometry queries and input do not create layout"
    )]
    NoLayoutSnapshot,
    /// A geometry consumer forced layout before the Document had an element root.
    #[error("current document has no element root to lay out")]
    NoLayoutRoot,
    /// A layout demand attempted to enter while another synchronous pass was active.
    #[error("a synchronous layout pass cannot re-enter itself")]
    ReentrantLayoutPass,
    /// A consumer requested paint data from a geometry-only output.
    #[error("this layout pass did not request a paint projection")]
    PaintProjectionNotRequested,
    /// A screenshot or screencast requested an invalid capture surface.
    #[error("invalid paint capture: {detail}")]
    InvalidPaintCapture { detail: String },
    /// One frozen layout tree is too large to publish or retain.
    #[error(
        "frozen layout tree contains {boxes} boxes, {fragments} fragments, and an estimated {estimated_bytes} bytes; limits are {max_boxes} boxes, {max_fragments} fragments, and {max_bytes} bytes"
    )]
    TreeRetentionBudgetExceeded {
        boxes: usize,
        fragments: usize,
        estimated_bytes: usize,
        max_boxes: usize,
        max_fragments: usize,
        max_bytes: usize,
    },
    /// The source root did not resolve a primary computed style.
    #[error("layout root {source_label} has no computed style")]
    MissingRootStyle { source_label: String },
    /// The source/flat-tree view exposed a cycle.
    #[error("layout source flat tree contains a cycle at {source_label}")]
    SourceCycle { source_label: String },
    /// The source view violated its flat-tree or element-semantics contract.
    #[error("invalid layout source {source_label}: {detail}")]
    SourceContract {
        source_label: String,
        detail: String,
    },
    /// A style resolver rejected a source or pseudo query.
    #[error("failed to resolve layout style for {source_label}: {detail}")]
    StyleResolution {
        source_label: String,
        detail: String,
    },
    /// An internal box reference did not belong to the current pass.
    #[error("layout box index {index} does not belong to this pass")]
    InvalidBoxReference { index: usize },
    /// The numeric parent relation violated the pass-local tree contract.
    #[error("numeric layout parent relation contains a cycle at box index {index}")]
    NumericTreeCycle { index: usize },
}

impl LayoutError {
    /// Creates a style-resolution error with owned diagnostic context.
    pub fn style_resolution(source: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::StyleResolution {
            source_label: source.into(),
            detail: detail.into(),
        }
    }

    /// Creates a source-contract error with deterministic owned context.
    pub fn source_contract(source: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::SourceContract {
            source_label: source.into(),
            detail: detail.into(),
        }
    }
}

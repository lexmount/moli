/// Document-wide runtime inputs for one layout demand.
///
/// These values are independent of authored CSS and apply to every generated
/// box, including pseudo elements and anonymous boxes. The renderer supplies
/// the same target policy when composing embedded documents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutEnvironment {
    /// Removes scrollbar controls and automatic gutters. Authored stable
    /// gutters still reserve space using the CSS scrollbar width.
    pub scrollbars_hidden: bool,
}

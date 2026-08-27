use std::{fmt::Debug, hash::Hash, sync::Arc};

use crate::{LayoutError, ResolvedLayoutStyle};

/// Source-node category needed by CSS box construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutSourceKind {
    Element,
    Text,
    Comment,
    Other,
}

/// Namespace family needed by box construction and diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LayoutNamespace {
    Html,
    Svg,
    MathMl,
    Other(Arc<str>),
}

impl LayoutNamespace {
    pub const HTML_URI: &'static str = "http://www.w3.org/1999/xhtml";
    pub const SVG_URI: &'static str = "http://www.w3.org/2000/svg";
    pub const MATHML_URI: &'static str = "http://www.w3.org/1998/Math/MathML";

    /// Classifies one canonical DOM namespace URI without retaining the DOM.
    pub fn from_uri(namespace: &str) -> Self {
        match namespace {
            Self::HTML_URI => Self::Html,
            Self::SVG_URI => Self::Svg,
            Self::MATHML_URI => Self::MathMl,
            other => Self::Other(Arc::from(other)),
        }
    }

    pub fn debug_name(&self) -> &str {
        match self {
            Self::Html => "html",
            Self::Svg => "svg",
            Self::MathMl => "mathml",
            Self::Other(namespace) => namespace,
        }
    }
}

/// HTML semantic role that affects later box construction or intrinsic sizing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayoutElementCategory {
    #[default]
    Generic,
    LineBreak,
    Table(LayoutTableRole),
    List(LayoutListRole),
    FormControl(LayoutFormControlKind),
}

impl LayoutElementCategory {
    pub const fn debug_name(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::LineBreak => "line-break",
            Self::Table(role) => role.debug_name(),
            Self::List(role) => role.debug_name(),
            Self::FormControl(kind) => kind.debug_name(),
        }
    }
}

/// HTML table-tree role, independent from the eventual computed `display`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutTableRole {
    Table,
    Caption,
    ColumnGroup,
    Column,
    HeaderGroup,
    BodyGroup,
    FooterGroup,
    Row,
    Cell,
}

impl LayoutTableRole {
    const fn debug_name(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Caption => "table-caption",
            Self::ColumnGroup => "table-column-group",
            Self::Column => "table-column",
            Self::HeaderGroup => "table-header-group",
            Self::BodyGroup => "table-row-group",
            Self::FooterGroup => "table-footer-group",
            Self::Row => "table-row",
            Self::Cell => "table-cell",
        }
    }
}

/// HTML list-tree role used by marker construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutListRole {
    Container,
    Item,
}

impl LayoutListRole {
    const fn debug_name(self) -> &'static str {
        match self {
            Self::Container => "list-container",
            Self::Item => "list-item",
        }
    }
}

/// Form-control role needed before control-specific box construction lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutFormControlKind {
    Button,
    Input(LayoutInputControlKind),
    TextArea,
    Select,
    Option,
    OptionGroup,
    FieldSet,
    Legend,
    Output,
    Progress,
    Meter,
}

impl LayoutFormControlKind {
    const fn debug_name(self) -> &'static str {
        match self {
            Self::Button => "form-button",
            Self::Input(kind) => kind.debug_name(),
            Self::TextArea => "form-textarea",
            Self::Select => "form-select",
            Self::Option => "form-option",
            Self::OptionGroup => "form-optgroup",
            Self::FieldSet => "form-fieldset",
            Self::Legend => "form-legend",
            Self::Output => "form-output",
            Self::Progress => "form-progress",
            Self::Meter => "form-meter",
        }
    }
}

/// Normalized HTML input state needed by control-specific construction and sizing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutInputControlKind {
    Button,
    Checkbox,
    Color,
    Date,
    DateTimeLocal,
    Email,
    File,
    Hidden,
    Image,
    Month,
    Number,
    Password,
    Radio,
    Range,
    Reset,
    Search,
    Submit,
    Telephone,
    Text,
    Time,
    Url,
    Week,
}

impl LayoutInputControlKind {
    pub const fn debug_name(self) -> &'static str {
        match self {
            Self::Button => "form-input-button",
            Self::Checkbox => "form-input-checkbox",
            Self::Color => "form-input-color",
            Self::Date => "form-input-date",
            Self::DateTimeLocal => "form-input-datetime-local",
            Self::Email => "form-input-email",
            Self::File => "form-input-file",
            Self::Hidden => "form-input-hidden",
            Self::Image => "form-input-image",
            Self::Month => "form-input-month",
            Self::Number => "form-input-number",
            Self::Password => "form-input-password",
            Self::Radio => "form-input-radio",
            Self::Range => "form-input-range",
            Self::Reset => "form-input-reset",
            Self::Search => "form-input-search",
            Self::Submit => "form-input-submit",
            Self::Telephone => "form-input-tel",
            Self::Text => "form-input-text",
            Self::Time => "form-input-time",
            Self::Url => "form-input-url",
            Self::Week => "form-input-week",
        }
    }
}

/// Replaced-content family. Pixel resources are deliberately not part of this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutReplacedKind {
    Image,
    Svg,
    Canvas,
    Embedded,
    Frame,
    Media,
    FormControl,
}

/// Typed HTML inputs consumed by table construction and sizing.
///
/// Values are normalized by the renderer adapter so the layout crate never
/// reaches back into a live DOM or accepts arbitrary attribute lookups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayoutTableData {
    pub column_span: u16,
    pub row_span: u16,
    pub span: u16,
}

impl Default for LayoutTableData {
    fn default() -> Self {
        Self {
            column_span: 1,
            row_span: 1,
            span: 1,
        }
    }
}

/// Typed HTML list inputs used to resolve marker counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LayoutListData {
    pub ordered: bool,
    pub start: Option<i32>,
    pub reversed: bool,
    pub value: Option<i32>,
}

/// Typed state needed for deterministic, resource-independent form-control
/// sizing and placeholder rendering.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayoutFormControlData {
    pub value: Arc<str>,
    pub placeholder: Arc<str>,
    pub size: Option<u16>,
    pub columns: u16,
    pub rows: u16,
    pub maximum_option_characters: u16,
    pub checked: bool,
    pub disabled: bool,
    pub multiple: bool,
}

impl Default for LayoutFormControlData {
    fn default() -> Self {
        Self {
            value: Arc::from(""),
            placeholder: Arc::from(""),
            size: None,
            columns: 20,
            rows: 2,
            maximum_option_characters: 0,
            checked: false,
            disabled: false,
            multiple: false,
        }
    }
}

/// Optional typed HTML metadata retained beside element classification.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LayoutElementMetadata {
    pub table: Option<LayoutTableData>,
    pub list: Option<LayoutListData>,
    pub form_control: Option<LayoutFormControlData>,
}

impl LayoutReplacedKind {
    pub const fn debug_name(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Svg => "svg",
            Self::Canvas => "canvas",
            Self::Embedded => "embedded",
            Self::Frame => "frame",
            Self::Media => "media",
            Self::FormControl => "form-control",
        }
    }
}

/// Owned, pass-local element identity and semantic classification.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayoutElementSemantics {
    pub namespace: LayoutNamespace,
    pub local_name: Arc<str>,
    pub category: LayoutElementCategory,
    pub replaced: Option<LayoutReplacedKind>,
    pub metadata: LayoutElementMetadata,
}

impl LayoutElementSemantics {
    pub fn new(
        namespace: LayoutNamespace,
        local_name: impl Into<Arc<str>>,
        category: LayoutElementCategory,
        replaced: Option<LayoutReplacedKind>,
    ) -> Self {
        Self {
            namespace,
            local_name: local_name.into(),
            category,
            replaced,
            metadata: LayoutElementMetadata {
                table: matches!(category, LayoutElementCategory::Table(_))
                    .then(LayoutTableData::default),
                list: matches!(category, LayoutElementCategory::List(_))
                    .then(LayoutListData::default),
                form_control: matches!(category, LayoutElementCategory::FormControl(_))
                    .then(LayoutFormControlData::default),
            },
        }
    }

    pub fn with_metadata(mut self, metadata: LayoutElementMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub const fn is_replaced(&self) -> bool {
        self.replaced.is_some()
    }

    pub(crate) fn is_html_element(&self, local_name: &str) -> bool {
        self.namespace == LayoutNamespace::Html && self.local_name.as_ref() == local_name
    }

    /// Whether HTML rendering suppresses this control independently of CSS display.
    pub(crate) const fn is_hidden_input(&self) -> bool {
        matches!(
            self.category,
            LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
                LayoutInputControlKind::Hidden
            ))
        )
    }

    /// Whether `display: contents` has a used value of `display: none`.
    ///
    /// This is the unusual-elements list from CSS Display's HTML appendix:
    /// the standard elements are covered by WPT
    /// `display-contents-unusual-html-elements-none.html`, with legacy
    /// `frame`/`frameset` retained consistently with Stylo's Gecko adjustment.
    /// That Stylo adjustment is Gecko-only, so the DOM-neutral box builder
    /// must apply it for Moli's Servo backend.
    pub(crate) fn display_contents_is_none(&self) -> bool {
        self.namespace == LayoutNamespace::Html
            && matches!(
                self.local_name.as_ref(),
                "br" | "wbr"
                    | "meter"
                    | "progress"
                    | "canvas"
                    | "embed"
                    | "object"
                    | "audio"
                    | "iframe"
                    | "img"
                    | "video"
                    | "frame"
                    | "frameset"
                    | "input"
                    | "textarea"
                    | "select"
            )
    }
}

/// Pseudo-elements that can participate in box construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutPseudo {
    Marker,
    Before,
    After,
}

impl LayoutPseudo {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Marker => "::marker",
            Self::Before => "::before",
            Self::After => "::after",
        }
    }
}

/// Replaced-element inputs known without decoding or querying a paint backend.
///
/// Attribute dimensions remain distinct from intrinsic dimensions because CSS
/// replaced sizing gives them different precedence. An unavailable HTML image
/// has no intrinsic dimensions and represents no content, while replaced
/// categories with a CSS default object size (for example canvas) keep their
/// category-specific fallback.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReplacedMetrics {
    pub intrinsic_width: Option<f32>,
    pub intrinsic_height: Option<f32>,
    pub attribute_width: Option<f32>,
    pub attribute_height: Option<f32>,
    pub intrinsic_ratio: Option<f32>,
}

/// Pass-local view of one renderer-owned decoded image resource.
///
/// The immutable pixel buffer may be shared with the document resource owner,
/// but this value contains no DOM callback or retained layout state.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutImageResource {
    pub intrinsic_width: f32,
    pub intrinsic_height: f32,
    pub pixels: Option<Arc<moli_image::RgbaImage>>,
    pub svg: Option<Arc<moli_image::SvgImage>>,
}

/// Pass-local resources aligned with the computed CSS image-layer lists.
///
/// The vectors preserve Stylo's layer indices. Missing entries represent a
/// pending, failed, or unsupported resource and never retain a callback into
/// the renderer's live resource owner.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LayoutCssImageResources {
    pub(crate) background: Vec<Option<LayoutImageResource>>,
    pub(crate) mask: Vec<Option<LayoutImageResource>>,
}

/// One document selection projected onto a source text node in UTF-16 units.
///
/// Equal endpoints represent a caret. The range is sampled before the
/// synchronous one-shot pass and copied into pass-local state; paint never
/// reads the live Selection owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayoutTextSelection {
    pub start: usize,
    pub end: usize,
}

impl LayoutTextSelection {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub const fn is_caret(self) -> bool {
        self.start == self.end
    }
}

/// A short-lived, read-only view of the renderer's canonical flat tree.
pub trait LayoutSource {
    type NodeId: Copy + Debug + Eq + Hash;
    type ChildIter<'a>: Iterator<Item = Self::NodeId>
    where
        Self: 'a;

    fn root(&self) -> Self::NodeId;
    /// Returns the parent in the same flattened tree exposed by [`Self::flat_children`].
    /// The view root must return `None`, even when it has a DOM parent outside the view.
    fn flat_parent(&self, node: Self::NodeId) -> Option<Self::NodeId>;
    fn flat_children(&self, node: Self::NodeId) -> Self::ChildIter<'_>;
    fn node_kind(&self, node: Self::NodeId) -> LayoutSourceKind;
    /// Returns element identity for every `Element` source and only for elements.
    fn element_semantics(&self, node: Self::NodeId) -> Option<LayoutElementSemantics>;
    fn text(&self, node: Self::NodeId) -> Option<&str>;
    fn label(&self, node: Self::NodeId) -> String;

    /// Returns the active document selection intersecting one source text node.
    fn text_selection(&self, _node: Self::NodeId) -> Option<LayoutTextSelection> {
        None
    }

    /// Returns the current CSS-pixel scroll offset owned by an element.
    ///
    /// The value is sampled once while building a pass-local world. It is not
    /// a callback into live state after construction and is never retained
    /// across layout demands.
    fn scroll_offset(&self, _node: Self::NodeId) -> crate::LayoutPoint {
        crate::LayoutPoint::ZERO
    }

    fn replaced_metrics(&self, _node: Self::NodeId) -> Option<ReplacedMetrics> {
        None
    }

    /// Samples immutable replaced content into the current pass.
    ///
    /// The resolved root style is supplied because atomic document resources
    /// such as inline SVG inherit paint inputs (`currentColor`, `font-size`)
    /// from their outer CSS box. Implementations must not retain the style or
    /// call back into live DOM/style state after returning.
    fn replaced_image(
        &self,
        _node: Self::NodeId,
        _style: &ResolvedLayoutStyle,
    ) -> Option<LayoutImageResource> {
        None
    }

    /// Samples one renderer-owned CSS `url()` image into the current pass.
    ///
    /// The URL is already absolute according to Stylo's stylesheet base URL.
    /// Implementations must only return immutable ready resources; layout does
    /// not initiate fetches or wait for asynchronous decode.
    fn css_image_resource(&self, _resolved_url: &str) -> Option<LayoutImageResource> {
        None
    }
}

/// One element's primary style and the pseudo styles eagerly cascaded with it.
///
/// The bundle is owned by one box-construction stack frame. It lets layout
/// retain `::after` until after descendant construction without holding a
/// renderer style-store borrow or introducing a side cache.
pub struct ResolvedLayoutElementStyles {
    primary: ResolvedLayoutStyle,
    before: Option<ResolvedLayoutPseudoStyle>,
    after: Option<ResolvedLayoutPseudoStyle>,
}

impl ResolvedLayoutElementStyles {
    pub fn new(
        primary: ResolvedLayoutStyle,
        before: Option<ResolvedLayoutStyle>,
        after: Option<ResolvedLayoutStyle>,
    ) -> Self {
        Self {
            primary,
            before: before.map(ResolvedLayoutPseudoStyle::new),
            after: after.map(ResolvedLayoutPseudoStyle::new),
        }
    }

    pub fn from_primary(primary: ResolvedLayoutStyle) -> Self {
        Self::new(primary, None, None)
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ResolvedLayoutStyle,
        Option<ResolvedLayoutPseudoStyle>,
        Option<ResolvedLayoutPseudoStyle>,
    ) {
        (self.primary, self.before, self.after)
    }
}

/// Heap-owned pseudo style carried through recursive box construction.
///
/// `ResolvedLayoutStyle` is intentionally rich and therefore large. Keeping
/// absent `::before` and `::after` values inline would reserve room for two
/// complete styles in every recursive builder frame. This wrapper keeps the
/// frame representation pointer-sized without introducing shared state.
pub struct ResolvedLayoutPseudoStyle(Box<ResolvedLayoutStyle>);

impl ResolvedLayoutPseudoStyle {
    pub fn new(style: ResolvedLayoutStyle) -> Self {
        Self(Box::new(style))
    }

    pub(crate) fn into_style(self) -> ResolvedLayoutStyle {
        *self.0
    }
}

/// Renderer-owned access to element, lazy marker, and anonymous styles.
pub trait LayoutStyleResolver<N>
where
    N: Copy + Debug + Eq + Hash,
{
    fn element_styles(
        &mut self,
        node: N,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError>;

    fn marker_style(&mut self, _node: N) -> Result<Option<ResolvedLayoutPseudoStyle>, LayoutError> {
        Ok(None)
    }

    fn anonymous_style(
        &mut self,
        _owner: N,
        parent: &ResolvedLayoutStyle,
        display: crate::LayoutDisplay,
    ) -> Result<ResolvedLayoutStyle, LayoutError> {
        Ok(ResolvedLayoutStyle::anonymous_from(parent, display))
    }
}

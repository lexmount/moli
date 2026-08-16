use std::{fmt::Debug, hash::Hash, sync::Arc};

use crate::{LayoutError, LayoutPoint, ResolvedLayoutStyle};

/// Source-node category needed by CSS box construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutSourceKind {
    Element,
    Text,
    Comment,
    Other,
}

/// Document mode that changes browser layout and CSSOM View semantics.
///
/// This is sampled once into the pass-local layout world. Layout never retains
/// or calls back into the live document after construction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayoutDocumentMode {
    #[default]
    NoQuirks,
    LimitedQuirks,
    Quirks,
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

/// Renderer-resolved fallback content for an unavailable HTML image.
///
/// Resource lifecycle is deliberately absent from the layout crate. The
/// renderer selects this disposition only after the image request can no
/// longer produce primary replaced content; layout then owns the resulting
/// box construction and used-style adjustment.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayoutImageFallbackContent {
    alternative_text: Arc<str>,
    has_nonempty_alt_attribute: bool,
    quirks_mode: bool,
}

impl LayoutImageFallbackContent {
    pub fn new(
        alternative_text: impl Into<Arc<str>>,
        has_nonempty_alt_attribute: bool,
        quirks_mode: bool,
    ) -> Self {
        Self {
            alternative_text: alternative_text.into(),
            has_nonempty_alt_attribute,
            quirks_mode,
        }
    }

    pub fn alternative_text(&self) -> &str {
        &self.alternative_text
    }

    pub const fn has_nonempty_alt_attribute(&self) -> bool {
        self.has_nonempty_alt_attribute
    }

    pub const fn is_quirks_mode(&self) -> bool {
        self.quirks_mode
    }
}

/// Mutually exclusive content selected for an element's principal layout box.
///
/// Keeping fallback content in this sum type prevents an element from being
/// classified as both a replaced leaf and a fallback container. That mirrors
/// the layout-object reattachment boundary used by browser engines when an
/// image request becomes terminally unavailable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayoutElementContent {
    #[default]
    Normal,
    Replaced(LayoutReplacedKind),
    ImageFallback(LayoutImageFallbackContent),
}

impl LayoutElementContent {
    pub const fn replaced_kind(&self) -> Option<LayoutReplacedKind> {
        match self {
            Self::Replaced(kind) => Some(*kind),
            Self::Normal | Self::ImageFallback(_) => None,
        }
    }

    pub const fn image_fallback(&self) -> Option<&LayoutImageFallbackContent> {
        match self {
            Self::ImageFallback(content) => Some(content),
            Self::Normal | Self::Replaced(_) => None,
        }
    }
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
    pub content: LayoutElementContent,
    pub metadata: LayoutElementMetadata,
}

impl LayoutElementSemantics {
    pub fn new(
        namespace: LayoutNamespace,
        local_name: impl Into<Arc<str>>,
        category: LayoutElementCategory,
        content: LayoutElementContent,
    ) -> Self {
        Self {
            namespace,
            local_name: local_name.into(),
            category,
            content,
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
        matches!(&self.content, LayoutElementContent::Replaced(_))
    }

    pub const fn replaced_kind(&self) -> Option<LayoutReplacedKind> {
        self.content.replaced_kind()
    }

    pub const fn image_fallback(&self) -> Option<&LayoutImageFallbackContent> {
        self.content.image_fallback()
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

/// Natural replaced-element inputs owned by the DOM/resource layer.
///
/// HTML width/height presentation attributes are deliberately absent here:
/// they enter layout through the normal computed-style cascade. Canvas
/// dimensions are different because they define the intrinsic bitmap and are
/// converted to natural metrics before crossing this boundary. An unavailable
/// HTML image therefore has no natural dimensions, while categories with a
/// concrete default object size retain their category-specific fallback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReplacedObjectSize {
    pub width: f32,
    pub height: f32,
}

impl ReplacedObjectSize {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReplacedMetrics {
    /// Actual natural content-box dimensions. Missing axes stay absent even
    /// when the decoded resource has a concrete default object size.
    pub intrinsic_width: Option<f32>,
    pub intrinsic_height: Option<f32>,
    /// Resource-specific concrete fallback for missing natural axes. When
    /// absent, the element category supplies its HTML/CSS default.
    pub default_object_size: Option<ReplacedObjectSize>,
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
    fn document_mode(&self) -> LayoutDocumentMode {
        LayoutDocumentMode::NoQuirks
    }
    /// The first HTML body belonging to this document view, whether or not it
    /// currently generates a CSS box.
    fn document_body(&self) -> Option<Self::NodeId> {
        None
    }
    /// Scroll offset owned by the layout viewport rather than by a CSS box.
    fn viewport_scroll_offset(&self) -> LayoutPoint {
        self.scroll_offset(self.root())
    }
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

/// Renderer-owned access to primary, pseudo, and anonymous computed styles.
pub trait LayoutStyleResolver<N>
where
    N: Copy + Debug + Eq + Hash,
{
    fn primary_style(&mut self, node: N) -> Result<Option<ResolvedLayoutStyle>, LayoutError>;

    fn pseudo_style(
        &mut self,
        node: N,
        pseudo: LayoutPseudo,
    ) -> Result<Option<ResolvedLayoutStyle>, LayoutError>;

    fn anonymous_style(
        &mut self,
        _owner: N,
        parent: &ResolvedLayoutStyle,
        display: crate::LayoutDisplay,
    ) -> Result<ResolvedLayoutStyle, LayoutError> {
        Ok(ResolvedLayoutStyle::anonymous_from(parent, display))
    }
}

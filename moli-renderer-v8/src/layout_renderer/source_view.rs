use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use moli_layout::{
    LayoutCssImageState, LayoutElementCategory, LayoutElementMetadata, LayoutElementSemantics,
    LayoutImageResource, LayoutNamespace, LayoutReplacedKind, LayoutSource, LayoutSourceKind,
    LayoutTableData, LayoutTableRole, LayoutTextSelection, ReplacedMetrics,
};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
    dom_presentation::{
        flat_children, flat_parent, form_control_data, layout_element_classification, list_data,
        optional_positive_u16_attribute, select_control_data,
    },
    native_bridge::{CssImagePaintSnapshot, PageScriptHost, element::StyleObservation},
};

pub(super) struct NativeLayoutSourceView<'a> {
    runtime: &'a PageScriptHost,
    root: DomHandle,
    document: Option<DomHandle>,
    styles: RefCell<StyleObservation<'a>>,
    paint_resources: bool,
    css_images: Option<&'a CssImagePaintSnapshot>,
    text_selections: HashMap<DomHandle, LayoutTextSelection>,
}

impl<'a> NativeLayoutSourceView<'a> {
    #[cfg(test)]
    pub(super) fn new(runtime: &'a PageScriptHost, root: DomHandle) -> Self {
        Self::with_paint_resources(runtime, root, StyleObservation::new(runtime), false, None)
    }

    pub(super) fn with_paint_resources(
        runtime: &'a PageScriptHost,
        root: DomHandle,
        styles: StyleObservation<'a>,
        paint_resources: bool,
        css_images: Option<&'a CssImagePaintSnapshot>,
    ) -> Self {
        Self {
            runtime,
            root,
            document: runtime.dom_host().owner_document_handle(root),
            styles: RefCell::new(styles),
            paint_resources,
            css_images,
            text_selections: document_text_selections(runtime, root),
        }
    }

    fn host(&self) -> &DomHost {
        self.runtime.dom_host()
    }

    pub(super) fn has_text_selection(&self) -> bool {
        !self.text_selections.is_empty()
    }

    pub(super) fn svg_input(
        &self,
        node: DomHandle,
        style: &moli_layout::ResolvedLayoutStyle,
        viewport: moli_layout::LayoutSize,
    ) -> Option<moli_layout::InlineSvgInput<DomHandle>> {
        super::inline_svg::collect_input(self.runtime, node, style, viewport, &self.styles)
    }
}

pub(crate) fn native_replaced_image(
    runtime: &PageScriptHost,
    node: DomHandle,
    kind: LayoutReplacedKind,
) -> Option<LayoutImageResource> {
    match kind {
        LayoutReplacedKind::Image => {
            let ready = runtime.ready_image_for_layout(node)?;
            Some(LayoutImageResource {
                intrinsic_width: ready.intrinsic_width,
                intrinsic_height: ready.intrinsic_height,
                pixels: ready.pixels,
                svg: ready.svg,
            })
        }
        LayoutReplacedKind::Canvas => {
            let pixels = runtime.canvas_pixels_for_layout(node)?;
            Some(LayoutImageResource {
                intrinsic_width: pixels.width as f32,
                intrinsic_height: pixels.height as f32,
                pixels: Some(pixels),
                svg: None,
            })
        }
        #[cfg(all(
            feature = "media-ext",
            any(target_os = "linux", target_os = "macos"),
            target_pointer_width = "64"
        ))]
        LayoutReplacedKind::Media => {
            let pixels = runtime.video_image(node)?;
            Some(LayoutImageResource {
                intrinsic_width: pixels.width as f32,
                intrinsic_height: pixels.height as f32,
                pixels: Some(pixels),
                svg: None,
            })
        }
        _ => None,
    }
}

pub(crate) fn native_scroll_offset(host: &DomHost, node: DomHandle) -> moli_layout::LayoutPoint {
    host.node(node)
        .and_then(Node::as_element)
        .map_or(moli_layout::LayoutPoint::ZERO, |element| {
            moli_layout::LayoutPoint::new(element.scroll_left() as f32, element.scroll_top() as f32)
        })
}

impl LayoutSource for NativeLayoutSourceView<'_> {
    type NodeId = DomHandle;
    type ChildIter<'a>
        = std::vec::IntoIter<DomHandle>
    where
        Self: 'a;

    fn root(&self) -> Self::NodeId {
        self.root
    }

    fn root_is_document_element(&self) -> bool {
        self.document.and_then(|document| {
            self.host()
                .dom()
                .document_element_handle_for_document(document)
        }) == Some(self.root)
    }

    fn quirks_mode(&self) -> style::context::QuirksMode {
        self.document
            .and_then(|document| self.host().node(document))
            .and_then(Node::as_document)
            .map_or(style::context::QuirksMode::NoQuirks, |document| {
                document.quirks_mode()
            })
    }

    fn disables_viewport_user_scrolling(&self) -> bool {
        if !self.root_is_document_element() {
            return false;
        }
        self.document
            .and_then(|document| {
                self.runtime
                    .child_browsing_context_host_for_document_handle(document)
            })
            .filter(|frame| {
                self.host().is_html_element_named(*frame, "iframe")
                    || self.host().is_html_element_named(*frame, "frame")
            })
            .and_then(|frame| self.host().get_attribute(frame, "scrolling"))
            .is_some_and(|value| {
                ["no", "off", "noscroll"]
                    .iter()
                    .any(|keyword| value.eq_ignore_ascii_case(keyword))
            })
    }

    fn flat_parent(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        flat_parent(self.host(), self.root, node)
    }

    fn hyperlink(&self, node: DomHandle) -> Option<moli_layout::LayoutHyperlink<DomHandle>> {
        let host = self.host();
        if !host.is_html_element_named(node, "a") {
            return None;
        }
        let document = self.document?;
        let href = host.get_attribute(node, "href")?;
        let url = host
            .document_base_url_for_handle(document)?
            .join(&href)
            .ok()?;
        // JavaScript handlers have no equivalent in a static PDF.
        if url.scheme() == "javascript" {
            return None;
        }
        let mut without_fragment = url.clone();
        without_fragment.set_fragment(None);
        let mut document_url = host.document_url_for_handle(document)?.clone();
        document_url.set_fragment(None);
        let destination = if without_fragment == document_url {
            let fragment = url.fragment().unwrap_or_default();
            let decoded = percent_encoding::percent_decode_str(fragment).decode_utf8_lossy();
            host.find_document_anchor(document, fragment)
                .or_else(|| host.find_document_anchor(document, &decoded))
                .or_else(|| {
                    (decoded.is_empty() || decoded.eq_ignore_ascii_case("top")).then_some(self.root)
                })
        } else {
            None
        };
        Some(moli_layout::LayoutHyperlink {
            url: url.as_str().into(),
            destination,
        })
    }

    fn flat_children(&self, node: Self::NodeId) -> Self::ChildIter<'_> {
        flat_children(self.host(), self.root, node).into_iter()
    }

    fn node_kind(&self, node: Self::NodeId) -> LayoutSourceKind {
        let Some(node) = self.host().node(node) else {
            return LayoutSourceKind::Other;
        };
        if node.is_text() || node.is_cdata_section() {
            return LayoutSourceKind::Text;
        }
        if node.as_comment().is_some() {
            return LayoutSourceKind::Comment;
        }
        if node.is_element() {
            return LayoutSourceKind::Element;
        }
        LayoutSourceKind::Other
    }

    fn element_semantics(&self, node: Self::NodeId) -> Option<LayoutElementSemantics> {
        self.host()
            .node(node)
            .and_then(Node::as_element)
            .map(|element| layout_element_semantics_for_source(self.host(), node, element))
    }

    fn text(&self, node: Self::NodeId) -> Option<&str> {
        self.host().node(node).and_then(Node::data_value)
    }

    fn label(&self, node: Self::NodeId) -> String {
        let Some(native_node) = self.host().node(node) else {
            return format!("detached({})", node.index());
        };
        let Some(element) = native_node.as_element() else {
            return format!("{}({})", native_node.kind_name(), node.index());
        };
        let id = element.attribute("id").filter(|id| !id.is_empty());
        match id {
            Some(id) => format!("{}#{id}", element.local_name()),
            None => format!("{}({})", element.local_name(), node.index()),
        }
    }

    fn text_selection(&self, node: Self::NodeId) -> Option<LayoutTextSelection> {
        if self.runtime.active_element_handle() == Some(node)
            && crate::native_bridge::element::is_text_control(self.runtime, node)
        {
            let element = self.host().node(node)?.as_element()?;
            if element.attribute("disabled").is_none() {
                return Some(LayoutTextSelection::new(
                    element.selection_start() as usize,
                    element.selection_end() as usize,
                ));
            }
        }
        self.text_selections.get(&node).copied()
    }

    fn scroll_offset(&self, node: Self::NodeId) -> moli_layout::LayoutPoint {
        native_scroll_offset(self.host(), node)
    }

    fn replaced_metrics(&self, node: Self::NodeId) -> Option<ReplacedMetrics> {
        let native_node = self.host().node(node)?;
        let element = native_node.as_element()?;
        let semantics = layout_element_semantics_for_source(self.host(), node, element);
        if !semantics.is_replaced() {
            return None;
        }
        if semantics.replaced == Some(LayoutReplacedKind::Svg) {
            return Some(super::inline_svg::replaced_metrics(element));
        }
        let attribute_width = numeric_dimension_attribute(self.host(), node, "width");
        let attribute_height = numeric_dimension_attribute(self.host(), node, "height");
        let intrinsic = self.runtime.image_resource_intrinsic_size(node);
        #[cfg(all(
            feature = "media-ext",
            any(target_os = "linux", target_os = "macos"),
            target_pointer_width = "64"
        ))]
        let intrinsic = self
            .runtime
            .video_dimensions(node)
            .map(|(width, height)| (width as f32, height as f32))
            .or(intrinsic);
        Some(ReplacedMetrics {
            intrinsic_width: intrinsic.map(|(width, _)| width),
            intrinsic_height: intrinsic.map(|(_, height)| height),
            attribute_width,
            attribute_height,
            intrinsic_ratio: intrinsic
                .and_then(|(width, height)| (height > 0.0).then_some(width / height)),
        })
    }

    fn replaced_image(
        &self,
        node: Self::NodeId,
        _style: &moli_layout::ResolvedLayoutStyle,
    ) -> Option<LayoutImageResource> {
        if !self.paint_resources {
            return None;
        }
        let semantics = self
            .host()
            .node(node)
            .and_then(Node::as_element)
            .map(|element| layout_element_semantics_for_source(self.host(), node, element))?;
        native_replaced_image(self.runtime, node, semantics.replaced?)
    }

    fn css_image_state(&self, resolved_url: &str) -> LayoutCssImageState {
        self.css_images
            .zip(self.document)
            .map_or_else(LayoutCssImageState::default, |(images, document)| {
                images.image(document, resolved_url)
            })
    }
}

fn document_text_selections(
    runtime: &PageScriptHost,
    root: DomHandle,
) -> HashMap<DomHandle, LayoutTextSelection> {
    let host = runtime.dom_host();
    let document = host.owner_document_handle(root).or_else(|| {
        host.node(root)
            .is_some_and(|node| node.is_document())
            .then_some(root)
    });
    let Some(selection) =
        document.and_then(|document| runtime.document_selection_snapshot(document))
    else {
        return HashMap::new();
    };

    let (start, end) = selection
        .ordered_in_dom(host)
        .unwrap_or((selection.start, selection.end));

    let mut text_nodes = Vec::new();
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            continue;
        }
        if host
            .node(node)
            .is_some_and(|node| node.is_text() || node.is_cdata_section())
        {
            text_nodes.push(node);
            continue;
        }
        let children = flat_children(host, root, node);
        stack.extend(children.into_iter().rev());
    }

    if start == end {
        if let Some(selection) = collapsed_text_selection(host, &text_nodes, start) {
            return [selection].into_iter().collect();
        }
        return HashMap::new();
    }

    text_nodes
        .into_iter()
        .filter_map(|node| {
            let length = host
                .node(node)
                .and_then(Node::data_value)
                .map(str::encode_utf16)
                .map(Iterator::count)
                .unwrap_or(0);
            let length_u32 = u32::try_from(length).unwrap_or(u32::MAX);
            let starts_before_end =
                crate::range_boundary::point_order_in_dom(host, node, 0, end.container, end.offset)
                    == Some(std::cmp::Ordering::Less);
            let ends_after_start = crate::range_boundary::point_order_in_dom(
                host,
                node,
                length_u32,
                start.container,
                start.offset,
            ) == Some(std::cmp::Ordering::Greater);
            if !starts_before_end || !ends_after_start {
                return None;
            }
            let selected_start = if node == start.container {
                start.offset as usize
            } else {
                0
            }
            .min(length);
            let selected_end = if node == end.container {
                end.offset as usize
            } else {
                length
            }
            .min(length);
            (selected_start < selected_end)
                .then_some((node, LayoutTextSelection::new(selected_start, selected_end)))
        })
        .collect()
}

fn collapsed_text_selection(
    host: &DomHost,
    text_nodes: &[DomHandle],
    point: crate::native_bridge::SelectionBoundarySnapshot,
) -> Option<(DomHandle, LayoutTextSelection)> {
    if let Some(node) = text_nodes
        .iter()
        .copied()
        .find(|node| *node == point.container)
    {
        let length = host
            .node(node)
            .and_then(Node::data_value)
            .map(str::encode_utf16)
            .map(Iterator::count)
            .unwrap_or(0);
        let offset = (point.offset as usize).min(length);
        return Some((node, LayoutTextSelection::new(offset, offset)));
    }

    for node in text_nodes {
        if !matches!(
            crate::range_boundary::point_order_in_dom(
                host,
                point.container,
                point.offset,
                *node,
                0,
            ),
            Some(std::cmp::Ordering::Greater)
        ) {
            return Some((*node, LayoutTextSelection::new(0, 0)));
        }
    }
    let node = *text_nodes.last()?;
    let length = host
        .node(node)
        .and_then(Node::data_value)
        .map(str::encode_utf16)
        .map(Iterator::count)
        .unwrap_or(0);
    Some((node, LayoutTextSelection::new(length, length)))
}

fn layout_element_semantics_for_source(
    host: &DomHost,
    node: DomHandle,
    element: &crate::dom::native::Element,
) -> LayoutElementSemantics {
    let namespace = LayoutNamespace::from_uri(element.namespace());
    let local_name = element.local_name();
    let (category, replaced) = layout_element_classification(element);
    let metadata = layout_element_metadata(host, node, element, category);
    LayoutElementSemantics::new(namespace, local_name, category, replaced).with_metadata(metadata)
}

fn layout_element_metadata(
    host: &DomHost,
    node: DomHandle,
    element: &crate::dom::native::Element,
    category: LayoutElementCategory,
) -> LayoutElementMetadata {
    let mut metadata = LayoutElementMetadata::default();
    match category {
        LayoutElementCategory::Table(role) => {
            let mut table = LayoutTableData::default();
            match role {
                LayoutTableRole::Cell => {
                    table.column_span = positive_u16_attribute(element, "colspan", 1, 1000);
                    table.row_span = non_negative_u16_attribute(element, "rowspan", 1, 65_534);
                }
                LayoutTableRole::Column | LayoutTableRole::ColumnGroup => {
                    table.span = positive_u16_attribute(element, "span", 1, 1000);
                }
                LayoutTableRole::Table
                | LayoutTableRole::Caption
                | LayoutTableRole::HeaderGroup
                | LayoutTableRole::BodyGroup
                | LayoutTableRole::FooterGroup
                | LayoutTableRole::Row => {}
            }
            metadata.table = Some(table);
        }
        LayoutElementCategory::List(_) => {
            metadata.list = list_data(element);
        }
        LayoutElementCategory::FormControl(_) => {
            metadata.form_control = Some(if element.is_html_select() {
                select_control_data(host, node, element)
            } else {
                form_control_data(host, node, element)
            });
        }
        LayoutElementCategory::Generic | LayoutElementCategory::LineBreak => {}
    }
    metadata
}

fn positive_u16_attribute(
    element: &crate::dom::native::Element,
    name: &str,
    default: u16,
    maximum: u16,
) -> u16 {
    optional_positive_u16_attribute(element, name, 1, maximum).unwrap_or(default)
}

fn non_negative_u16_attribute(
    element: &crate::dom::native::Element,
    name: &str,
    default: u16,
    maximum: u16,
) -> u16 {
    element
        .attribute(name)
        .and_then(|value| value.trim().parse::<u32>().ok())
        .map(|value| value.min(u32::from(maximum)) as u16)
        .unwrap_or(default)
}

fn numeric_dimension_attribute(host: &DomHost, node: DomHandle, name: &str) -> Option<f32> {
    let value = host.get_attribute(node, name)?;
    let value = value.trim().parse::<f32>().ok()?;
    value.is_finite().then_some(value.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::{flat_children, flat_parent, layout_element_semantics_for_source};
    use crate::{
        dom::native::{DomHost, NativeDom},
        dom_presentation::{html_element_semantics, html_input_control_kind},
    };
    use moli_layout::{
        LayoutElementCategory, LayoutFormControlKind, LayoutInputControlKind, LayoutListRole,
        LayoutNamespace, LayoutReplacedKind, LayoutTableRole,
    };

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/layout-source").unwrap(),
        ))
    }

    #[test]
    fn html_semantic_matrix_covers_future_box_construction_inputs() {
        use LayoutElementCategory::{FormControl, Generic, LineBreak, List, Table};
        use LayoutFormControlKind::{
            Button, FieldSet, Input, Legend, Meter, Option as FormOption, OptionGroup, Output,
            Progress, Select, TextArea,
        };
        use LayoutListRole::{Container, Item};
        use LayoutTableRole::{
            BodyGroup, Caption, Cell, Column, ColumnGroup, FooterGroup, HeaderGroup, Row,
            Table as TableRoot,
        };

        let cases = [
            ("br", (LineBreak, None)),
            ("table", (Table(TableRoot), None)),
            ("caption", (Table(Caption), None)),
            ("colgroup", (Table(ColumnGroup), None)),
            ("col", (Table(Column), None)),
            ("thead", (Table(HeaderGroup), None)),
            ("tbody", (Table(BodyGroup), None)),
            ("tfoot", (Table(FooterGroup), None)),
            ("tr", (Table(Row), None)),
            ("td", (Table(Cell), None)),
            ("th", (Table(Cell), None)),
            ("ol", (List(Container), None)),
            ("ul", (List(Container), None)),
            ("menu", (List(Container), None)),
            ("li", (List(Item), None)),
            ("button", (FormControl(Button), None)),
            (
                "input",
                (
                    FormControl(Input(LayoutInputControlKind::Text)),
                    Some(LayoutReplacedKind::FormControl),
                ),
            ),
            (
                "textarea",
                (FormControl(TextArea), Some(LayoutReplacedKind::FormControl)),
            ),
            (
                "select",
                (FormControl(Select), Some(LayoutReplacedKind::FormControl)),
            ),
            ("option", (FormControl(FormOption), None)),
            ("optgroup", (FormControl(OptionGroup), None)),
            ("fieldset", (FormControl(FieldSet), None)),
            ("legend", (FormControl(Legend), None)),
            ("output", (FormControl(Output), None)),
            (
                "progress",
                (FormControl(Progress), Some(LayoutReplacedKind::FormControl)),
            ),
            (
                "meter",
                (FormControl(Meter), Some(LayoutReplacedKind::FormControl)),
            ),
            ("img", (Generic, Some(LayoutReplacedKind::Image))),
            ("canvas", (Generic, Some(LayoutReplacedKind::Canvas))),
            ("object", (Generic, None)),
            ("embed", (Generic, Some(LayoutReplacedKind::Embedded))),
            ("frame", (Generic, Some(LayoutReplacedKind::Frame))),
            ("iframe", (Generic, Some(LayoutReplacedKind::Frame))),
            ("audio", (Generic, Some(LayoutReplacedKind::Media))),
            ("video", (Generic, Some(LayoutReplacedKind::Media))),
            ("article", (Generic, None)),
        ];

        for (local_name, expected) in cases {
            assert_eq!(
                html_element_semantics(local_name),
                expected,
                "local name={local_name}"
            );
        }
    }

    #[test]
    fn html_input_type_is_normalized_without_exposing_attributes_to_layout() {
        use LayoutInputControlKind as Input;

        let cases = [
            (Some("button"), Input::Button),
            (Some(" CHECKBOX "), Input::Checkbox),
            (Some("color"), Input::Color),
            (Some("date"), Input::Date),
            (Some("datetime-local"), Input::DateTimeLocal),
            (Some("email"), Input::Email),
            (Some("file"), Input::File),
            (Some("hidden"), Input::Hidden),
            (Some("image"), Input::Image),
            (Some("month"), Input::Month),
            (Some("number"), Input::Number),
            (Some("password"), Input::Password),
            (Some("radio"), Input::Radio),
            (Some("range"), Input::Range),
            (Some("reset"), Input::Reset),
            (Some("search"), Input::Search),
            (Some("submit"), Input::Submit),
            (Some("tel"), Input::Telephone),
            (Some("text"), Input::Text),
            (Some("time"), Input::Time),
            (Some("url"), Input::Url),
            (Some("week"), Input::Week),
            (Some(" SuBmIt "), Input::Submit),
            (Some("not-a-real-type"), Input::Text),
            (None, Input::Text),
        ];

        for (attribute, expected) in cases {
            assert_eq!(
                html_input_control_kind(attribute),
                expected,
                "type={attribute:?}"
            );
        }
    }

    #[test]
    fn qualified_element_identity_preserves_non_html_namespaces() {
        let mut host = test_host();
        let svg = host
            .create_element_ns(Some(LayoutNamespace::SVG_URI), "svg")
            .unwrap();
        let math = host
            .create_element_ns(Some(LayoutNamespace::MATHML_URI), "math")
            .unwrap();
        let custom = host
            .create_element_ns(Some("urn:moli:layout"), "lm:box")
            .unwrap();

        let svg = layout_element_semantics_for_source(
            &host,
            svg,
            host.node(svg).unwrap().as_element().unwrap(),
        );
        assert_eq!(svg.namespace, LayoutNamespace::Svg);
        assert_eq!(&*svg.local_name, "svg");
        assert_eq!(svg.category, LayoutElementCategory::Generic);
        assert_eq!(svg.replaced, Some(LayoutReplacedKind::Svg));

        let math = layout_element_semantics_for_source(
            &host,
            math,
            host.node(math).unwrap().as_element().unwrap(),
        );
        assert_eq!(math.namespace, LayoutNamespace::MathMl);
        assert_eq!(&*math.local_name, "math");

        let custom = layout_element_semantics_for_source(
            &host,
            custom,
            host.node(custom).unwrap().as_element().unwrap(),
        );
        assert_eq!(
            custom.namespace,
            LayoutNamespace::Other("urn:moli:layout".into())
        );
        assert_eq!(&*custom.local_name, "box");
    }

    #[test]
    fn textarea_layout_uses_default_content_until_value_is_set() {
        let mut host = test_host();
        let textarea = host.create_element("textarea");
        let text = host.create_text_node("first\r\nsecond");
        assert!(host.append_child(textarea, text));
        let layout_value = |host: &DomHost| {
            layout_element_semantics_for_source(
                host,
                textarea,
                host.node(textarea).unwrap().as_element().unwrap(),
            )
            .metadata
            .form_control
            .unwrap()
            .value
        };
        assert_eq!(&*layout_value(&host), "first\nsecond");
        assert!(host.set_input_value(textarea, "edited"));
        assert_eq!(&*layout_value(&host), "edited");
        assert!(host.set_input_value(textarea, ""));
        assert_eq!(&*layout_value(&host), "");
    }

    #[test]
    fn phase_four_metadata_is_normalized_from_live_dom_state() {
        use moli_layout::{
            LayoutElementMetadata, LayoutFormControlData, LayoutListData, LayoutTableData,
        };

        let mut host = test_host();

        let cell = host.create_element("td");
        assert!(host.set_attribute(cell, "colspan", "5000"));
        assert!(host.set_attribute(cell, "rowspan", "17"));
        let cell_semantics = layout_element_semantics_for_source(
            &host,
            cell,
            host.node(cell).unwrap().as_element().unwrap(),
        );
        assert_eq!(
            cell_semantics.metadata,
            LayoutElementMetadata {
                table: Some(LayoutTableData {
                    column_span: 1000,
                    row_span: 17,
                    span: 1,
                }),
                ..LayoutElementMetadata::default()
            }
        );

        assert!(host.set_attribute(cell, "rowspan", "0"));
        let zero_row_span = layout_element_semantics_for_source(
            &host,
            cell,
            host.node(cell).unwrap().as_element().unwrap(),
        );
        assert_eq!(zero_row_span.metadata.table.unwrap().row_span, 0);

        let list = host.create_element("ol");
        assert!(host.set_attribute(list, "start", "-3"));
        assert!(host.set_attribute(list, "reversed", ""));
        let list_semantics = layout_element_semantics_for_source(
            &host,
            list,
            host.node(list).unwrap().as_element().unwrap(),
        );
        assert_eq!(
            list_semantics.metadata.list,
            Some(LayoutListData {
                ordered: true,
                start: Some(-3),
                reversed: true,
                value: None,
            })
        );

        let input = host.create_element("input");
        assert!(host.set_attribute(input, "placeholder", "attribute fallback"));
        assert!(host.set_attribute(input, "size", "7"));
        assert!(host.set_input_value(input, "live value"));
        let input_semantics = layout_element_semantics_for_source(
            &host,
            input,
            host.node(input).unwrap().as_element().unwrap(),
        );
        assert_eq!(
            input_semantics.metadata.form_control,
            Some(LayoutFormControlData {
                value: "live value".into(),
                placeholder: "attribute fallback".into(),
                size: Some(7),
                ..LayoutFormControlData::default()
            })
        );

        let checkbox = host.create_element("input");
        assert!(host.set_attribute(checkbox, "type", "checkbox"));
        assert!(host.set_checked_state(checkbox, true));
        let checkbox_semantics = layout_element_semantics_for_source(
            &host,
            checkbox,
            host.node(checkbox).unwrap().as_element().unwrap(),
        );
        assert!(
            checkbox_semantics
                .metadata
                .form_control
                .expect("checkbox metadata")
                .checked
        );

        let select = host.create_element("select");
        let first = host.create_element("option");
        let second = host.create_element("option");
        let second_text = host.create_text_node("selected text");
        assert!(host.set_attribute(first, "label", "first label"));
        assert!(host.append_child(select, first));
        assert!(host.append_child(select, second));
        assert!(host.append_child(second, second_text));
        assert!(host.set_selected_state(second, true));
        let select_semantics = layout_element_semantics_for_source(
            &host,
            select,
            host.node(select).unwrap().as_element().unwrap(),
        );
        let select_data = select_semantics
            .metadata
            .form_control
            .expect("select metadata");
        assert_eq!(select_data.value.as_ref(), "selected text");
        assert_eq!(select_data.maximum_option_characters, 13);
    }

    #[test]
    fn flat_tree_uses_shadow_children_assignments_and_fallback_without_leaks() {
        let mut host = test_host();
        let document = host.document_node_id();
        let root = host.create_element("main");
        let shadow_root = host.attach_shadow_root(root, "open").unwrap();
        let assigned_slot = host.create_element("slot");
        let fallback_slot = host.create_element("slot");
        let assigned = host.create_element("strong");
        let unassigned = host.create_element("em");
        let fallback = host.create_element("span");

        assert!(host.set_attribute(assigned_slot, "name", "selected"));
        assert!(host.set_attribute(fallback_slot, "name", "fallback"));
        assert!(host.set_attribute(assigned, "slot", "selected"));
        assert!(host.set_attribute(unassigned, "slot", "missing"));
        assert!(host.append_child(document, root));
        assert!(host.append_child(shadow_root, assigned_slot));
        assert!(host.append_child(shadow_root, fallback_slot));
        assert!(host.append_child(fallback_slot, fallback));
        assert!(host.append_child(root, assigned));
        assert!(host.append_child(root, unassigned));

        assert_eq!(
            flat_children(&host, root, root),
            vec![assigned_slot, fallback_slot]
        );
        assert_eq!(flat_children(&host, root, assigned_slot), vec![assigned]);
        assert_eq!(flat_children(&host, root, fallback_slot), vec![fallback]);
        assert_eq!(flat_parent(&host, root, assigned_slot), Some(root));
        assert_eq!(flat_parent(&host, root, assigned), Some(assigned_slot));
        assert_eq!(flat_parent(&host, root, fallback), Some(fallback_slot));
        assert_eq!(flat_parent(&host, root, unassigned), None);
        assert_eq!(flat_parent(&host, root, root), None);
    }
}

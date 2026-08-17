use std::collections::{HashMap, HashSet};

use moli_layout::{
    LayoutElementCategory, LayoutElementMetadata, LayoutElementSemantics, LayoutFormControlData,
    LayoutFormControlKind, LayoutImageResource, LayoutInputControlKind, LayoutListData,
    LayoutListRole, LayoutNamespace, LayoutReplacedKind, LayoutSource, LayoutSourceKind,
    LayoutTableData, LayoutTableRole, LayoutTextSelection, ReplacedMetrics,
};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
    native_bridge::JsContextHost,
};

pub(super) struct NativeLayoutSourceView<'a> {
    runtime: &'a JsContextHost,
    root: DomHandle,
    document: Option<DomHandle>,
    include_paint_resources: bool,
    text_selections: HashMap<DomHandle, LayoutTextSelection>,
}

impl<'a> NativeLayoutSourceView<'a> {
    pub(super) fn new(runtime: &'a JsContextHost, root: DomHandle) -> Self {
        Self::with_paint_resources(runtime, root, false)
    }

    pub(super) fn with_paint_resources(
        runtime: &'a JsContextHost,
        root: DomHandle,
        include_paint_resources: bool,
    ) -> Self {
        Self {
            runtime,
            root,
            document: runtime.dom_host().owner_document_handle(root),
            include_paint_resources,
            text_selections: document_text_selections(runtime, root),
        }
    }

    fn host(&self) -> &DomHost {
        self.runtime.dom_host()
    }
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

    fn flat_parent(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        native_flat_parent(self.host(), self.root, node)
    }

    fn flat_children(&self, node: Self::NodeId) -> Self::ChildIter<'_> {
        native_flat_children(self.host(), self.root, node).into_iter()
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
        self.text_selections.get(&node).copied()
    }

    fn scroll_offset(&self, node: Self::NodeId) -> moli_layout::LayoutPoint {
        self.host().node(node).and_then(Node::as_element).map_or(
            moli_layout::LayoutPoint::ZERO,
            |element| {
                moli_layout::LayoutPoint::new(
                    element.scroll_left() as f32,
                    element.scroll_top() as f32,
                )
            },
        )
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
        style: &moli_layout::ResolvedLayoutStyle,
    ) -> Option<LayoutImageResource> {
        if !self.include_paint_resources {
            return None;
        }
        let semantics = self
            .host()
            .node(node)
            .and_then(Node::as_element)
            .map(|element| layout_element_semantics_for_source(self.host(), node, element))?;
        match semantics.replaced {
            Some(LayoutReplacedKind::Image) => {
                let ready = self.runtime.ready_image_for_layout(node)?;
                Some(LayoutImageResource {
                    intrinsic_width: ready.intrinsic_width,
                    intrinsic_height: ready.intrinsic_height,
                    pixels: ready.pixels,
                    svg: ready.svg,
                })
            }
            Some(LayoutReplacedKind::Svg) => {
                super::inline_svg::replaced_resource(self.host(), node, style)
            }
            Some(LayoutReplacedKind::Canvas) => {
                let pixels = self.runtime.canvas_pixels_for_layout(node)?;
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

    fn css_image_resource(&self, resolved_url: &str) -> Option<LayoutImageResource> {
        if !self.include_paint_resources {
            return None;
        }
        let parsed = url::Url::parse(resolved_url).ok()?;
        // Blink shares the fetched bytes using a fragment-free cache key, but
        // then applies SVG view/element-fragment semantics separately. Until
        // that latter projection exists, painting the whole SVG would be a
        // visibly incorrect fallback.
        if parsed.fragment().is_some() {
            return None;
        }
        let ready = self
            .runtime
            .ready_css_image_for_layout(self.document?, parsed.as_str())?;
        Some(LayoutImageResource {
            intrinsic_width: ready.intrinsic_width,
            intrinsic_height: ready.intrinsic_height,
            pixels: ready.pixels,
            svg: ready.svg,
        })
    }
}

fn document_text_selections(
    runtime: &JsContextHost,
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

    let mut start = selection.start;
    let mut end = selection.end;
    if crate::range_boundary::point_order_in_dom(
        host,
        start.container,
        start.offset,
        end.container,
        end.offset,
    ) == Some(std::cmp::Ordering::Greater)
    {
        std::mem::swap(&mut start, &mut end);
    }

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
        let children = native_flat_children(host, root, node);
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

fn native_flat_children(host: &DomHost, root: DomHandle, node: DomHandle) -> Vec<DomHandle> {
    let candidates = if let Some(shadow_root) = host.shadow_root_handle(node) {
        host.child_handles(shadow_root).collect::<Vec<_>>()
    } else if host.is_html_element_named(node, "slot") {
        let assigned = host.assigned_nodes_for_slot_with_options(node, false);
        if assigned.is_empty() {
            host.child_handles(node).collect::<Vec<_>>()
        } else {
            assigned
        }
    } else {
        host.child_handles(node).collect::<Vec<_>>()
    };
    candidates
        .into_iter()
        .filter(|child| native_flat_parent(host, root, *child) == Some(node))
        .collect()
}

fn native_flat_parent(host: &DomHost, root: DomHandle, node: DomHandle) -> Option<DomHandle> {
    if node == root {
        return None;
    }
    if let Some(slot) = host.assigned_slot_for_node(node) {
        return Some(slot);
    }
    let parent = host.node(node).and_then(Node::parent_node)?;
    if host.is_shadow_root(parent) {
        return host.shadow_root_host(parent);
    }
    if host.is_html_element_named(parent, "slot")
        && !host
            .assigned_nodes_for_slot_with_options(parent, false)
            .is_empty()
    {
        return None;
    }
    if host.shadow_root_handle(parent).is_some() && native_node_is_slotable(host, node) {
        return None;
    }
    Some(parent)
}

fn native_node_is_slotable(host: &DomHost, node: DomHandle) -> bool {
    host.node(node)
        .is_some_and(|node| node.is_element() || node.is_text())
}

fn layout_element_semantics(element: &crate::dom::native::Element) -> LayoutElementSemantics {
    let namespace = LayoutNamespace::from_uri(element.namespace());
    let local_name = element.local_name();
    let (category, replaced) = if namespace == LayoutNamespace::Html {
        if local_name == "input" {
            (
                LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
                    html_input_control_kind(element.attribute("type")),
                )),
                Some(LayoutReplacedKind::FormControl),
            )
        } else {
            html_element_semantics(local_name)
        }
    } else if namespace == LayoutNamespace::Svg && local_name == "svg" {
        (
            LayoutElementCategory::Generic,
            Some(LayoutReplacedKind::Svg),
        )
    } else {
        (LayoutElementCategory::Generic, None)
    };
    let metadata = layout_element_metadata(element, category, None, 0);
    LayoutElementSemantics::new(namespace, local_name, category, replaced).with_metadata(metadata)
}

fn layout_element_semantics_for_source(
    host: &DomHost,
    node: DomHandle,
    element: &crate::dom::native::Element,
) -> LayoutElementSemantics {
    let mut semantics = layout_element_semantics(element);
    let (selected_text, maximum_option_characters) = if element.is_html_select() {
        let options = host.select_option_elements(node);
        let option_text = |option| {
            host.node(option)
                .and_then(Node::as_element)
                .and_then(|option| option.attribute("label"))
                .map(str::to_owned)
                .or_else(|| host.text_content(option))
                .unwrap_or_default()
        };
        let maximum = options
            .iter()
            .map(|option| option_text(*option).chars().count())
            .max()
            .unwrap_or_default();
        let selected = host
            .select_selected_option_elements(node)
            .first()
            .copied()
            .map(option_text)
            .unwrap_or_default();
        (Some(selected), u16::try_from(maximum).unwrap_or(u16::MAX))
    } else {
        (None, 0)
    };
    semantics.metadata = layout_element_metadata(
        element,
        semantics.category,
        selected_text,
        maximum_option_characters,
    );
    semantics
}

fn layout_element_metadata(
    element: &crate::dom::native::Element,
    category: LayoutElementCategory,
    selected_text: Option<String>,
    maximum_option_characters: u16,
) -> LayoutElementMetadata {
    let mut metadata = LayoutElementMetadata::default();
    match category {
        LayoutElementCategory::Table(role) => {
            let mut table = LayoutTableData::default();
            match role {
                LayoutTableRole::Cell => {
                    table.column_span = positive_u16_attribute(element, "colspan", 1, 1000);
                    table.row_span = positive_u16_attribute(element, "rowspan", 1, 65_534);
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
        LayoutElementCategory::List(role) => {
            metadata.list = Some(LayoutListData {
                ordered: role == LayoutListRole::Container && element.local_name() == "ol",
                start: (role == LayoutListRole::Container)
                    .then(|| signed_i32_attribute(element, "start"))
                    .flatten(),
                reversed: role == LayoutListRole::Container
                    && element.attribute("reversed").is_some(),
                value: (role == LayoutListRole::Item)
                    .then(|| signed_i32_attribute(element, "value"))
                    .flatten(),
            });
        }
        LayoutElementCategory::FormControl(_) => {
            let value = if element.is_html_select() {
                selected_text.unwrap_or_default()
            } else if element.is_html_input() || element.is_html_textarea() {
                element.input_value()
            } else {
                element.attribute("value").unwrap_or("").to_owned()
            };
            metadata.form_control = Some(LayoutFormControlData {
                value: value.into(),
                placeholder: element.attribute("placeholder").unwrap_or("").into(),
                size: optional_positive_u16_attribute(element, "size", 1, 65_535),
                columns: positive_u16_attribute(element, "cols", 20, 65_535),
                rows: positive_u16_attribute(element, "rows", 2, 65_535),
                maximum_option_characters,
                checked: element.checked(),
                disabled: element.attribute("disabled").is_some(),
                multiple: element.attribute("multiple").is_some(),
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

fn optional_positive_u16_attribute(
    element: &crate::dom::native::Element,
    name: &str,
    minimum: u16,
    maximum: u16,
) -> Option<u16> {
    element
        .attribute(name)?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value.clamp(u32::from(minimum), u32::from(maximum)) as u16)
}

fn signed_i32_attribute(element: &crate::dom::native::Element, name: &str) -> Option<i32> {
    element.attribute(name)?.trim().parse::<i32>().ok()
}

fn html_element_semantics(local_name: &str) -> (LayoutElementCategory, Option<LayoutReplacedKind>) {
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

    match local_name {
        "br" => (LineBreak, None),
        "table" => (Table(TableRoot), None),
        "caption" => (Table(Caption), None),
        "colgroup" => (Table(ColumnGroup), None),
        "col" => (Table(Column), None),
        "thead" => (Table(HeaderGroup), None),
        "tbody" => (Table(BodyGroup), None),
        "tfoot" => (Table(FooterGroup), None),
        "tr" => (Table(Row), None),
        "td" | "th" => (Table(Cell), None),
        "ol" | "ul" | "menu" => (List(Container), None),
        "li" => (List(Item), None),
        "button" => (FormControl(Button), None),
        "input" => (
            FormControl(Input(LayoutInputControlKind::Text)),
            Some(LayoutReplacedKind::FormControl),
        ),
        "textarea" => (FormControl(TextArea), Some(LayoutReplacedKind::FormControl)),
        "select" => (FormControl(Select), Some(LayoutReplacedKind::FormControl)),
        "option" => (FormControl(FormOption), None),
        "optgroup" => (FormControl(OptionGroup), None),
        "fieldset" => (FormControl(FieldSet), None),
        "legend" => (FormControl(Legend), None),
        "output" => (FormControl(Output), None),
        "progress" => (FormControl(Progress), Some(LayoutReplacedKind::FormControl)),
        "meter" => (FormControl(Meter), Some(LayoutReplacedKind::FormControl)),
        "img" => (Generic, Some(LayoutReplacedKind::Image)),
        "canvas" => (Generic, Some(LayoutReplacedKind::Canvas)),
        // `<object>` is intentionally not unconditional replaced content: when its resource is
        // unavailable the fallback DOM children must still construct boxes.
        "embed" => (Generic, Some(LayoutReplacedKind::Embedded)),
        "frame" | "iframe" => (Generic, Some(LayoutReplacedKind::Frame)),
        "audio" | "video" => (Generic, Some(LayoutReplacedKind::Media)),
        _ => (Generic, None),
    }
}

fn html_input_control_kind(value: Option<&str>) -> LayoutInputControlKind {
    use LayoutInputControlKind as Input;
    match value
        .map(str::trim)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "button" => Input::Button,
        "checkbox" => Input::Checkbox,
        "color" => Input::Color,
        "date" => Input::Date,
        "datetime-local" => Input::DateTimeLocal,
        "email" => Input::Email,
        "file" => Input::File,
        "hidden" => Input::Hidden,
        "image" => Input::Image,
        "month" => Input::Month,
        "number" => Input::Number,
        "password" => Input::Password,
        "radio" => Input::Radio,
        "range" => Input::Range,
        "reset" => Input::Reset,
        "search" => Input::Search,
        "submit" => Input::Submit,
        "tel" => Input::Telephone,
        "time" => Input::Time,
        "url" => Input::Url,
        "week" => Input::Week,
        _ => Input::Text,
    }
}

fn numeric_dimension_attribute(host: &DomHost, node: DomHandle, name: &str) -> Option<f32> {
    let value = host.get_attribute(node, name)?;
    let value = value.trim().parse::<f32>().ok()?;
    value.is_finite().then_some(value.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::{
        html_element_semantics, html_input_control_kind, layout_element_semantics,
        layout_element_semantics_for_source, native_flat_children, native_flat_parent,
    };
    use crate::dom::native::{DomHost, NativeDom};
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

        let svg = layout_element_semantics(host.node(svg).unwrap().as_element().unwrap());
        assert_eq!(svg.namespace, LayoutNamespace::Svg);
        assert_eq!(&*svg.local_name, "svg");
        assert_eq!(svg.category, LayoutElementCategory::Generic);
        assert_eq!(svg.replaced, Some(LayoutReplacedKind::Svg));

        let math = layout_element_semantics(host.node(math).unwrap().as_element().unwrap());
        assert_eq!(math.namespace, LayoutNamespace::MathMl);
        assert_eq!(&*math.local_name, "math");

        let custom = layout_element_semantics(host.node(custom).unwrap().as_element().unwrap());
        assert_eq!(
            custom.namespace,
            LayoutNamespace::Other("urn:moli:layout".into())
        );
        assert_eq!(&*custom.local_name, "box");
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
            native_flat_children(&host, root, root),
            vec![assigned_slot, fallback_slot]
        );
        assert_eq!(
            native_flat_children(&host, root, assigned_slot),
            vec![assigned]
        );
        assert_eq!(
            native_flat_children(&host, root, fallback_slot),
            vec![fallback]
        );
        assert_eq!(native_flat_parent(&host, root, assigned_slot), Some(root));
        assert_eq!(
            native_flat_parent(&host, root, assigned),
            Some(assigned_slot)
        );
        assert_eq!(
            native_flat_parent(&host, root, fallback),
            Some(fallback_slot)
        );
        assert_eq!(native_flat_parent(&host, root, unassigned), None);
        assert_eq!(native_flat_parent(&host, root, root), None);
    }
}

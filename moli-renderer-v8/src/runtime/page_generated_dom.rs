use crate::dom::native::{DomHost, NodeType};
use moli_dom::forms::{InputType, MeterElementValues, MeterGaugeRegion};
use moli_page_types::{
    DocumentNodeAttributeSnapshot, DocumentNodeInspectorIdentity, DocumentNodeSnapshot,
};

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const INPUT_TEXT_CONTROL_TREE_KIND: u16 = 1;
const TEXTAREA_CONTROL_TREE_KIND: u16 = 2;
const SELECT_MENU_TREE_KIND: u16 = 3;
const SELECT_LIST_BOX_TREE_KIND: u16 = 4;
const OPTION_TREE_KIND: u16 = 5;
const NUMBER_INPUT_TREE_KIND: u16 = 6;
const DATE_INPUT_TREE_KIND: u16 = 7;
const DETAILS_TREE_KIND: u16 = 8;
const SEARCH_INPUT_TREE_KIND: u16 = 9;
const PROGRESS_TREE_KIND: u16 = 10;
const OPTGROUP_TREE_KIND: u16 = 11;
const RANGE_INPUT_TREE_KIND: u16 = 12;
const METER_TREE_KIND: u16 = 13;
const DETAILS_UA_STYLE_TEXT: &str = "\
\n:host summary {
  display: list-item;
  counter-increment: list-item 0;
  list-style: disclosure-closed inside;
}
:host([open]) summary {
  list-style-type: disclosure-open;
}
";

pub(super) fn user_agent_shadow_root_snapshot(
    dom_host: &DomHost,
    originating_element: &DocumentNodeSnapshot,
    depth: i32,
    include_whitespace: bool,
) -> Option<DocumentNodeSnapshot> {
    let mut root = full_user_agent_shadow_root_snapshot(dom_host, originating_element)?;
    if !include_whitespace {
        filter_inspector_whitespace_text_nodes(&mut root);
    }
    Some(truncate_snapshot_to_depth(root, depth))
}

pub(super) fn user_agent_shadow_node_snapshot(
    dom_host: &DomHost,
    originating_element: &DocumentNodeSnapshot,
    identity: DocumentNodeInspectorIdentity,
    depth: i32,
    include_whitespace: bool,
) -> Option<DocumentNodeSnapshot> {
    let mut root = full_user_agent_shadow_root_snapshot(dom_host, originating_element)?;
    if !include_whitespace {
        filter_inspector_whitespace_text_nodes(&mut root);
    }
    let mut stack = vec![root];
    while let Some(snapshot) = stack.pop() {
        if snapshot.inspector_identity == Some(identity) {
            return Some(truncate_snapshot_to_depth(snapshot, depth));
        }
        stack.extend(snapshot.pseudo_elements);
        stack.extend(snapshot.shadow_roots);
        stack.extend(snapshot.children);
    }
    None
}

fn filter_inspector_whitespace_text_nodes(snapshot: &mut DocumentNodeSnapshot) {
    snapshot
        .children
        .retain(|child| !super::page_dom::inspector_whitespace_text_snapshot(child));
    snapshot.child_count = snapshot.children.len();
    for child in &mut snapshot.children {
        filter_inspector_whitespace_text_nodes(child);
    }
    for shadow_root in &mut snapshot.shadow_roots {
        filter_inspector_whitespace_text_nodes(shadow_root);
    }
    for pseudo_element in &mut snapshot.pseudo_elements {
        filter_inspector_whitespace_text_nodes(pseudo_element);
    }
    if let Some(associated) = snapshot.associated.as_deref_mut() {
        filter_inspector_whitespace_text_nodes(associated.node_mut());
    }
}

fn full_user_agent_shadow_root_snapshot(
    dom_host: &DomHost,
    originating_element: &DocumentNodeSnapshot,
) -> Option<DocumentNodeSnapshot> {
    let element = dom_host.node(originating_element.node_id)?.as_element()?;
    if element.is_html_select() {
        return Some(select_shadow_root_snapshot(dom_host, originating_element));
    }
    if element.is_html_option() {
        return Some(option_shadow_root_snapshot(dom_host, originating_element));
    }
    if element.is_html_element("details") {
        return Some(details_shadow_root_snapshot(dom_host, originating_element));
    }
    let has_datalist = element.is_html_input()
        && dom_host
            .input_datalist_handle(originating_element.node_id)
            .is_some();
    if element.is_html_input() && element.input_type() == InputType::Number {
        return Some(number_input_shadow_root_snapshot(
            element.input_value(),
            has_datalist,
            originating_element,
        ));
    }
    if element.is_html_input() && element.input_type() == InputType::Range {
        return Some(range_input_shadow_root_snapshot(originating_element));
    }
    if element.is_html_input() && element.input_type() == InputType::Date {
        return Some(date_input_shadow_root_snapshot(
            element.input_value(),
            element.attribute("min"),
            element.attribute("max"),
            originating_element,
        ));
    }
    if element.is_html_input() && element.input_type() == InputType::Search {
        return Some(search_input_shadow_root_snapshot(
            element.input_value(),
            element.input_value_dirty(),
            has_datalist,
            originating_element,
        ));
    }
    if element.is_html_element("progress") {
        let values = moli_dom::forms::progress_element_values(
            element.attribute("value"),
            element.attribute("max"),
        );
        return Some(progress_shadow_root_snapshot(
            values.position,
            originating_element,
        ));
    }
    if element.is_html_element("meter") {
        let values = moli_dom::forms::meter_element_values(
            element.attribute("value"),
            element.attribute("min"),
            element.attribute("max"),
            element.attribute("low"),
            element.attribute("high"),
            element.attribute("optimum"),
        );
        return Some(meter_shadow_root_snapshot(values, originating_element));
    }
    if element.is_html_element("optgroup") {
        return Some(optgroup_shadow_root_snapshot(
            element.attribute("label"),
            originating_element,
        ));
    }
    let (tree_kind, value) = if element.is_html_input()
        && matches!(
            element.input_type(),
            InputType::Text
                | InputType::Tel
                | InputType::Url
                | InputType::Email
                | InputType::Password
        ) {
        (INPUT_TEXT_CONTROL_TREE_KIND, element.input_value())
    } else if element.is_html_textarea() {
        let value = if element.input_value_dirty() {
            element.input_value()
        } else {
            dom_host
                .dom()
                .text_content(originating_element.node_id)
                .unwrap_or_default()
        };
        (
            TEXTAREA_CONTROL_TREE_KIND,
            normalize_textarea_api_value(&value),
        )
    } else {
        return None;
    };

    if tree_kind == INPUT_TEXT_CONTROL_TREE_KIND && has_datalist {
        return Some(datalist_text_input_shadow_root_snapshot(
            value,
            element
                .datalist_text_decoration_initial_value_dirty()
                .unwrap_or_else(|| element.input_value_dirty()),
            originating_element,
        ));
    }

    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let editor_identity = user_agent_shadow_identity(tree_kind, 1);
    let editor_children = if tree_kind == TEXTAREA_CONTROL_TREE_KIND {
        textarea_editor_children(originating_element, editor_identity, &value)
    } else if value.is_empty() {
        Vec::new()
    } else {
        vec![text_snapshot(
            originating_element,
            tree_kind,
            2,
            editor_identity,
            value.clone(),
            inspector_state_fingerprint(&value),
        )]
    };
    let editor = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "div",
        Vec::new(),
        editor_children,
    );
    Some(shadow_root_snapshot(
        originating_element,
        root_identity,
        vec![editor],
    ))
}

fn search_input_shadow_root_snapshot(
    value: String,
    value_dirty: bool,
    has_datalist: bool,
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let tree_kind = SEARCH_INPUT_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let container_identity = user_agent_shadow_identity(tree_kind, 1);
    let viewport_identity = user_agent_shadow_identity(tree_kind, 2);
    let editor_identity = user_agent_shadow_identity(tree_kind, 3);
    let mut editor_children = Vec::new();
    let value_is_empty = value.is_empty();
    if !value_is_empty {
        let state = inspector_state_fingerprint(&value);
        editor_children.push(text_snapshot(
            originating_element,
            tree_kind,
            4,
            editor_identity,
            value,
            state,
        ));
    }
    let editor = element_snapshot(
        originating_element,
        tree_kind,
        3,
        viewport_identity,
        "div",
        Vec::new(),
        editor_children,
    );
    let viewport = element_snapshot(
        originating_element,
        tree_kind,
        2,
        container_identity,
        "div",
        vec![attribute_snapshot("id", "editing-view-port")],
        vec![editor],
    );
    let mut clear_attributes = vec![
        attribute_snapshot("pseudo", "-webkit-search-cancel-button"),
        attribute_snapshot("id", "search-clear"),
    ];
    if value_is_empty {
        clear_attributes.push(attribute_snapshot(
            "style",
            "opacity: 0; pointer-events: none;",
        ));
    } else if value_dirty {
        clear_attributes.push(attribute_snapshot("style", ""));
    }
    let clear = element_snapshot(
        originating_element,
        tree_kind,
        5,
        container_identity,
        "div",
        clear_attributes,
        Vec::new(),
    );
    let mut container_children = vec![viewport, clear];
    if has_datalist {
        container_children.push(datalist_picker_snapshot(
            originating_element,
            tree_kind,
            6,
            7,
            container_identity,
        ));
    }
    let container = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "div",
        vec![
            attribute_snapshot("id", "text-field-container"),
            attribute_snapshot("pseudo", "-webkit-textfield-decoration-container"),
            attribute_snapshot("style", "unicode-bidi: normal;"),
        ],
        container_children,
    );
    shadow_root_snapshot(originating_element, root_identity, vec![container])
}

fn datalist_text_input_shadow_root_snapshot(
    value: String,
    value_dirty: bool,
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let tree_kind = INPUT_TEXT_CONTROL_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let container_identity = user_agent_shadow_identity(tree_kind, 3);
    let viewport_identity = user_agent_shadow_identity(tree_kind, 4);
    let editor_identity = user_agent_shadow_identity(tree_kind, 5);
    let editor_children = if value.is_empty() {
        Vec::new()
    } else {
        vec![text_snapshot(
            originating_element,
            tree_kind,
            6,
            editor_identity,
            value.clone(),
            inspector_state_fingerprint(&value),
        )]
    };
    let editor = element_snapshot(
        originating_element,
        tree_kind,
        5,
        viewport_identity,
        "div",
        Vec::new(),
        editor_children,
    );
    let viewport = element_snapshot(
        originating_element,
        tree_kind,
        4,
        container_identity,
        "div",
        vec![attribute_snapshot("id", "editing-view-port")],
        vec![editor],
    );
    let picker = datalist_picker_snapshot(originating_element, tree_kind, 7, 8, container_identity);
    let mut container_attributes = vec![
        attribute_snapshot("id", "text-field-container"),
        attribute_snapshot("pseudo", "-webkit-textfield-decoration-container"),
    ];
    if !value_dirty {
        container_attributes.push(attribute_snapshot("style", "unicode-bidi: normal;"));
    }
    let container = element_snapshot(
        originating_element,
        tree_kind,
        3,
        root_identity,
        "div",
        container_attributes,
        vec![viewport, picker],
    );
    shadow_root_snapshot(originating_element, root_identity, vec![container])
}

fn range_input_shadow_root_snapshot(
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let tree_kind = RANGE_INPUT_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let container_identity = user_agent_shadow_identity(tree_kind, 1);
    let track_identity = user_agent_shadow_identity(tree_kind, 2);
    let thumb = element_snapshot(
        originating_element,
        tree_kind,
        3,
        track_identity,
        "div",
        vec![attribute_snapshot("id", "thumb")],
        Vec::new(),
    );
    let track = element_snapshot(
        originating_element,
        tree_kind,
        2,
        container_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "-webkit-slider-runnable-track"),
            attribute_snapshot("id", "track"),
        ],
        vec![thumb],
    );
    let container = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "div",
        Vec::new(),
        vec![track],
    );
    shadow_root_snapshot(originating_element, root_identity, vec![container])
}

fn datalist_picker_snapshot(
    originating_element: &DocumentNodeSnapshot,
    tree_kind: u16,
    ordinal: u16,
    marker_ordinal: u16,
    parent_identity: DocumentNodeInspectorIdentity,
) -> DocumentNodeSnapshot {
    let picker_identity = user_agent_shadow_identity(tree_kind, ordinal);
    let mut picker = element_snapshot(
        originating_element,
        tree_kind,
        ordinal,
        parent_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "-webkit-calendar-picker-indicator"),
            attribute_snapshot("id", "picker"),
            attribute_snapshot("aria-hidden", "true"),
            attribute_snapshot(
                "style",
                "display: list-item; list-style: inside disclosure-open; counter-increment: \
                 list-item 0; block-size: 1em;",
            ),
        ],
        Vec::new(),
    );
    picker.pseudo_elements.push(user_agent_marker_snapshot(
        originating_element,
        tree_kind,
        marker_ordinal,
        picker_identity,
    ));
    picker
}

fn user_agent_marker_snapshot(
    originating_element: &DocumentNodeSnapshot,
    tree_kind: u16,
    ordinal: u16,
    parent_identity: DocumentNodeInspectorIdentity,
) -> DocumentNodeSnapshot {
    DocumentNodeSnapshot {
        node_id: originating_element.node_id,
        parent_id: None,
        inspector_identity: Some(user_agent_shadow_identity(tree_kind, ordinal)),
        inspector_parent_identity: Some(parent_identity),
        frontend_node_id: None,
        parent_frontend_node_id: None,
        backend_node_id: None,
        frame_id: None,
        node_type: NodeType::Element as u8,
        node_name: "::marker".to_owned(),
        local_name: "::marker".to_owned(),
        node_value: String::new(),
        child_count: 0,
        document_url: originating_element.document_url.clone(),
        base_url: originating_element.base_url.clone(),
        namespace_uri: None,
        attributes: Vec::new(),
        document_type_name: None,
        public_id: None,
        system_id: None,
        is_element: true,
        has_geometry: false,
        shadow_root_type: None,
        shadow_roots: Vec::new(),
        pseudo_type: Some("marker".to_owned()),
        pseudo_elements: Vec::new(),
        associated: None,
        children: Vec::new(),
    }
}

fn progress_shadow_root_snapshot(
    position: f64,
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let tree_kind = PROGRESS_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let inner_identity = user_agent_shadow_identity(tree_kind, 1);
    let bar_identity = user_agent_shadow_identity(tree_kind, 2);
    let value = element_snapshot(
        originating_element,
        tree_kind,
        3,
        bar_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "-webkit-progress-value"),
            attribute_snapshot(
                "style",
                &format!(
                    "inline-size: {}%; block-size: 100%;",
                    format_percentage(position)
                ),
            ),
        ],
        Vec::new(),
    );
    let bar = element_snapshot(
        originating_element,
        tree_kind,
        2,
        inner_identity,
        "div",
        vec![attribute_snapshot("pseudo", "-webkit-progress-bar")],
        vec![value],
    );
    let inner = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "div",
        vec![attribute_snapshot(
            "pseudo",
            "-webkit-progress-inner-element",
        )],
        vec![bar],
    );
    shadow_root_snapshot(originating_element, root_identity, vec![inner])
}

fn meter_shadow_root_snapshot(
    values: MeterElementValues,
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let tree_kind = METER_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let inner_identity = user_agent_shadow_identity(tree_kind, 1);
    let bar_identity = user_agent_shadow_identity(tree_kind, 2);
    let value_pseudo = match values.gauge_region {
        MeterGaugeRegion::Optimum => "-webkit-meter-optimum-value",
        MeterGaugeRegion::Suboptimum => "-webkit-meter-suboptimum-value",
        MeterGaugeRegion::EvenLessGood => "-webkit-meter-even-less-good-value",
    };
    let value = element_snapshot(
        originating_element,
        tree_kind,
        3,
        bar_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", value_pseudo),
            attribute_snapshot(
                "style",
                &format!(
                    "inline-size: {}%; block-size: 100%;",
                    format_percentage(values.position)
                ),
            ),
        ],
        Vec::new(),
    );
    let bar = element_snapshot(
        originating_element,
        tree_kind,
        2,
        inner_identity,
        "div",
        vec![attribute_snapshot("pseudo", "-webkit-meter-bar")],
        vec![value],
    );
    let inner = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "div",
        vec![attribute_snapshot("pseudo", "-webkit-meter-inner-element")],
        vec![bar],
    );
    shadow_root_snapshot(originating_element, root_identity, vec![inner])
}

fn optgroup_shadow_root_snapshot(
    label: Option<&str>,
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let tree_kind = OPTGROUP_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let label_identity = user_agent_shadow_identity(tree_kind, 1);
    let mut label_attributes = vec![
        attribute_snapshot("aria-hidden", "true"),
        attribute_snapshot("pseudo", "-internal-optgroup-label"),
    ];
    let mut label_children = Vec::new();
    if let Some(label) = label {
        label_attributes.push(attribute_snapshot("aria-label", label));
        if !label.is_empty() {
            label_children.push(text_snapshot(
                originating_element,
                tree_kind,
                2,
                label_identity,
                label.to_owned(),
                inspector_state_fingerprint(label),
            ));
        }
    }
    let label = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "div",
        label_attributes,
        label_children,
    );
    let options_slot = element_snapshot(
        originating_element,
        tree_kind,
        3,
        root_identity,
        "slot",
        Vec::new(),
        Vec::new(),
    );
    shadow_root_snapshot(
        originating_element,
        root_identity,
        vec![label, options_slot],
    )
}

fn format_percentage(position: f64) -> String {
    let percentage = position * 100.0;
    if percentage == 0.0 {
        return "0".to_owned();
    }
    let magnitude = percentage.abs().log10().floor() as i32;
    let decimal_places = (5 - magnitude).clamp(0, 12) as usize;
    let scale = 10_f64.powi(decimal_places as i32);
    let rounded = (percentage * scale).round() / scale;
    let mut serialized = format!("{rounded:.decimal_places$}");
    if serialized.contains('.') {
        while serialized.ends_with('0') {
            serialized.pop();
        }
        if serialized.ends_with('.') {
            serialized.pop();
        }
    }
    serialized
}

fn select_shadow_root_snapshot(
    dom_host: &DomHost,
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let element = dom_host
        .node(originating_element.node_id)
        .and_then(|node| node.as_element())
        .expect("select UA shadow model requires a live element");
    let size = element
        .attribute("size")
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(0);
    if element.has_attribute("multiple") || size > 1 {
        let root_identity = user_agent_shadow_identity(SELECT_LIST_BOX_TREE_KIND, 0);
        let options_slot = element_snapshot(
            originating_element,
            SELECT_LIST_BOX_TREE_KIND,
            1,
            root_identity,
            "slot",
            vec![attribute_snapshot("id", "select-options")],
            Vec::new(),
        );
        return shadow_root_snapshot(originating_element, root_identity, vec![options_slot]);
    }

    let tree_kind = SELECT_MENU_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let inner_identity = user_agent_shadow_identity(tree_kind, 1);
    let selected_label = dom_host
        .dom()
        .select_selected_option_elements(originating_element.node_id)
        .first()
        .and_then(|option_handle| {
            dom_host
                .node(*option_handle)
                .and_then(|node| node.as_element())
                .map(|option| option.option_label(dom_host.dom(), *option_handle))
        })
        .unwrap_or_default();
    let inner_children = vec![text_snapshot(
        originating_element,
        tree_kind,
        2,
        inner_identity,
        selected_label,
        0,
    )];
    let inner = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "div",
        vec![
            attribute_snapshot("aria-hidden", "true"),
            attribute_snapshot("pseudo", "-internal-select-inner-element"),
        ],
        inner_children,
    );
    let button_slot = element_snapshot(
        originating_element,
        tree_kind,
        3,
        root_identity,
        "slot",
        vec![attribute_snapshot("pseudo", "-internal-select-button-slot")],
        Vec::new(),
    );
    let picker_identity = user_agent_shadow_identity(tree_kind, 4);
    let picker_options_slot = element_snapshot(
        originating_element,
        tree_kind,
        5,
        picker_identity,
        "slot",
        vec![attribute_snapshot("id", "select-popover-options")],
        Vec::new(),
    );
    let picker = element_snapshot(
        originating_element,
        tree_kind,
        4,
        root_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "picker(select)"),
            attribute_snapshot("popover", "auto"),
        ],
        vec![picker_options_slot],
    );
    let preview_identity = user_agent_shadow_identity(tree_kind, 6);
    let preview_text = element_snapshot(
        originating_element,
        tree_kind,
        7,
        preview_identity,
        "div",
        vec![attribute_snapshot(
            "pseudo",
            "-internal-select-autofill-preview-text",
        )],
        Vec::new(),
    );
    let preview = element_snapshot(
        originating_element,
        tree_kind,
        6,
        root_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "-internal-select-autofill-preview"),
            attribute_snapshot("popover", "manual"),
        ],
        vec![preview_text],
    );
    shadow_root_snapshot(
        originating_element,
        root_identity,
        vec![inner, button_slot, picker, preview],
    )
}

fn option_shadow_root_snapshot(
    dom_host: &DomHost,
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let option = dom_host
        .node(originating_element.node_id)
        .and_then(|node| node.as_element())
        .expect("option UA shadow model requires a live element");
    let label = option.option_label(dom_host.dom(), originating_element.node_id);
    let tree_kind = OPTION_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let label_container_identity = user_agent_shadow_identity(tree_kind, 1);
    let mut label_children = Vec::new();
    if !label.is_empty() {
        let state = inspector_state_fingerprint(&label);
        label_children.push(text_snapshot(
            originating_element,
            tree_kind,
            2,
            label_container_identity,
            label,
            state,
        ));
    }
    let label_container = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "span",
        vec![
            attribute_snapshot("pseudo", "-internal-option-label-container"),
            attribute_snapshot("aria-hidden", "true"),
        ],
        label_children,
    );
    let option_slot = element_snapshot(
        originating_element,
        tree_kind,
        3,
        root_identity,
        "slot",
        vec![attribute_snapshot("pseudo", "-internal-option-slot")],
        Vec::new(),
    );
    shadow_root_snapshot(
        originating_element,
        root_identity,
        vec![label_container, option_slot],
    )
}

fn number_input_shadow_root_snapshot(
    value: String,
    has_datalist: bool,
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let tree_kind = NUMBER_INPUT_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let container_identity = user_agent_shadow_identity(tree_kind, 1);
    let viewport_identity = user_agent_shadow_identity(tree_kind, 2);
    let editor_identity = user_agent_shadow_identity(tree_kind, 3);
    let mut editor_children = Vec::new();
    if !value.is_empty() {
        let state = inspector_state_fingerprint(&value);
        editor_children.push(text_snapshot(
            originating_element,
            tree_kind,
            4,
            editor_identity,
            value,
            state,
        ));
    }
    let editor = element_snapshot(
        originating_element,
        tree_kind,
        3,
        viewport_identity,
        "div",
        Vec::new(),
        editor_children,
    );
    let viewport = element_snapshot(
        originating_element,
        tree_kind,
        2,
        container_identity,
        "div",
        vec![attribute_snapshot("id", "editing-view-port")],
        vec![editor],
    );
    let spin = element_snapshot(
        originating_element,
        tree_kind,
        5,
        container_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "-webkit-inner-spin-button"),
            attribute_snapshot("id", "spin"),
        ],
        Vec::new(),
    );
    let mut container_children = vec![viewport];
    if has_datalist {
        container_children.push(datalist_picker_snapshot(
            originating_element,
            tree_kind,
            6,
            7,
            container_identity,
        ));
    }
    container_children.push(spin);
    let container = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "div",
        vec![
            attribute_snapshot("id", "text-field-container"),
            attribute_snapshot("pseudo", "-webkit-textfield-decoration-container"),
            attribute_snapshot("style", "unicode-bidi: normal;"),
        ],
        container_children,
    );
    shadow_root_snapshot(originating_element, root_identity, vec![container])
}

fn date_input_shadow_root_snapshot(
    value: String,
    min: Option<&str>,
    max: Option<&str>,
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let tree_kind = DATE_INPUT_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let container_identity = user_agent_shadow_identity(tree_kind, 1);
    let edit_identity = user_agent_shadow_identity(tree_kind, 2);
    let fields_wrapper_identity = user_agent_shadow_identity(tree_kind, 3);
    let parts = parse_date_input_parts(&value);
    let min_year = min
        .and_then(parse_date_input_parts)
        .map(|parts| parts.year.to_string())
        .unwrap_or_else(|| "1".to_owned());
    let max_year = max
        .and_then(parse_date_input_parts)
        .map(|parts| parts.year.to_string())
        .unwrap_or_else(|| "275760".to_owned());
    let state_value = parts
        .as_ref()
        .map(|parts| {
            format!(
                "{}-{}-{}",
                parts.year_text, parts.month_text, parts.day_text
            )
        })
        .unwrap_or_default();
    let fields_state =
        inspector_state_fingerprint(&format!("{state_value}\0{min_year}\0{max_year}"));

    let month = date_field_snapshot(
        originating_element,
        tree_kind,
        4,
        5,
        fields_wrapper_identity,
        fields_state,
        "Month",
        "mm",
        "1",
        "12",
        "-webkit-datetime-edit-month-field",
        parts
            .as_ref()
            .map(|parts| (parts.month, parts.month_text.as_str())),
    );
    let first_separator = date_separator_snapshot(
        originating_element,
        tree_kind,
        6,
        7,
        fields_wrapper_identity,
        fields_state,
    );
    let day = date_field_snapshot(
        originating_element,
        tree_kind,
        8,
        9,
        fields_wrapper_identity,
        fields_state,
        "Day",
        "dd",
        "1",
        "31",
        "-webkit-datetime-edit-day-field",
        parts
            .as_ref()
            .map(|parts| (parts.day, parts.day_text.as_str())),
    );
    let second_separator = date_separator_snapshot(
        originating_element,
        tree_kind,
        10,
        11,
        fields_wrapper_identity,
        fields_state,
    );
    let year = date_field_snapshot(
        originating_element,
        tree_kind,
        12,
        13,
        fields_wrapper_identity,
        fields_state,
        "Year",
        "yyyy",
        &min_year,
        &max_year,
        "-webkit-datetime-edit-year-field",
        parts
            .as_ref()
            .map(|parts| (parts.year, parts.year_text.as_str())),
    );
    let fields_wrapper = element_snapshot(
        originating_element,
        tree_kind,
        3,
        edit_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "-webkit-datetime-edit-fields-wrapper"),
            attribute_snapshot("style", "unicode-bidi: normal;"),
        ],
        vec![month, first_separator, day, second_separator, year],
    );
    let edit = element_snapshot(
        originating_element,
        tree_kind,
        2,
        container_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "-webkit-datetime-edit"),
            attribute_snapshot("id", "date-time-edit"),
            attribute_snapshot("datetimeformat", "M/d/yy"),
            attribute_snapshot("style", "unicode-bidi: normal;"),
        ],
        vec![fields_wrapper],
    );
    let picker = element_snapshot(
        originating_element,
        tree_kind,
        14,
        container_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "-webkit-calendar-picker-indicator"),
            attribute_snapshot("id", "picker"),
            attribute_snapshot("tabindex", "0"),
            attribute_snapshot("aria-haspopup", "menu"),
            attribute_snapshot("role", "button"),
            attribute_snapshot("title", "Show date picker"),
        ],
        Vec::new(),
    );
    let container = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "-internal-datetime-container"),
            attribute_snapshot("style", "unicode-bidi: normal;"),
        ],
        vec![edit, picker],
    );
    shadow_root_snapshot(originating_element, root_identity, vec![container])
}

struct DateInputParts {
    year: u32,
    month: u32,
    day: u32,
    year_text: String,
    month_text: String,
    day_text: String,
}

fn parse_date_input_parts(value: &str) -> Option<DateInputParts> {
    if !moli_dom::forms::is_valid_date_input_value(value) {
        return None;
    }
    let mut components = value.split('-');
    let year_text = components.next()?;
    let month_text = components.next()?;
    let day_text = components.next()?;
    if components.next().is_some()
        || year_text.len() < 4
        || month_text.len() != 2
        || day_text.len() != 2
        || !year_text.bytes().all(|byte| byte.is_ascii_digit())
        || !month_text.bytes().all(|byte| byte.is_ascii_digit())
        || !day_text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some(DateInputParts {
        year: year_text.parse().ok()?,
        month: month_text.parse().ok()?,
        day: day_text.parse().ok()?,
        year_text: year_text.to_owned(),
        month_text: month_text.to_owned(),
        day_text: day_text.to_owned(),
    })
}

#[allow(clippy::too_many_arguments)]
fn date_field_snapshot(
    originating_element: &DocumentNodeSnapshot,
    tree_kind: u16,
    ordinal: u16,
    text_ordinal: u16,
    parent_identity: DocumentNodeInspectorIdentity,
    state: u64,
    label: &str,
    placeholder: &str,
    minimum: &str,
    maximum: &str,
    pseudo: &str,
    value: Option<(u32, &str)>,
) -> DocumentNodeSnapshot {
    let identity = stateful_user_agent_shadow_identity(tree_kind, ordinal, state);
    let mut attributes = vec![
        attribute_snapshot("role", "spinbutton"),
        attribute_snapshot("aria-placeholder", placeholder),
        attribute_snapshot("aria-valuemin", minimum),
        attribute_snapshot("aria-valuemax", maximum),
        attribute_snapshot("aria-label", label),
        attribute_snapshot("pseudo", pseudo),
    ];
    let text_value = if let Some((numeric_value, rendered_value)) = value {
        attributes.push(attribute_snapshot(
            "aria-valuenow",
            &numeric_value.to_string(),
        ));
        attributes.push(attribute_snapshot("aria-valuetext", rendered_value));
        rendered_value
    } else {
        placeholder
    };
    let text = text_snapshot(
        originating_element,
        tree_kind,
        text_ordinal,
        identity,
        text_value.to_owned(),
        state,
    );
    stateful_element_snapshot(
        originating_element,
        tree_kind,
        ordinal,
        state,
        parent_identity,
        "span",
        attributes,
        vec![text],
    )
}

fn date_separator_snapshot(
    originating_element: &DocumentNodeSnapshot,
    tree_kind: u16,
    ordinal: u16,
    text_ordinal: u16,
    parent_identity: DocumentNodeInspectorIdentity,
    state: u64,
) -> DocumentNodeSnapshot {
    let identity = stateful_user_agent_shadow_identity(tree_kind, ordinal, state);
    let text = text_snapshot(
        originating_element,
        tree_kind,
        text_ordinal,
        identity,
        "/".to_owned(),
        state,
    );
    stateful_element_snapshot(
        originating_element,
        tree_kind,
        ordinal,
        state,
        parent_identity,
        "div",
        vec![
            attribute_snapshot("pseudo", "-webkit-datetime-edit-text"),
            attribute_snapshot("style", "unicode-bidi: normal;"),
        ],
        vec![text],
    )
}

fn details_shadow_root_snapshot(
    dom_host: &DomHost,
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    let details = dom_host
        .node(originating_element.node_id)
        .and_then(|node| node.as_element())
        .expect("details UA shadow model requires a live element");
    let tree_kind = DETAILS_TREE_KIND;
    let root_identity = user_agent_shadow_identity(tree_kind, 0);
    let summary_slot_identity = user_agent_shadow_identity(tree_kind, 1);
    let fallback_summary_identity = user_agent_shadow_identity(tree_kind, 2);
    let fallback_text = text_snapshot(
        originating_element,
        tree_kind,
        3,
        fallback_summary_identity,
        "Details".to_owned(),
        0,
    );
    let has_author_summary = dom_host
        .child_handles(originating_element.node_id)
        .any(|child| {
            dom_host
                .node(child)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.is_html_element("summary"))
        });
    let mut fallback_summary = element_snapshot(
        originating_element,
        tree_kind,
        2,
        summary_slot_identity,
        "summary",
        Vec::new(),
        vec![fallback_text],
    );
    if !has_author_summary {
        fallback_summary
            .pseudo_elements
            .push(details_fallback_marker_snapshot(originating_element));
    }
    let summary_slot = element_snapshot(
        originating_element,
        tree_kind,
        1,
        root_identity,
        "slot",
        vec![attribute_snapshot("id", "details-summary")],
        vec![fallback_summary],
    );
    let content_style = if details.has_attribute("open") {
        "display: block;"
    } else {
        "content-visibility: hidden; display: block;"
    };
    let content_slot = element_snapshot(
        originating_element,
        tree_kind,
        4,
        root_identity,
        "slot",
        vec![
            attribute_snapshot("id", "details-content"),
            attribute_snapshot("pseudo", "details-content"),
            attribute_snapshot("style", content_style),
        ],
        Vec::new(),
    );
    let style_identity = user_agent_shadow_identity(tree_kind, 5);
    let style_text = text_snapshot(
        originating_element,
        tree_kind,
        6,
        style_identity,
        DETAILS_UA_STYLE_TEXT.to_owned(),
        0,
    );
    let style = element_snapshot(
        originating_element,
        tree_kind,
        5,
        root_identity,
        "style",
        Vec::new(),
        vec![style_text],
    );
    shadow_root_snapshot(
        originating_element,
        root_identity,
        vec![summary_slot, content_slot, style],
    )
}

fn details_fallback_marker_snapshot(
    originating_element: &DocumentNodeSnapshot,
) -> DocumentNodeSnapshot {
    DocumentNodeSnapshot {
        node_id: originating_element.node_id,
        parent_id: None,
        inspector_identity: Some(user_agent_shadow_identity(DETAILS_TREE_KIND, 7)),
        inspector_parent_identity: None,
        frontend_node_id: None,
        parent_frontend_node_id: None,
        backend_node_id: None,
        frame_id: None,
        node_type: NodeType::Element as u8,
        node_name: "::marker".to_owned(),
        local_name: "::marker".to_owned(),
        node_value: String::new(),
        child_count: 0,
        document_url: originating_element.document_url.clone(),
        base_url: originating_element.base_url.clone(),
        namespace_uri: None,
        attributes: Vec::new(),
        document_type_name: None,
        public_id: None,
        system_id: None,
        is_element: true,
        has_geometry: false,
        shadow_root_type: None,
        shadow_roots: Vec::new(),
        pseudo_type: Some("marker".to_owned()),
        pseudo_elements: Vec::new(),
        associated: None,
        children: Vec::new(),
    }
}

fn attribute_snapshot(name: &str, value: &str) -> DocumentNodeAttributeSnapshot {
    DocumentNodeAttributeSnapshot {
        local_name: name.to_owned(),
        value: value.to_owned(),
    }
}

fn user_agent_shadow_identity(tree_kind: u16, ordinal: u16) -> DocumentNodeInspectorIdentity {
    stateful_user_agent_shadow_identity(tree_kind, ordinal, 0)
}

fn stateful_user_agent_shadow_identity(
    tree_kind: u16,
    ordinal: u16,
    state: u64,
) -> DocumentNodeInspectorIdentity {
    DocumentNodeInspectorIdentity::UserAgentShadowTreeNode {
        tree_kind,
        ordinal,
        state,
    }
}

fn inspector_state_fingerprint(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |fingerprint, byte| {
            (fingerprint ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

fn normalize_textarea_api_value(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            output.push('\n');
        } else {
            output.push(ch);
        }
    }
    output
}

fn textarea_editor_children(
    originating_element: &DocumentNodeSnapshot,
    editor_identity: DocumentNodeInspectorIdentity,
    value: &str,
) -> Vec<DocumentNodeSnapshot> {
    let mut children = Vec::new();
    let mut lines = value.split('\n').peekable();
    while let Some(line) = lines.next() {
        if !line.is_empty() {
            let child_index = children.len();
            children.push(text_snapshot(
                originating_element,
                TEXTAREA_CONTROL_TREE_KIND,
                textarea_editor_child_ordinal(child_index),
                editor_identity,
                line.to_owned(),
                textarea_editor_child_state(value, child_index),
            ));
        }
        if lines.peek().is_some() {
            let child_index = children.len();
            children.push(stateful_element_snapshot(
                originating_element,
                TEXTAREA_CONTROL_TREE_KIND,
                textarea_editor_child_ordinal(child_index),
                textarea_editor_child_state(value, child_index),
                editor_identity,
                "br",
                Vec::new(),
                Vec::new(),
            ));
        }
    }
    if value.ends_with('\n') {
        let child_index = children.len();
        children.push(stateful_element_snapshot(
            originating_element,
            TEXTAREA_CONTROL_TREE_KIND,
            textarea_editor_child_ordinal(child_index),
            textarea_editor_child_state(value, child_index),
            editor_identity,
            "br",
            Vec::new(),
            Vec::new(),
        ));
    }
    children
}

fn textarea_editor_child_ordinal(child_index: usize) -> u16 {
    u16::try_from(child_index.saturating_add(2)).unwrap_or(u16::MAX)
}

fn textarea_editor_child_state(value: &str, child_index: usize) -> u64 {
    u64::try_from(child_index)
        .unwrap_or(u64::MAX)
        .to_le_bytes()
        .iter()
        .fold(inspector_state_fingerprint(value), |fingerprint, byte| {
            (fingerprint ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

fn shadow_root_snapshot(
    originating_element: &DocumentNodeSnapshot,
    identity: DocumentNodeInspectorIdentity,
    children: Vec<DocumentNodeSnapshot>,
) -> DocumentNodeSnapshot {
    DocumentNodeSnapshot {
        node_id: originating_element.node_id,
        parent_id: None,
        inspector_identity: Some(identity),
        inspector_parent_identity: None,
        frontend_node_id: None,
        parent_frontend_node_id: None,
        backend_node_id: None,
        frame_id: None,
        node_type: NodeType::DocumentFragment as u8,
        node_name: "#document-fragment".to_owned(),
        local_name: String::new(),
        node_value: String::new(),
        child_count: children.len(),
        document_url: originating_element.document_url.clone(),
        base_url: originating_element.base_url.clone(),
        namespace_uri: None,
        attributes: Vec::new(),
        document_type_name: None,
        public_id: None,
        system_id: None,
        is_element: false,
        has_geometry: false,
        shadow_root_type: Some("user-agent".to_owned()),
        shadow_roots: Vec::new(),
        pseudo_type: None,
        pseudo_elements: Vec::new(),
        associated: None,
        children,
    }
}

fn element_snapshot(
    originating_element: &DocumentNodeSnapshot,
    tree_kind: u16,
    ordinal: u16,
    parent_identity: DocumentNodeInspectorIdentity,
    local_name: &str,
    attributes: Vec<DocumentNodeAttributeSnapshot>,
    children: Vec<DocumentNodeSnapshot>,
) -> DocumentNodeSnapshot {
    stateful_element_snapshot(
        originating_element,
        tree_kind,
        ordinal,
        0,
        parent_identity,
        local_name,
        attributes,
        children,
    )
}

#[allow(clippy::too_many_arguments)]
fn stateful_element_snapshot(
    originating_element: &DocumentNodeSnapshot,
    tree_kind: u16,
    ordinal: u16,
    state: u64,
    parent_identity: DocumentNodeInspectorIdentity,
    local_name: &str,
    attributes: Vec<DocumentNodeAttributeSnapshot>,
    children: Vec<DocumentNodeSnapshot>,
) -> DocumentNodeSnapshot {
    DocumentNodeSnapshot {
        node_id: originating_element.node_id,
        parent_id: None,
        inspector_identity: Some(stateful_user_agent_shadow_identity(
            tree_kind, ordinal, state,
        )),
        inspector_parent_identity: Some(parent_identity),
        frontend_node_id: None,
        parent_frontend_node_id: None,
        backend_node_id: None,
        frame_id: None,
        node_type: NodeType::Element as u8,
        node_name: local_name.to_ascii_uppercase(),
        local_name: local_name.to_owned(),
        node_value: String::new(),
        child_count: children.len(),
        document_url: originating_element.document_url.clone(),
        base_url: originating_element.base_url.clone(),
        namespace_uri: Some(HTML_NAMESPACE.to_owned()),
        attributes,
        document_type_name: None,
        public_id: None,
        system_id: None,
        is_element: true,
        has_geometry: false,
        shadow_root_type: None,
        shadow_roots: Vec::new(),
        pseudo_type: None,
        pseudo_elements: Vec::new(),
        associated: None,
        children,
    }
}

fn text_snapshot(
    originating_element: &DocumentNodeSnapshot,
    tree_kind: u16,
    ordinal: u16,
    parent_identity: DocumentNodeInspectorIdentity,
    value: String,
    state: u64,
) -> DocumentNodeSnapshot {
    DocumentNodeSnapshot {
        node_id: originating_element.node_id,
        parent_id: None,
        inspector_identity: Some(stateful_user_agent_shadow_identity(
            tree_kind, ordinal, state,
        )),
        inspector_parent_identity: Some(parent_identity),
        frontend_node_id: None,
        parent_frontend_node_id: None,
        backend_node_id: None,
        frame_id: None,
        node_type: NodeType::Text as u8,
        node_name: "#text".to_owned(),
        local_name: String::new(),
        node_value: value,
        child_count: 0,
        document_url: originating_element.document_url.clone(),
        base_url: originating_element.base_url.clone(),
        namespace_uri: None,
        attributes: Vec::new(),
        document_type_name: None,
        public_id: None,
        system_id: None,
        is_element: false,
        has_geometry: false,
        shadow_root_type: None,
        shadow_roots: Vec::new(),
        pseudo_type: None,
        pseudo_elements: Vec::new(),
        associated: None,
        children: Vec::new(),
    }
}

fn truncate_snapshot_to_depth(
    mut snapshot: DocumentNodeSnapshot,
    depth: i32,
) -> DocumentNodeSnapshot {
    if depth == 0 {
        snapshot.children.clear();
        snapshot.shadow_roots.clear();
        snapshot.pseudo_elements = snapshot
            .pseudo_elements
            .into_iter()
            .map(|pseudo_element| truncate_snapshot_to_depth(pseudo_element, 0))
            .collect();
        snapshot.associated = None;
        return snapshot;
    }
    let next_depth = if depth > 0 { depth - 1 } else { depth };
    snapshot.children = snapshot
        .children
        .into_iter()
        .map(|child| truncate_snapshot_to_depth(child, next_depth))
        .collect();
    snapshot
}

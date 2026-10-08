use super::*;

pub(super) fn resolve_computed_custom_function_calls(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    inputs: &FullStyleWorldSnapshot,
    context: StyleComputationContext,
) -> String {
    if !property.starts_with("--") || !value.contains("--") || !value.contains("()") {
        return value.to_owned();
    }
    let calls = dashed_no_arg_function_calls(value);
    if calls.is_empty() {
        return value.to_owned();
    }
    let scope = computed_custom_property_source_scope(
        runtime,
        handle,
        property,
        value,
        inputs,
        context.viewport_width(),
    )
    .or_else(|| runtime.dom_host().containing_shadow_root(handle));
    let Some(scope) = scope else {
        return value.to_owned();
    };
    let functions = visible_custom_functions_for_scope(runtime, inputs, scope, context.viewport());
    if functions.is_empty() {
        return value.to_owned();
    }
    let mut resolved = value.to_owned();
    let mut changed = false;
    for call in calls {
        let Some(function) = functions.get(call.as_str()) else {
            continue;
        };
        let Some(result) = evaluate_custom_function(runtime, handle, function, context) else {
            continue;
        };
        resolved = resolved.replace(&format!("{call}()"), &result);
        changed = true;
    }
    if changed { resolved } else { value.to_owned() }
}

pub(super) fn dashed_no_arg_function_calls(value: &str) -> Vec<String> {
    let mut calls = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index + 4 <= bytes.len() {
        if bytes[index] != b'-' || bytes.get(index + 1) != Some(&b'-') {
            index += 1;
            continue;
        }
        let start = index;
        index += 2;
        while index < bytes.len() && css_identifier_byte(bytes[index]) {
            index += 1;
        }
        if index == start + 2 || bytes.get(index) != Some(&b'(') {
            continue;
        }
        let mut close = index + 1;
        while close < bytes.len() && bytes[close].is_ascii_whitespace() {
            close += 1;
        }
        if bytes.get(close) != Some(&b')') {
            continue;
        }
        let name = &value[start..index];
        if !calls.iter().any(|call| call == name) {
            calls.push(name.to_owned());
        }
        index = close + 1;
    }
    calls
}

fn css_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn computed_custom_property_source_scope(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    inputs: &FullStyleWorldSnapshot,
    viewport_width: Option<f64>,
) -> Option<DomHandle> {
    for (root, sources) in inputs.shadow_stylesheet_sources.iter().rev() {
        if stylesheet_sources_compute_custom_property_value(
            runtime,
            handle,
            property,
            value,
            inputs,
            Some((*root, sources)),
            viewport_width,
        ) {
            return Some(*root);
        }
    }
    if stylesheet_sources_compute_custom_property_value(
        runtime,
        handle,
        property,
        value,
        inputs,
        None,
        viewport_width,
    ) {
        return runtime.dom_host().owner_document_handle(handle);
    }
    None
}

fn stylesheet_sources_compute_custom_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    inputs: &FullStyleWorldSnapshot,
    scoped_sources: Option<(DomHandle, &[StyloStylesheetSource])>,
    viewport_width: Option<f64>,
) -> bool {
    let mut scoped_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: Vec::new(),
        shadow_stylesheet_sources: Vec::new(),
        script_custom_property_registrations: inputs.script_custom_property_registrations.clone(),
        environment: inputs.environment,
        quirks_mode: inputs.quirks_mode,
    };
    if let Some((root, sources)) = scoped_sources {
        scoped_inputs
            .shadow_stylesheet_sources
            .push((root, sources.to_vec()));
    } else {
        scoped_inputs.document_stylesheet_sources = inputs.document_stylesheet_sources.clone();
    }
    let viewport = StyleViewport {
        width: viewport_width.or_else(|| runtime.style_viewport().width),
        ..runtime.style_viewport()
    };
    let Some(read_document) = runtime.dom_host().owner_document_handle(handle) else {
        return false;
    };
    runtime
        .computed_style_property_value_from_ephemeral_stylo(
            handle,
            property,
            &scoped_inputs,
            read_document,
            viewport,
        )
        .is_some_and(|candidate| candidate.trim() == value.trim())
}

#[derive(Clone, Debug)]
pub(super) struct CustomCssFunction {
    pub(super) result: Option<String>,
    pub(super) container_results: Vec<CustomCssFunctionContainerResult>,
}

#[derive(Clone, Debug)]
pub(super) struct CustomCssFunctionContainerResult {
    pub(super) container_name: String,
    pub(super) width_px: f64,
    pub(super) result: String,
}

struct CustomFunctionRuleText {
    pub(super) name: String,
    pub(super) block: String,
}

pub(super) struct CustomFunctionContainerRuleText {
    pub(super) css_text: String,
    pub(super) block: String,
}

fn visible_custom_functions_for_scope(
    runtime: &JsContextHost,
    inputs: &FullStyleWorldSnapshot,
    scope: DomHandle,
    viewport: StyleViewport,
) -> HashMap<String, CustomCssFunction> {
    let mut functions = HashMap::new();
    for source in inputs
        .document_stylesheet_sources
        .iter()
        .filter(|source| source.media_matches(runtime.emulated_media(), viewport))
    {
        collect_custom_functions_from_stylesheet_source(runtime, source, &mut functions);
    }
    if runtime
        .dom_host()
        .node(scope)
        .is_some_and(Node::is_document)
    {
        return functions;
    }
    let shadow_chain = shadow_root_ancestor_chain(runtime, scope);
    for (root, sources) in &inputs.shadow_stylesheet_sources {
        if shadow_chain.contains(root) {
            for source in sources
                .iter()
                .filter(|source| source.media_matches(runtime.emulated_media(), viewport))
            {
                collect_custom_functions_from_stylesheet_source(runtime, source, &mut functions);
            }
        }
    }
    functions
}

fn collect_custom_functions_from_stylesheet_source(
    runtime: &JsContextHost,
    source: &StyloStylesheetSource,
    functions: &mut HashMap<String, CustomCssFunction>,
) {
    if let Some(owner) = source.owner_style_sheet_owner()
        && let Some(processing_source) = runtime.owner_style_sheet_processing_source(owner)
    {
        // `@function` is a renderer compatibility extension that Stylo does not
        // retain in its parsed rule tree. Inline owners therefore project this
        // extension from their immutable processing input while cascade and
        // CSSOM continue to share the live Stylo stylesheet.
        collect_custom_functions_from_css(processing_source.css_text(), functions);
        return;
    }
    collect_custom_functions_from_css(&source.serialized_css_text(), functions);
}

fn shadow_root_ancestor_chain(runtime: &JsContextHost, scope: DomHandle) -> Vec<DomHandle> {
    let mut chain = Vec::new();
    let mut current = Some(scope);
    while let Some(root) = current {
        if !runtime.dom_host().is_shadow_root(root) {
            break;
        }
        chain.push(root);
        current = runtime
            .dom_host()
            .shadow_root_host(root)
            .and_then(|host| runtime.dom_host().containing_shadow_root(host));
    }
    chain.reverse();
    chain
}

pub(super) fn collect_custom_functions_from_css(
    css_text: &str,
    functions: &mut HashMap<String, CustomCssFunction>,
) {
    for rule in custom_function_rule_texts(css_text) {
        if let Some(function) = parse_custom_css_function(&rule.block) {
            functions.insert(rule.name, function);
        }
    }
}

fn custom_function_name(prelude: &str) -> Option<String> {
    let trimmed = prelude.trim();
    let open = trimmed.find('(')?;
    let name = trimmed[..open].trim();
    let after_open = &trimmed[open + 1..];
    let close = after_open.find(')')?;
    if !after_open[..close].trim().is_empty() {
        return None;
    }
    if !name.starts_with("--") || name.len() == 2 {
        return None;
    }
    Some(name.to_owned())
}

fn parse_custom_css_function(block: &str) -> Option<CustomCssFunction> {
    let declarations = moli_css_parse::parse_declaration_list(
        block,
        moli_css_parse::DeclarationParseOptions {
            canonicalize_property_name: false,
            unescape_value_semicolons: true,
            preserve_empty_values: false,
        },
    );
    let result = declarations
        .into_iter()
        .rev()
        .find(|declaration| declaration.name.eq_ignore_ascii_case("result"))
        .map(|declaration| declaration.value);
    let mut container_results = Vec::new();
    for rule in custom_function_container_rule_texts(block) {
        let Some(container) = parse_custom_function_container_rule(&rule) else {
            continue;
        };
        container_results.push(container);
    }
    if result.is_none() && container_results.is_empty() {
        return None;
    }
    Some(CustomCssFunction {
        result,
        container_results,
    })
}

fn parse_custom_function_container_rule(
    rule: &CustomFunctionContainerRuleText,
) -> Option<CustomCssFunctionContainerResult> {
    let view = moli_css_parse::parse_condition_rule_view_with_stylo(&rule.css_text)?;
    if view.rule_type != CssRuleType::Container {
        return None;
    }
    let width_px = container_query_width_equality_px(view.container_query.as_deref()?)?;
    let declarations = moli_css_parse::parse_declaration_list(
        &rule.block,
        moli_css_parse::DeclarationParseOptions {
            canonicalize_property_name: false,
            unescape_value_semicolons: true,
            preserve_empty_values: false,
        },
    );
    let result = declarations
        .into_iter()
        .rev()
        .find(|declaration| declaration.name.eq_ignore_ascii_case("result"))?
        .value;
    Some(CustomCssFunctionContainerResult {
        container_name: view.container_name.unwrap_or_default(),
        width_px,
        result,
    })
}

fn custom_function_rule_texts(css_text: &str) -> Vec<CustomFunctionRuleText> {
    custom_css_projection_at_rules(css_text)
        .into_iter()
        .filter_map(|rule| {
            if !rule.name.eq_ignore_ascii_case("function") {
                return None;
            }
            Some(CustomFunctionRuleText {
                name: custom_function_name(&rule.prelude)?,
                block: rule.block?,
            })
        })
        .collect()
}

pub(super) fn custom_function_container_rule_texts(
    block: &str,
) -> Vec<CustomFunctionContainerRuleText> {
    custom_css_projection_at_rules(block)
        .into_iter()
        .filter_map(|rule| {
            if !rule.name.eq_ignore_ascii_case("container") {
                return None;
            }
            Some(CustomFunctionContainerRuleText {
                css_text: rule.css_text,
                block: rule.block?,
            })
        })
        .collect()
}

fn container_query_width_equality_px(query: &str) -> Option<f64> {
    let query = query.trim();
    let inner = query.strip_prefix('(')?.strip_suffix(')')?.trim();
    let (feature, value) = inner.split_once('=')?;
    if feature.trim() != "width" {
        return None;
    }
    moli_css_parse::parse_px_length(value, moli_css_parse::UnitlessLength::ZeroOnly)
}

fn evaluate_custom_function(
    runtime: &JsContextHost,
    handle: DomHandle,
    function: &CustomCssFunction,
    context: StyleComputationContext,
) -> Option<String> {
    for container in &function.container_results {
        if named_container_width(runtime, handle, &container.container_name, context)
            .is_some_and(|width| css_px_values_equal(width, container.width_px))
        {
            return Some(container.result.clone());
        }
    }
    function.result.clone()
}

fn named_container_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    container_name: &str,
    context: StyleComputationContext,
) -> Option<f64> {
    let mut current = flat_tree_element_parent(runtime, handle);
    let mut visited = HashSet::new();
    while let Some(candidate) = current {
        if !visited.insert(candidate) {
            return None;
        }
        if element_is_named_size_container(runtime, candidate, container_name, context) {
            return inline_width_px(runtime, candidate);
        }
        current = flat_tree_element_parent(runtime, candidate);
    }
    None
}

pub(super) fn flat_tree_element_parent(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    if let Some(slot) = runtime.dom_host().assigned_slot_for_node(handle) {
        return runtime
            .dom_host()
            .node(slot)
            .is_some_and(Node::is_element)
            .then_some(slot);
    }
    if runtime.dom_host().is_shadow_root(handle) {
        return runtime.dom_host().shadow_root_host(handle);
    }
    let parent = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)?;
    if runtime.dom_host().is_shadow_root(parent) {
        return runtime.dom_host().shadow_root_host(parent);
    }
    runtime
        .dom_host()
        .node(parent)
        .is_some_and(Node::is_element)
        .then_some(parent)
}

fn element_is_named_size_container(
    runtime: &JsContextHost,
    handle: DomHandle,
    container_name: &str,
    context: StyleComputationContext,
) -> bool {
    let name = style_property_value_with_context(
        runtime,
        handle,
        StyleMode::Computed,
        "container-name",
        context,
    );
    if !container_name_list_contains(&name, container_name) {
        return false;
    }
    let ty = style_property_value_with_context(
        runtime,
        handle,
        StyleMode::Computed,
        "container-type",
        context,
    );
    container_type_is_size_container(&ty)
}

fn container_name_list_contains(value: &str, container_name: &str) -> bool {
    value.split_whitespace().any(|name| name == container_name)
}

pub(super) fn container_type_is_size_container(value: &str) -> bool {
    value
        .split_whitespace()
        .any(|ty| matches!(ty, "size" | "inline-size"))
}

fn inline_width_px(runtime: &JsContextHost, handle: DomHandle) -> Option<f64> {
    let read = ComputedStyleRead::new(runtime, handle);
    inline_width_px_with_resolution(runtime, handle, read.resolution_context())
}

pub(super) fn inline_width_px_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    let inline = style_property_value(runtime, handle, StyleMode::Inline, "width");
    let computed = if inline.is_empty() {
        resolution.computed_property(runtime, handle, "width")
    } else {
        inline
    };
    moli_css_parse::parse_px_length(&computed, moli_css_parse::UnitlessLength::ZeroOnly)
}

fn css_px_values_equal(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.001
}

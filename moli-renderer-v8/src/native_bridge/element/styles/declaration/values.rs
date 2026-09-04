use crate::css_custom_function::custom_css_projection_at_rules;
use crate::{
    dom::native::Node,
    {
        css_style::{
            CssStyleEntry as StyleEntry, background_shorthand_color, border_shorthand_color,
            border_shorthand_style, border_shorthand_width, box_shorthand_value_components,
            canonical_style_property_name, ident_is_system_color,
            normalize_cssom_component_value_serialization_with_spaced_slash,
            normalize_cssom_flex_basis_value, normalize_cssom_flex_shorthand_value,
            resolve_css_url_function, system_color_rgb, top_level_comma_separated_component_values,
        },
        document_runtime::DomHandle,
        native_bridge::element::geometry::{
            ClientRect, observable_bounding_client_rect, observable_bounding_client_rects,
            observable_used_grid_tracks,
        },
        style_engine::{
            ComputedDisplayKind, ComputedRenderedStyleFacts, FullStyleWorldSnapshot, StyleViewport,
            StylesheetResourceSnapshot, StyloAnonymousBoxKind, StyloComputedStyleSnapshot,
            StyloStylesheetSource, computed_property_is_queryable,
        },
    },
};
use cssparser::{Parser, ParserInput, Token, serialize_identifier, serialize_string};
use moli_selector::html_directionality;
use std::collections::{HashMap, HashSet};
use style::{properties::ComputedValues, servo_arc::Arc as ServoArc, stylesheets::CssRuleType};

use super::super::super::super::JsContextHost;
use super::observation::{
    ComputedStyleRead, RetainedStyleObservation, StyleComputationContext, StyleObservation,
};
use super::style_world::{
    effective_raw_stylesheet_sources, style_base_url, stylesheet_source_document_for_handle,
};
use super::{
    StyleMode, all_shorthand_applies_to, animation_shorthand_longhands, css_wide_keyword,
    font_variant_longhands, inline_state_property_priority_with_pdb,
    inline_state_property_value_with_pdb, known_style_property, normalize_css_integer_token,
    parse_inline_css_text_with_base, shorthand_longhands, style_entries,
    style_entries_property_priority_with_pdb, style_entries_property_value_with_pdb,
    text_decoration_shorthand_longhands, transition_shorthand_longhands,
};

#[derive(Clone, Copy)]
struct StyleResolutionContext<'a> {
    computation: StyleComputationContext,
    inputs: Option<&'a FullStyleWorldSnapshot>,
    observation: Option<&'a dyn RetainedStyleObservation>,
    retained_style: Option<(DomHandle, &'a StyloComputedStyleSnapshot)>,
}

impl<'a> StyleResolutionContext<'a> {
    const fn independent(computation: StyleComputationContext) -> Self {
        Self {
            computation,
            inputs: None,
            observation: None,
            retained_style: None,
        }
    }

    const fn prepared(
        computation: StyleComputationContext,
        inputs: &'a FullStyleWorldSnapshot,
    ) -> Self {
        Self {
            computation,
            inputs: Some(inputs),
            observation: None,
            retained_style: None,
        }
    }

    const fn retained(
        computation: StyleComputationContext,
        inputs: &'a FullStyleWorldSnapshot,
        handle: DomHandle,
        style: &'a StyloComputedStyleSnapshot,
    ) -> Self {
        Self {
            computation,
            inputs: Some(inputs),
            observation: None,
            retained_style: Some((handle, style)),
        }
    }

    const fn retained_without_inputs(
        computation: StyleComputationContext,
        handle: DomHandle,
        style: &'a StyloComputedStyleSnapshot,
    ) -> Self {
        Self {
            computation,
            inputs: None,
            observation: None,
            retained_style: Some((handle, style)),
        }
    }

    const fn observed(
        computation: StyleComputationContext,
        inputs: Option<&'a FullStyleWorldSnapshot>,
        observation: &'a dyn RetainedStyleObservation,
        handle: DomHandle,
        style: Option<&'a StyloComputedStyleSnapshot>,
    ) -> Self {
        Self {
            computation,
            inputs,
            observation: Some(observation),
            retained_style: match style {
                Some(style) => Some((handle, style)),
                None => None,
            },
        }
    }

    fn computed_property(
        self,
        runtime: &JsContextHost,
        handle: DomHandle,
        property: &str,
    ) -> String {
        if let Some(inputs) = self.inputs {
            return computed_style_property_value_with_prepared_inputs(
                runtime,
                handle,
                property,
                inputs,
                self.computation,
                self.retained_style
                    .filter(|(retained_handle, _)| *retained_handle == handle)
                    .map(|(_, style)| style),
            );
        }
        if let Some(observation) = self.observation
            && let Some(style) = observation.style_snapshot(handle)
        {
            return computed_style_property_value_after_style_update(
                runtime,
                handle,
                property,
                self.computation,
                None,
                Some(observation),
                Some(&style),
            );
        }
        style_property_value_with_context(
            runtime,
            handle,
            StyleMode::Computed,
            property,
            self.computation,
        )
    }

    fn raw_property(self, runtime: &JsContextHost, handle: DomHandle, property: &str) -> String {
        if let Some((retained_handle, style)) = self.retained_style
            && retained_handle == handle
        {
            return style.property_value(property).unwrap_or_default();
        }
        if let Some(inputs) = self.inputs {
            return raw_stylo_computed_style_value_with_inputs(
                runtime,
                handle,
                property,
                inputs,
                self.computation,
            );
        }
        if let Some(observation) = self.observation {
            return observation
                .style_snapshot(handle)
                .and_then(|style| style.property_value(property))
                .unwrap_or_default();
        }
        raw_stylo_computed_style_value(runtime, handle, property)
    }
}

impl<'a> ComputedStyleRead<'a> {
    pub(crate) fn new(runtime: &'a JsContextHost, handle: DomHandle) -> Self {
        StyleObservation::new(runtime).read(handle)
    }

    pub(in crate::native_bridge::element::styles) fn new_with_context(
        runtime: &'a JsContextHost,
        handle: DomHandle,
        context: StyleComputationContext,
    ) -> Self {
        StyleObservation::new_with_context(runtime, context).read(handle)
    }

    pub(in crate::native_bridge::element) fn property(&self, property: &str) -> String {
        let Some(property) = canonical_computed_cssom_query_property_name(property) else {
            return String::new();
        };
        self.property_in_prepared_scope(&property)
    }

    fn resolution_context(&self) -> StyleResolutionContext<'_> {
        StyleResolutionContext::observed(
            self.context,
            None,
            self.observation_inputs.as_ref(),
            self.handle,
            self.stylo_style.as_ref(),
        )
    }

    pub(in crate::native_bridge::element) fn rendered_style_facts(
        &self,
    ) -> Option<ComputedRenderedStyleFacts> {
        let mut facts = self.stylo_style.as_ref()?.rendered_style_facts();
        if element_has_hidden_attribute(self.runtime, self.handle) {
            facts.display = ComputedDisplayKind::None;
        }
        Some(facts)
    }

    pub(crate) fn computed_values(&self) -> Option<ServoArc<ComputedValues>> {
        self.stylo_style
            .as_ref()
            .map(StyloComputedStyleSnapshot::computed_values)
    }

    /// Transfers the observation's owned Stylo handles into box construction.
    /// The only reference-count increments happened when the canonical
    /// `ElementStyles` crossed the renderer borrow boundary.
    pub(crate) fn into_element_computed_values(
        self,
    ) -> Option<(
        ServoArc<ComputedValues>,
        Option<ServoArc<ComputedValues>>,
        Option<ServoArc<ComputedValues>>,
    )> {
        self.stylo_style
            .map(StyloComputedStyleSnapshot::into_element_computed_values)
    }

    /// Returns the typed resource manifest published with this read's exact
    /// retained stylesheet world. No CSS text is serialized or reparsed.
    pub(crate) fn stylesheet_resource_snapshot(&self) -> Option<StylesheetResourceSnapshot> {
        let document = self.observation_inputs.source_document()?;
        self.runtime
            .stylesheet_resource_snapshot_for_document(document)
    }

    pub(crate) fn pseudo_computed_values(
        &self,
        pseudo_element: &str,
    ) -> Option<ServoArc<ComputedValues>> {
        self.stylo_style.as_ref()?;
        let read_document = self
            .context
            .resolved_read_document(self.runtime, self.handle);
        self.runtime
            .computed_pseudo_style_snapshot_from_current_observation(
                self.handle,
                pseudo_element,
                read_document,
            )
            .map(|snapshot| snapshot.computed_values())
    }

    pub(crate) fn anonymous_computed_values(
        &self,
        parent_style: &ComputedValues,
        anonymous_kind: StyloAnonymousBoxKind,
    ) -> Option<ServoArc<ComputedValues>> {
        self.stylo_style.as_ref()?;
        let read_document = self
            .context
            .resolved_read_document(self.runtime, self.handle);
        self.runtime
            .computed_anonymous_style_snapshot_from_current_observation(
                self.handle,
                parent_style,
                anonymous_kind,
                read_document,
            )
            .map(|snapshot| snapshot.computed_values())
    }

    fn property_in_prepared_scope(&self, property: &str) -> String {
        let stylesheet_query_snapshot =
            computed_property_requires_stylesheet_sources(property, self.stylo_style.as_ref())
                .then(|| self.observation_inputs.stylesheet_query_snapshot());
        computed_style_property_value_after_style_update(
            self.runtime,
            self.handle,
            property,
            self.context,
            stylesheet_query_snapshot.as_deref(),
            Some(self.observation_inputs.as_ref()),
            self.stylo_style.as_ref(),
        )
    }

    pub(in crate::native_bridge::element) fn raw_pseudo_property(
        &self,
        pseudo_element: &str,
        property: &str,
    ) -> String {
        let Some(property) = canonical_computed_cssom_query_property_name(property) else {
            return String::new();
        };
        let read_document = self
            .context
            .resolved_read_document(self.runtime, self.handle);
        self.runtime
            .computed_pseudo_style_snapshot_from_current_observation(
                self.handle,
                pseudo_element,
                read_document,
            )
            .and_then(|style| style.resolved_property_value(&property))
            .unwrap_or_default()
    }

    fn raw_primary_property(&self, property: &str) -> Option<String> {
        let property = canonical_computed_cssom_query_property_name(property)?;
        self.stylo_style
            .as_ref()
            .and_then(|style| style.property_value(&property))
    }

    pub(in crate::native_bridge::element::styles) fn custom_property_names(&self) -> Vec<String> {
        self.stylo_style
            .as_ref()
            .map(StyloComputedStyleSnapshot::custom_property_names)
            .unwrap_or_default()
    }

    pub(in crate::native_bridge::element::styles) fn property_names(&self) -> Vec<String> {
        super::super::computed_names::computed_property_names_for_read(self)
    }

    pub(in crate::native_bridge::element::styles) fn properties(&self) -> Vec<(String, String)> {
        self.property_names()
            .into_iter()
            .map(|name| {
                let value = self.property_in_prepared_scope(&name);
                (name, value)
            })
            .collect()
    }
}

pub(crate) fn css_animation_start_applies(runtime: &JsContextHost, handle: DomHandle) -> bool {
    // This is a yes/no predicate for queuing animationstart, not a computed-style
    // value read. Resolve active animation names once and scan keyframes for the
    // small property subset Moli models instead of rewalking stylesheets once
    // per supported property.
    if !runtime
        .dom_host()
        .node(handle)
        .is_some_and(|node| node.is_connected() && node.as_element().is_some())
    {
        return false;
    }

    let read = ComputedStyleRead::new(runtime, handle);
    let resolution = read.resolution_context();
    let names = active_css_animation_names_with_resolution(runtime, handle, resolution);
    if names.is_empty() {
        return false;
    }

    let Some(document) = stylesheet_source_document_for_handle(runtime, handle) else {
        return false;
    };
    for source in effective_raw_stylesheet_sources(
        runtime,
        document,
        false,
        resolution.computation.viewport(),
    ) {
        if keyframe_has_supported_animation_values(&source.serialized_css_text(), &names) {
            return true;
        }
    }
    false
}

fn active_css_animation_midpoint_px_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    let (from, to) = active_css_animation_property_values_with_resolution(
        runtime, handle, property, resolution,
    )?;
    let from = moli_css_parse::parse_px_length(&from, moli_css_parse::UnitlessLength::ZeroOnly)?;
    let to = moli_css_parse::parse_px_length(&to, moli_css_parse::UnitlessLength::ZeroOnly)?;
    Some((from + to) / 2.0)
}

fn active_registered_length_custom_property_animation_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    underlying_value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if !property.starts_with("--") {
        return None;
    }
    let document = stylesheet_source_document_for_handle(runtime, handle)?;
    let registration = runtime.registered_css_custom_property_registration(document, property)?;
    if registration.syntax.trim() != "<length>" {
        return None;
    }
    let (from, to) = active_css_animation_property_values_with_resolution(
        runtime, handle, property, resolution,
    )?;
    let from = registered_length_keyframe_endpoint_px(&from, underlying_value)?;
    let to = registered_length_keyframe_endpoint_px(&to, underlying_value)?;
    Some(format!("{}px", (from + to) / 2.0))
}

fn registered_length_keyframe_endpoint_px(value: &str, underlying_value: &str) -> Option<f64> {
    let value = if value.trim().eq_ignore_ascii_case("revert") {
        underlying_value
    } else {
        value
    };
    moli_css_parse::parse_px_length(value, moli_css_parse::UnitlessLength::ZeroOnly)
}

fn active_css_animation_translate_x_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    let (from, to) = active_css_animation_property_values_with_resolution(
        runtime,
        handle,
        "transform",
        resolution,
    )?;
    let from = transform_translate_x_px(&from)?;
    let to = transform_translate_x_px(&to)?;
    Some((from + to) / 2.0)
}

pub(crate) fn active_css_animation_transform_value(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let read = ComputedStyleRead::new(runtime, handle);
    active_css_animation_transform_value_with_resolution(runtime, handle, read.resolution_context())
}

fn active_css_animation_transform_value_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    active_css_animation_translate_x_with_resolution(runtime, handle, resolution)
        .map(|value| format!("translateX({value}px)"))
}

fn active_css_animation_static_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let (from, to) = active_css_animation_property_values_with_resolution(
        runtime, handle, property, resolution,
    )?;
    (from.eq_ignore_ascii_case(&to)).then_some(from)
}

fn active_css_animation_property_values_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<(String, String)> {
    let names = active_css_animation_names_with_resolution(runtime, handle, resolution);
    if names.is_empty() {
        return None;
    }
    let document = stylesheet_source_document_for_handle(runtime, handle)?;
    for source in effective_raw_stylesheet_sources(
        runtime,
        document,
        false,
        resolution.computation.viewport(),
    ) {
        if let Some(values) =
            keyframe_property_values(&source.serialized_css_text(), &names, property)
        {
            return Some(values);
        }
    }
    None
}

fn active_css_animation_names_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Vec<String> {
    let animation_name = resolution.computed_property(runtime, handle, "animation-name");
    let mut names = animation_name
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("none"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    // A non-empty computed animation-name (including the initial `none`) is
    // authoritative. Only use the legacy shorthand fallback when the
    // longhand serializer itself is unavailable.
    if names.is_empty() && animation_name.trim().is_empty() {
        let animation = resolution.computed_property(runtime, handle, "animation");
        names = animation_shorthand_names(&animation);
    }
    names
}

fn animation_shorthand_names(value: &str) -> Vec<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let mut names = Vec::new();
    let mut current = None;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::Comma => {
                if let Some(name) = current.take() {
                    names.push(name);
                }
            }
            Token::Ident(value) if !ident_is_animation_shorthand_keyword(&value) => {
                current = Some(value.to_string());
            }
            Token::Function(_) => {
                let _ = input.parse_nested_block(|_| Ok::<_, cssparser::ParseError<'_, ()>>(()));
            }
            _ => {}
        }
    }
    if let Some(name) = current {
        names.push(name);
    }
    names
}

fn ident_is_animation_shorthand_keyword(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "none"
            | "normal"
            | "linear"
            | "ease"
            | "ease-in"
            | "ease-out"
            | "ease-in-out"
            | "step-start"
            | "step-end"
            | "infinite"
            | "alternate"
            | "alternate-reverse"
            | "reverse"
            | "forwards"
            | "backwards"
            | "both"
            | "running"
            | "paused"
    )
}

fn keyframe_property_values(
    css_text: &str,
    animation_names: &[String],
    property: &str,
) -> Option<(String, String)> {
    let rules = moli_css_parse::parse_stylesheet_rule_snapshots_with_stylo(css_text);
    keyframe_rule_snapshots_property_values(&rules, animation_names, property, 0)
}

const KEYFRAME_NESTING_DEPTH_LIMIT: usize = 32;

fn keyframe_has_supported_animation_values(css_text: &str, animation_names: &[String]) -> bool {
    let rules = moli_css_parse::parse_stylesheet_rule_snapshots_with_stylo(css_text);
    keyframe_rule_snapshots_have_supported_animation_values(&rules, animation_names, 0)
}

fn keyframe_rule_snapshots_property_values(
    rules: &[moli_css_parse::CssRuleSnapshot],
    animation_names: &[String],
    property: &str,
    depth: usize,
) -> Option<(String, String)> {
    if depth > KEYFRAME_NESTING_DEPTH_LIMIT {
        return None;
    }
    for rule in rules {
        match rule.rule_type {
            CssRuleType::Keyframes if keyframe_rule_name_matches(rule, animation_names) => {
                if let Some(values) =
                    keyframe_child_rule_snapshots_property_values(&rule.child_rules, property)
                {
                    return Some(values);
                }
            }
            CssRuleType::Media | CssRuleType::Supports => {
                if let Some(values) = keyframe_rule_snapshots_property_values(
                    &rule.child_rules,
                    animation_names,
                    property,
                    depth + 1,
                ) {
                    return Some(values);
                }
            }
            _ => {}
        }
    }
    None
}

fn keyframe_rule_snapshots_have_supported_animation_values(
    rules: &[moli_css_parse::CssRuleSnapshot],
    animation_names: &[String],
    depth: usize,
) -> bool {
    if depth > KEYFRAME_NESTING_DEPTH_LIMIT {
        return false;
    }
    for rule in rules {
        match rule.rule_type {
            CssRuleType::Keyframes if keyframe_rule_name_matches(rule, animation_names) => {
                if keyframe_child_rule_snapshots_have_supported_animation_values(&rule.child_rules)
                {
                    return true;
                }
            }
            CssRuleType::Media | CssRuleType::Supports
                if keyframe_rule_snapshots_have_supported_animation_values(
                    &rule.child_rules,
                    animation_names,
                    depth + 1,
                ) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn keyframe_rule_name_matches(
    rule: &moli_css_parse::CssRuleSnapshot,
    animation_names: &[String],
) -> bool {
    moli_css_parse::parse_keyframes_rule_view_with_stylo(&rule.css_text).is_some_and(|view| {
        animation_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&view.name))
    })
}

fn keyframe_child_rule_snapshots_have_supported_animation_values(
    rules: &[moli_css_parse::CssRuleSnapshot],
) -> bool {
    keyframe_child_rule_snapshots_property_values(rules, "left").is_some_and(|(from, to)| {
        moli_css_parse::parse_px_length(&from, moli_css_parse::UnitlessLength::ZeroOnly).is_some()
            && moli_css_parse::parse_px_length(&to, moli_css_parse::UnitlessLength::ZeroOnly)
                .is_some()
    }) || keyframe_child_rule_snapshots_property_values(rules, "transform").is_some_and(
        |(from, to)| {
            transform_translate_x_px(&from).is_some() && transform_translate_x_px(&to).is_some()
        },
    ) || keyframe_child_rule_snapshots_property_values(rules, "color")
        .is_some_and(|(from, to)| from.eq_ignore_ascii_case(&to))
        || keyframe_child_rule_snapshots_property_values(rules, "background-color")
            .is_some_and(|(from, to)| from.eq_ignore_ascii_case(&to))
}

fn keyframe_child_rule_snapshots_property_values(
    rules: &[moli_css_parse::CssRuleSnapshot],
    property: &str,
) -> Option<(String, String)> {
    let mut from = None;
    let mut to = None;
    for snapshot in rules {
        if snapshot.rule_type != CssRuleType::Keyframe {
            continue;
        }
        let Some(selector_text) = snapshot.selector_text.as_deref() else {
            continue;
        };
        let Some(selector_text) =
            moli_css_parse::normalize_keyframe_selector_text_with_stylo(selector_text)
        else {
            continue;
        };
        let Some(style_text) = snapshot.declaration_text.as_deref() else {
            continue;
        };
        let value = parse_inline_css_text_with_base(style_text, None)
            .into_iter()
            .find(|entry| entry.name == property)
            .map(|entry| entry.value);
        let Some(value) = value else {
            continue;
        };
        if keyframe_selector_text_contains_normalized_endpoint(&selector_text, "0%") {
            from = Some(value.clone());
        }
        if keyframe_selector_text_contains_normalized_endpoint(&selector_text, "100%") {
            to = Some(value);
        }
    }
    Some((from?, to?))
}

fn keyframe_selector_text_contains_normalized_endpoint(
    selector_text: &str,
    endpoint: &str,
) -> bool {
    selector_text
        .split(',')
        .any(|selector| selector.trim() == endpoint)
}

fn transform_translate_x_px(value: &str) -> Option<f64> {
    let function = moli_css_parse::parse_transform_function_list(value)?
        .into_iter()
        .find(|function| matches!(function.name.as_str(), "translate" | "translatex"))?;
    let raw = function.arguments.first()?;
    moli_css_parse::parse_px_length(raw, moli_css_parse::UnitlessLength::ZeroOnly)
}

fn stylo_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
) -> Option<String> {
    ComputedStyleRead::new_with_context(runtime, handle, context).raw_primary_property(property)
}

fn stylo_computed_pseudo_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    pseudo_element: &str,
    property: &str,
    context: StyleComputationContext,
) -> Option<String> {
    let read = ComputedStyleRead::new_with_context(runtime, handle, context);
    let value = read.raw_pseudo_property(pseudo_element, property);
    (!value.is_empty()).then_some(value)
}

fn normalized_stylo_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
) -> Option<String> {
    let value = stylo_computed_style_value(runtime, handle, property, context)?;
    normalize_stylo_computed_style_value(runtime, handle, property, &value, context)
}

fn normalized_stylo_computed_style_value_with_inputs(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
    inputs: &FullStyleWorldSnapshot,
) -> Option<String> {
    let read_document = context.resolved_read_document(runtime, handle);
    let value = runtime.computed_style_property_value_from_stylo(
        handle,
        property,
        None,
        inputs,
        read_document,
        context.viewport,
    )?;
    normalize_stylo_computed_style_value(runtime, handle, property, &value, context)
}

fn normalize_stylo_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    context: StyleComputationContext,
) -> Option<String> {
    normalize_stylo_computed_style_value_with_resolution(
        runtime,
        handle,
        property,
        value,
        StyleResolutionContext::independent(context),
    )
}

fn normalize_stylo_computed_style_value_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if property == "text-size-adjust" {
        return computed_text_size_adjust_specified_value(runtime, handle, value)
            .or_else(|| Some(value.to_owned()));
    }
    if property == "touch-action" {
        return Some(normalize_touch_action_serialization(property, value));
    }
    if color_property_is_resolved_color(property) {
        return Some(resolve_computed_color_property_value(
            runtime, handle, property, value, resolution,
        ));
    }
    Some(value.to_owned())
}

fn normalize_touch_action_serialization(property: &str, value: &str) -> String {
    if property == "touch-action" && value == "pan-x pan-y pinch-zoom" {
        "manipulation".to_owned()
    } else {
        value.to_owned()
    }
}

fn normalized_stylo_computed_pseudo_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    pseudo_element: &str,
    property: &str,
    context: StyleComputationContext,
) -> Option<String> {
    let value =
        stylo_computed_pseudo_style_value(runtime, handle, pseudo_element, property, context)?;
    normalize_stylo_computed_style_value(runtime, handle, property, &value, context)
}

fn inline_style_entry_for_inline_style(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<StyleEntry> {
    // Non-computed CSSStyleDeclaration reads are specified/inline style only.
    // Stylesheet cascade is resolved by Stylo in the computed-style branch.
    inline_style_entry(runtime, handle, property)
}

fn inline_style_property_value_for_inline_style(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let property = canonical_style_property_name(property);
    if let Some(state) = runtime.element_inline_style_declaration_state(handle)
        && let Some(value) = inline_state_property_value_with_pdb(state, &property)
    {
        return Some(normalize_touch_action_serialization(&property, &value));
    }
    inline_style_entry_for_inline_style(runtime, handle, &property)
        .map(|entry| normalize_touch_action_serialization(&property, &entry.value))
}

pub(crate) fn raw_inline_style_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    inline_style_property_value_for_inline_style(runtime, handle, property)
}

pub(in crate::native_bridge::element::styles) fn normalize_style_value(
    name: &str,
    value: &str,
) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if !name.starts_with("--")
        && !moli_css_parse::css_declaration_value_has_valid_env_functions(trimmed)
    {
        return String::new();
    }
    if name == "content" {
        return normalize_content_specified_value(trimmed).unwrap_or_else(|| trimmed.to_owned());
    }
    if name == "font-family" {
        return normalize_cssom_font_family_value(trimmed).unwrap_or_else(|| trimmed.to_owned());
    }
    if name == "all" {
        return css_wide_keyword(trimmed).unwrap_or_default();
    }
    let serialized = if name.starts_with("--") {
        trimmed.to_owned()
    } else {
        moli_css_parse::normalize_cssom_component_value_serialization(trimmed)
            .unwrap_or_else(|| trimmed.to_owned())
    };
    let serialized = serialized.as_str();
    match name {
        "width" | "margin" | "min-width" | "max-width" | "padding" | "inset-inline-end"
        | "inset-inline-start" | "left" | "right" | "top" | "bottom" | "outline"
            if serialized == "0" =>
        {
            "0px".to_owned()
        }
        "accent-color" | "color" | "background-color" | "caret-color" | "outline-color"
            if simple_var_function_parts(serialized).is_some() =>
        {
            serialized.to_owned()
        }
        "accent-color" if serialized.eq_ignore_ascii_case("auto") => "auto".to_owned(),
        "border-color" => serialized.to_owned(),
        name if color_property_is_resolved_color(name)
            && !specified_color_value_is_valid(serialized) =>
        {
            String::new()
        }
        "width" | "height" | "min-width" | "max-width" if is_negative_length_like(serialized) => {
            String::new()
        }
        "font" => normalize_font_shorthand_specified_value(serialized)
            .unwrap_or_else(|| serialized.to_owned()),
        "flex" => normalize_cssom_flex_shorthand_value(serialized)
            .unwrap_or_else(|| serialized.to_owned()),
        "flex-basis" => {
            normalize_cssom_flex_basis_value(serialized).unwrap_or_else(|| serialized.to_owned())
        }
        "width" if serialized.starts_with("anchor-size(") => {
            normalize_anchor_size_function(serialized).unwrap_or_else(|| serialized.to_owned())
        }
        _ => serialized.to_owned(),
    }
}

pub(in crate::native_bridge::element::styles) fn normalize_style_value_with_base(
    name: &str,
    value: &str,
    _base_url: Option<&url::Url>,
) -> String {
    normalize_style_value(name, value)
}

fn normalize_font_shorthand_specified_value(value: &str) -> Option<String> {
    normalize_cssom_component_value_serialization_with_spaced_slash(value)
}

fn normalize_content_specified_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if cssom_value_is_attr_function(trimmed) {
        return Some(trimmed.to_owned());
    }
    if let Some(counter) = normalize_cssom_counter_function(trimmed) {
        return Some(counter);
    }
    moli_css_parse::normalize_cssom_component_value_serialization(trimmed)
}

fn normalize_cssom_font_family_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let mut input = ParserInput::new(trimmed);
    let mut input = Parser::new(&mut input);
    let token = input
        .next_including_whitespace_and_comments()
        .cloned()
        .ok()?;
    if let Token::QuotedString(name) = token {
        while let Ok(token) = input.next_including_whitespace_and_comments() {
            if !matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
                return moli_css_parse::normalize_cssom_component_value_serialization(trimmed);
            }
        }
        let name = name.to_string();
        if font_family_name_can_serialize_unquoted(&name) {
            return Some(name);
        }
        let mut quoted = String::new();
        serialize_string(&name, &mut quoted).ok()?;
        return Some(quoted);
    }
    moli_css_parse::normalize_cssom_component_value_serialization(trimmed)
}

fn font_family_name_can_serialize_unquoted(name: &str) -> bool {
    if name.is_empty()
        || name.trim() != name
        || name.chars().any(|ch| ch.is_whitespace() && ch != ' ')
    {
        return false;
    }
    let lowered = name.to_ascii_lowercase();
    if font_family_name_is_reserved(&lowered) {
        return false;
    }
    name.split(' ').all(font_family_ident_is_valid)
}

fn font_family_name_is_reserved(lowered: &str) -> bool {
    matches!(
        lowered,
        "serif"
            | "sans-serif"
            | "monospace"
            | "cursive"
            | "fantasy"
            | "system-ui"
            | "ui-serif"
            | "ui-sans-serif"
            | "ui-monospace"
            | "ui-rounded"
            | "math"
            | "fangsong"
            | "initial"
            | "inherit"
            | "unset"
            | "revert"
            | "revert-layer"
            | "revert-rule"
            | "default"
    )
}

fn font_family_ident_is_valid(ident: &str) -> bool {
    let mut chars = ident.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii_digit() {
        return false;
    }
    if first == '-' && chars.clone().next().is_some_and(|ch| ch.is_ascii_digit()) {
        return false;
    }
    font_family_ident_char_is_valid(first) && chars.all(font_family_ident_char_is_valid)
}

fn font_family_ident_char_is_valid(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || !ch.is_ascii()
}

fn normalize_anchor_size_function(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix("anchor-size(")?
        .strip_suffix(')')?
        .trim();
    let (head, fallback) = match split_top_level_once(inner, ',') {
        Some((head, fallback)) => (head.trim(), Some(fallback.trim())),
        None => (inner, None),
    };
    let tokens = head.split_whitespace().collect::<Vec<_>>();
    let canonical_head = match tokens.as_slice() {
        [single] => (*single).to_owned(),
        [first, second] if first.starts_with("--") || !second.starts_with("--") => {
            format!("{first} {second}")
        }
        [first, second] => format!("{second} {first}"),
        _ => head.to_owned(),
    };
    let fallback = fallback
        .map(|value| normalize_anchor_size_function(value).unwrap_or_else(|| value.to_owned()));
    Some(match fallback {
        Some(fallback) => format!("anchor-size({canonical_head}, {fallback})"),
        None => format!("anchor-size({canonical_head})"),
    })
}

fn split_top_level_once(input: &str, needle: char) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escape = false;
    for (index, ch) in input.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if quote.is_some() => {
                escape = true;
            }
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                }
            }
            '(' if quote.is_none() => depth += 1,
            ')' if quote.is_none() && depth > 0 => depth -= 1,
            _ if ch == needle && quote.is_none() && depth == 0 => {
                return Some((&input[..index], &input[index + ch.len_utf8()..]));
            }
            _ => {}
        }
    }
    None
}

fn cssom_value_is_attr_function(value: &str) -> bool {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    matches!(
        input.next_including_whitespace_and_comments(),
        Ok(Token::Function(name)) if name.eq_ignore_ascii_case("attr")
    )
}

fn normalize_cssom_counter_function(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let Ok(Token::Function(name)) = input.next_including_whitespace_and_comments() else {
        return None;
    };
    if !name.eq_ignore_ascii_case("counter") {
        return None;
    }
    let (counter_name, style): (String, Option<String>) = input
        .parse_nested_block(|input| {
            let counter_name = input.expect_ident_cloned()?.to_string();
            let style = if input.is_exhausted() {
                None
            } else {
                input.expect_comma()?;
                let style = input.expect_ident_cloned()?.to_string();
                input.expect_exhausted()?;
                Some(style)
            };
            Ok::<_, cssparser::ParseError<'_, ()>>((counter_name, style))
        })
        .ok()?;
    input.expect_exhausted().ok()?;
    if style.is_some_and(|style| style.eq_ignore_ascii_case("decimal")) {
        return Some(format!("counter({counter_name})"));
    }
    None
}

fn is_negative_length_like(value: &str) -> bool {
    let value = value.trim_start();
    value.starts_with('-')
        && value
            .chars()
            .nth(1)
            .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SimpleVarFunction {
    name: String,
    fallback: Option<String>,
}

fn simple_var_function_parts(value: &str) -> Option<SimpleVarFunction> {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    parser.expect_function_matching("var").ok()?;
    let parts = parser
        .parse_nested_block(|input| {
            let name = input.expect_ident_cloned()?;
            let name = name.to_string();
            let fallback = if input.is_exhausted() {
                None
            } else {
                input.expect_comma()?;
                Some(
                    simple_var_fallback_component_text(input)
                        .ok_or_else(|| input.new_custom_error(()))?,
                )
            };
            Ok::<_, cssparser::ParseError<'_, ()>>(SimpleVarFunction { name, fallback })
        })
        .ok()?;
    parser.expect_exhausted().ok()?;
    parts.name.starts_with("--").then_some(parts)
}

fn simple_var_fallback_component_text(input: &mut Parser<'_, '_>) -> Option<String> {
    let start = input.position();
    while input.next_including_whitespace_and_comments().is_ok() {}
    let value = input.slice_from(start).trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(super) fn inline_style_entry(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<StyleEntry> {
    let property = canonical_style_property_name(property);
    if property == "all" {
        return inline_all_style_entry(runtime, handle);
    }
    if let Some((shorthand, shorthand_index)) = box_shorthand_for_longhand(&property) {
        return inline_style_entries_for_property(runtime, handle, |entry| {
            if entry.name == property {
                Some(entry.clone())
            } else if entry.name == "all" && all_shorthand_applies_to(&property) {
                Some(StyleEntry {
                    name: "all".to_owned(),
                    value: entry.value.clone(),
                    priority: entry.priority,
                })
            } else if entry.name == shorthand {
                if moli_css_parse::css_value_may_contain_var_function(&entry.value) {
                    return Some(StyleEntry {
                        name: property.clone(),
                        value: String::new(),
                        priority: entry.priority,
                    });
                }
                box_shorthand_component(&entry.value, shorthand_index).map(|value| StyleEntry {
                    name: property.clone(),
                    value,
                    priority: entry.priority,
                })
            } else {
                None
            }
        });
    }
    inline_style_entries_for_property(runtime, handle, |entry| match entry.name.as_str() {
        name if name == property => Some(entry.clone()),
        "all" if all_shorthand_applies_to(&property) => Some(StyleEntry {
            name: "all".to_owned(),
            value: entry.value.clone(),
            priority: entry.priority,
        }),
        _ => None,
    })
}

fn inline_all_style_entry(runtime: &JsContextHost, handle: DomHandle) -> Option<StyleEntry> {
    let entries = style_entries(runtime, handle);
    let all_index = entries
        .iter()
        .enumerate()
        .rev()
        .find(|(_, entry)| entry.name == "all")
        .map(|(index, _)| index)?;
    let all = entries[all_index].clone();
    let overridden = entries.iter().skip(all_index + 1).any(|entry| {
        all_shorthand_applies_to(&entry.name)
            && (entry.priority != all.priority || entry.value != all.value)
    });
    (!overridden).then_some(all)
}

fn inline_style_entries_for_property(
    runtime: &JsContextHost,
    handle: DomHandle,
    candidate: impl Fn(&StyleEntry) -> Option<StyleEntry>,
) -> Option<StyleEntry> {
    let mut normal = None;
    let mut important = None;
    style_entries(runtime, handle)
        .into_iter()
        .filter_map(|entry| candidate(&entry))
        .for_each(|entry| {
            if entry.priority {
                important = Some(entry);
            } else {
                normal = Some(entry);
            }
        });
    important.or(normal)
}

fn box_shorthand_for_longhand(property: &str) -> Option<(&'static str, usize)> {
    Some(match property {
        "margin-top" => ("margin", 0),
        "margin-right" => ("margin", 1),
        "margin-bottom" => ("margin", 2),
        "margin-left" => ("margin", 3),
        "padding-top" => ("padding", 0),
        "padding-right" => ("padding", 1),
        "padding-bottom" => ("padding", 2),
        "padding-left" => ("padding", 3),
        "overscroll-behavior-x" => ("overscroll-behavior", 0),
        "overscroll-behavior-y" => ("overscroll-behavior", 1),
        _ => return None,
    })
}

fn box_shorthand_component(value: &str, shorthand_index: usize) -> Option<String> {
    let components = box_shorthand_value_components(value)?;
    match components.as_slice() {
        [single] => Some(single.clone()),
        [vertical, _horizontal] if shorthand_index == 0 || shorthand_index == 2 => {
            Some(vertical.clone())
        }
        [_vertical, horizontal] => Some(horizontal.clone()),
        [top, _right, _bottom] if shorthand_index == 0 => Some(top.clone()),
        [_top, right, _bottom] if shorthand_index == 1 || shorthand_index == 3 => {
            Some(right.clone())
        }
        [_top, _right, bottom] => Some(bottom.clone()),
        [top, _right, _bottom, _left] if shorthand_index == 0 => Some(top.clone()),
        [_top, right, _bottom, _left] if shorthand_index == 1 => Some(right.clone()),
        [_top, _right, bottom, _left] if shorthand_index == 2 => Some(bottom.clone()),
        [_top, _right, _bottom, left] => Some(left.clone()),
        _ => None,
    }
}

pub(super) fn computed_style_default_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> String {
    let property = property.to_ascii_lowercase();
    match property.as_str() {
        "display" => runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .map(|element| match element.local_name() {
                "table" => "table",
                "thead" => "table-header-group",
                "tbody" => "table-row-group",
                "tfoot" => "table-footer-group",
                "col" => "table-column",
                "colgroup" => "table-column-group",
                "tr" => "table-row",
                "td" | "th" => "table-cell",
                "caption" => "table-caption",
                "div" | "body" | "html" | "p" | "section" | "article" | "header" | "footer"
                | "main" | "nav" | "ul" | "ol" | "li" | "form" => "block",
                "slot" => "contents",
                _ => "inline",
            })
            .unwrap_or("inline")
            .to_owned(),
        "visibility" => "visible".to_owned(),
        "will-change" => "auto".to_owned(),
        "zoom" => "1".to_owned(),
        "direction" => "ltr".to_owned(),
        "unicode-bidi" => "normal".to_owned(),
        "container-name" => "none".to_owned(),
        "container-type" => "normal".to_owned(),
        "container" => "none".to_owned(),
        "bookmark-level" => "none".to_owned(),
        "bookmark-state" => "open".to_owned(),
        "color-scheme" => inherited_computed_style_value(runtime, handle, "color-scheme", "normal"),
        "forced-color-adjust" => {
            inherited_computed_style_value(runtime, handle, "forced-color-adjust", "auto")
        }
        "opacity" => "1".to_owned(),
        "pointer-events" => {
            inherited_computed_style_value(runtime, handle, "pointer-events", "auto")
        }
        "touch-action" => "auto".to_owned(),
        "accent-color" => "auto".to_owned(),
        "appearance" | "-webkit-appearance" => "none".to_owned(),
        "color" => inherited_computed_style_value(runtime, handle, "color", "rgb(0, 0, 0)"),
        "font-size" => inherited_computed_style_value(runtime, handle, "font-size", "16px"),
        "font-style" => "normal".to_owned(),
        "font-variant" => "normal".to_owned(),
        "font-variant-alternates"
        | "font-variant-caps"
        | "font-variant-east-asian"
        | "font-variant-emoji"
        | "font-variant-ligatures"
        | "font-variant-numeric"
        | "font-variant-position" => "normal".to_owned(),
        "font-weight" => "400".to_owned(),
        "line-height" => "normal".to_owned(),
        "link-parameters" => "none".to_owned(),
        "content" => "normal".to_owned(),
        "background-color" => "rgba(0, 0, 0, 0)".to_owned(),
        "background-attachment" => "scroll".to_owned(),
        "background-blend-mode" | "mix-blend-mode" => "normal".to_owned(),
        "background-image" => "none".to_owned(),
        "background-position-x"
        | "background-position-y"
        | "mask-position-x"
        | "mask-position-y" => "0%".to_owned(),
        "background" => "none".to_owned(),
        "alignment-baseline" => "baseline".to_owned(),
        "baseline-source" => "auto".to_owned(),
        "border-collapse" => "separate".to_owned(),
        "border-image" => "none".to_owned(),
        "caption-side" => "top".to_owned(),
        "clear" => "none".to_owned(),
        "clip" => "auto".to_owned(),
        "empty-cells" => "show".to_owned(),
        "isolation" => "auto".to_owned(),
        "mask" => "none".to_owned(),
        "-webkit-text-stroke" => {
            let color = inherited_computed_style_value(runtime, handle, "color", "rgb(0, 0, 0)");
            format!("0px {color}")
        }
        "-webkit-text-stroke-color" => {
            inherited_computed_style_value(runtime, handle, "color", "rgb(0, 0, 0)")
        }
        "-webkit-text-stroke-width" => "0px".to_owned(),
        "text-decoration-fill" | "text-decoration-stroke" => "match-text".to_owned(),
        "text-decoration-inset" => "0px".to_owned(),
        "text-decoration-line" => "none".to_owned(),
        "text-decoration-skip-ink" => "auto".to_owned(),
        "text-decoration-skip-spaces" => "start end".to_owned(),
        "text-decoration-style" => "solid".to_owned(),
        "text-emphasis-style" => "none".to_owned(),
        "text-emphasis-color" => {
            inherited_computed_style_value(runtime, handle, "color", "rgb(0, 0, 0)")
        }
        "text-emphasis-position" => "auto".to_owned(),
        "text-shadow" => "none".to_owned(),
        "text-transform" => "none".to_owned(),
        "text-underline-position" => "auto".to_owned(),
        "text-decoration-thickness" | "text-underline-offset" => "auto".to_owned(),
        "border" | "border-bottom" | "border-left" | "border-right" | "border-top" => {
            "medium none currentcolor".to_owned()
        }
        "border-radius" => "0px".to_owned(),
        "-webkit-border-radius" => "0px".to_owned(),
        "font" => "16px sans-serif".to_owned(),
        "flex" | "-webkit-flex" => "0 1 auto".to_owned(),
        "flex-flow" | "-webkit-flex-flow" => "row nowrap".to_owned(),
        "gap" | "row-gap" | "column-gap" | "place-content" => "normal".to_owned(),
        "grid-column" | "grid-column-start" | "grid-column-end" => "auto".to_owned(),
        "list-style" => "disc outside none".to_owned(),
        "list-style-image" => "none".to_owned(),
        "list-style-position" => "outside".to_owned(),
        "list-style-type" => "disc".to_owned(),
        "outline" => "medium none currentcolor".to_owned(),
        "outline-style" => "none".to_owned(),
        "overscroll-behavior"
        | "overscroll-behavior-block"
        | "overscroll-behavior-inline"
        | "overscroll-behavior-x"
        | "overscroll-behavior-y" => "auto".to_owned(),
        "print-color-adjust" => {
            inherited_computed_style_value(runtime, handle, "print-color-adjust", "economy")
        }
        "quotes" => inherited_computed_style_value(runtime, handle, "quotes", "auto"),
        "scrollbar-color" => {
            inherited_computed_style_value(runtime, handle, "scrollbar-color", "auto")
        }
        "scrollbar-width" => "auto".to_owned(),
        "text-size-adjust" => {
            inherited_computed_style_value(runtime, handle, "text-size-adjust", "auto")
        }
        "left" | "right" | "top" | "bottom" => "auto".to_owned(),
        "block-size" => "auto".to_owned(),
        "orphans" | "widows" => "2".to_owned(),
        "page-break-after" | "page-break-before" | "page-break-inside" => "auto".to_owned(),
        "table-layout" => "auto".to_owned(),
        "transition" | "-webkit-transition" => "all".to_owned(),
        "transition-behavior" => "normal".to_owned(),
        "transition-delay" | "transition-duration" => "0s".to_owned(),
        "transition-property" => "all".to_owned(),
        "transition-timing-function" => "ease".to_owned(),
        "animation" | "-webkit-animation" => "none".to_owned(),
        "rotate" | "scale" | "transform" | "-webkit-transform" => "none".to_owned(),
        "-webkit-mask"
        | "-webkit-mask-box-image"
        | "-webkit-mask-box-image-source"
        | "-webkit-mask-image" => "none".to_owned(),
        "-webkit-mask-box-image-outset" | "-webkit-mask-box-image-slice" => "0".to_owned(),
        "-webkit-mask-box-image-repeat" => "stretch".to_owned(),
        "-webkit-mask-box-image-width" | "-webkit-mask-size" => "auto".to_owned(),
        "-webkit-mask-clip" | "-webkit-mask-origin" => "border-box".to_owned(),
        "-webkit-mask-composite" => "source-over".to_owned(),
        "-webkit-mask-position" => "0% 0%".to_owned(),
        "-webkit-mask-repeat" => "repeat".to_owned(),
        "-webkit-perspective" => "none".to_owned(),
        "user-select" | "-webkit-user-select" => "auto".to_owned(),
        "white-space" => "normal".to_owned(),
        _ => String::new(),
    }
}

fn inherited_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    initial: &str,
) -> String {
    inherited_style_parent(runtime, handle)
        .map(|parent| style_property_value(runtime, parent, StyleMode::Computed, property))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| initial.to_owned())
}

fn inherited_style_parent(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    flat_tree_element_parent(runtime, handle)
}

pub(crate) fn style_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
) -> String {
    if mode == StyleMode::Computed {
        return ComputedStyleRead::new(runtime, handle).property(property);
    }
    style_property_value_with_viewport_width(runtime, handle, mode, property, None)
}

fn normalize_specified_cssom_value(property: &str, value: String) -> String {
    let value = if property == "font-family" {
        normalize_cssom_font_family_value(&value).unwrap_or(value)
    } else {
        value
    };
    normalize_touch_action_serialization(property, &value)
}

pub(in crate::native_bridge::element::styles) fn style_property_value_with_viewport_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
    viewport_width: Option<f64>,
) -> String {
    let canonicalize = if mode == StyleMode::Computed {
        canonical_computed_cssom_query_property_name
    } else {
        canonical_specified_cssom_query_property_name
    };
    let Some(property) = canonicalize(property) else {
        return String::new();
    };
    if mode == StyleMode::Computed {
        let viewport = StyleViewport {
            width: viewport_width.or_else(|| runtime.style_viewport().width),
            ..runtime.style_viewport()
        };
        return computed_style_property_value_with_context(
            runtime,
            handle,
            &property,
            StyleComputationContext::new(viewport),
        );
    }
    if let Some(state) = runtime.element_inline_style_declaration_state(handle)
        && let Some(value) = inline_state_property_value_with_pdb(state, &property)
    {
        return normalize_specified_cssom_value(&property, value);
    }
    let entries = style_entries(runtime, handle);
    if let Some(value) = style_entries_property_value_with_pdb(&entries, &property) {
        return normalize_specified_cssom_value(&property, value);
    }
    if property == "overflow" {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        let overflow_x = inline_style_entry_for_inline_style(runtime, handle, "overflow-x")
            .map(|entry| entry.value);
        let overflow_y = inline_style_entry_for_inline_style(runtime, handle, "overflow-y")
            .map(|entry| entry.value);
        return match (overflow_x, overflow_y) {
            (Some(left), Some(right)) if left == right => left,
            (Some(left), Some(right)) => format!("{left} {right}"),
            _ => String::new(),
        };
    }
    if property == "overflow-x" || property == "overflow-y" {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "overflow") {
            let tokens = entry.value.split_whitespace().collect::<Vec<_>>();
            return match tokens.as_slice() {
                [single] => (*single).to_owned(),
                [left, right] if property == "overflow-x" => (*left).to_owned(),
                [left, right] if property == "overflow-y" => (*right).to_owned(),
                _ => String::new(),
            };
        }
        return String::new();
    }
    if property == "animation" {
        return inline_animation_shorthand_value(runtime, handle);
    }
    if property == "animation-range" {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property)
            && entry.name == property
        {
            return entry.value;
        }
        return inline_animation_range_shorthand_value(runtime, handle);
    }
    if property == "transition" {
        return inline_transition_shorthand_value(runtime, handle);
    }
    if property == "text-decoration" {
        return inline_text_decoration_shorthand_value(runtime, handle);
    }
    if property == "text-emphasis" {
        return inline_text_emphasis_shorthand_value(runtime, handle);
    }
    if property == "font-variant" {
        return inline_font_variant_shorthand_value(runtime, handle);
    }
    if property == "list-style" {
        return inline_list_style_shorthand_value(runtime, handle);
    }
    if property == "outline" {
        return inline_outline_shorthand_value(runtime, handle);
    }
    if property == "border" {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        return inline_border_shorthand_value(runtime, handle);
    }
    if let Some(side) = border_side_shorthand_property(&property) {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "border") {
            return entry.value;
        }
        let value = inline_border_side_shorthand_value(runtime, handle, side);
        if !value.is_empty() {
            return value;
        }
        return inline_border_side_component_shorthand_value(runtime, handle, side);
    }
    if border_color_property(&property) {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if property == "border-color"
            && let Some(value) =
                inline_shorthand_value_from_longhands(runtime, handle, border_color_longhands())
        {
            return value;
        }
        if let Some(index) = border_color_property_index(&property)
            && let Some(value) =
                border_component_from_component_shorthand(runtime, handle, "border-color", index)
        {
            return value;
        }
        if let Some(color) = border_color_from_shorthand(runtime, handle) {
            return color;
        }
        return String::new();
    }
    if border_style_property(&property) {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if property == "border-style"
            && let Some(value) =
                inline_shorthand_value_from_longhands(runtime, handle, border_style_longhands())
        {
            return value;
        }
        if let Some(index) = border_style_property_index(&property)
            && let Some(value) =
                border_component_from_component_shorthand(runtime, handle, "border-style", index)
        {
            return value;
        }
        if let Some(style) = border_style_from_shorthand(runtime, handle) {
            return style;
        }
        return String::new();
    }
    if border_width_property_index(&property).is_some() {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if property == "border-width"
            && let Some(value) =
                inline_shorthand_value_from_longhands(runtime, handle, border_width_longhands())
        {
            return value;
        }
        if let Some(index) = border_width_property_index(&property).filter(|index| *index < 4)
            && let Some(value) =
                border_component_from_component_shorthand(runtime, handle, "border-width", index)
        {
            return value;
        }
        if let Some(value) = border_width_from_shorthand(runtime, handle, &property) {
            return value;
        }
        return String::new();
    }
    if let Some(longhands) = shorthand_longhands(&property) {
        if let Some((index, entry)) =
            inline_exact_style_entry_with_index(runtime, handle, &property)
        {
            if moli_css_parse::css_value_may_contain_var_function(&entry.value)
                && exact_shorthand_has_later_overriding_longhand(
                    runtime, handle, index, &entry, longhands,
                )
            {
                return String::new();
            }
            return compress_box_shorthand_value(&entry.value);
        }
        let mut values = Vec::with_capacity(longhands.len());
        let mut priority = None;
        for longhand in longhands {
            let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, longhand) else {
                return String::new();
            };
            if priority.is_some_and(|current| current != entry.priority) {
                return String::new();
            }
            priority = Some(entry.priority);
            values.push(entry.value);
        }
        if values.iter().any(|value| value.is_empty()) {
            return String::new();
        }
        if values.iter().any(|value| css_wide_keyword(value).is_some())
            && !values.windows(2).all(|pair| pair[0] == pair[1])
        {
            return String::new();
        }
        return compress_box_components(&values).unwrap_or_default();
    }
    if let Some((shorthand, index)) = list_style_longhand_index(&property)
        && let Some(value) = fixed_shorthand_component(runtime, handle, shorthand, index)
    {
        return value;
    }
    if let Some((shorthand, index)) = outline_longhand_index(&property)
        && let Some(value) = fixed_shorthand_component(runtime, handle, shorthand, index)
    {
        return value;
    }
    if let Some(index) = font_variant_longhand_index(&property) {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "font-variant") {
            return font_variant_longhand_value_from_shorthand(&entry.value, index)
                .unwrap_or_default();
        }
        return String::new();
    }
    if property == "background-color" {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "background")
            && let Some(color) = background_shorthand_color(&entry.value)
        {
            return color;
        }
        return String::new();
    }
    inline_style_entry_for_inline_style(runtime, handle, &property)
        .or_else(|| inset_shorthand_style_entry(runtime, handle, &property))
        .or_else(|| logical_inset_style_entry(runtime, handle, &property))
        .map(|entry| entry.value)
        .unwrap_or_default()
}

fn inline_exact_style_entry_with_index(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<(usize, StyleEntry)> {
    let mut normal = None;
    let mut important = None;
    for (index, entry) in style_entries(runtime, handle).into_iter().enumerate() {
        if entry.name != property {
            continue;
        }
        if entry.priority {
            important = Some((index, entry));
        } else {
            normal = Some((index, entry));
        }
    }
    important.or(normal)
}

fn exact_shorthand_has_later_overriding_longhand(
    runtime: &JsContextHost,
    handle: DomHandle,
    shorthand_index: usize,
    shorthand: &StyleEntry,
    longhands: &[&str],
) -> bool {
    style_entries(runtime, handle)
        .into_iter()
        .enumerate()
        .skip(shorthand_index + 1)
        .any(|(_, entry)| {
            longhands.contains(&entry.name.as_str()) && (!shorthand.priority || entry.priority)
        })
}

fn inline_shorthand_value_from_longhands(
    runtime: &JsContextHost,
    handle: DomHandle,
    longhands: &[&str],
) -> Option<String> {
    let mut values = Vec::with_capacity(longhands.len());
    let mut priority = None;
    for longhand in longhands {
        let entry = inline_style_entry_for_inline_style(runtime, handle, longhand)?;
        if priority.is_some_and(|current| current != entry.priority) {
            return None;
        }
        priority = Some(entry.priority);
        values.push(entry.value);
    }
    if values.iter().any(|value| value.is_empty()) {
        return None;
    }
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let first = values.first()?;
        return values
            .iter()
            .all(|value| value == first)
            .then(|| first.clone());
    }
    compress_box_components(&values)
}

fn inline_text_emphasis_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let Some(style) = inline_style_entry_for_inline_style(runtime, handle, "text-emphasis-style")
    else {
        return String::new();
    };
    let Some(color) = inline_style_entry_for_inline_style(runtime, handle, "text-emphasis-color")
    else {
        return String::new();
    };
    if style.priority != color.priority {
        return String::new();
    }
    if style.value == color.value && css_wide_keyword(&style.value).is_some() {
        return style.value;
    }
    if css_wide_keyword(&style.value).is_some() || css_wide_keyword(&color.value).is_some() {
        return String::new();
    }
    format!("{} {}", style.value, color.value)
}

fn inline_font_variant_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let exact = inline_exact_style_entry_with_index(runtime, handle, "font-variant");
    let mut values = exact
        .as_ref()
        .and_then(|(_, entry)| font_variant_longhand_values_from_shorthand(&entry.value))
        .unwrap_or_else(|| vec!["normal".to_owned(); font_variant_longhands().len()]);
    let mut has_font_variant_state = exact.is_some();

    for (index, longhand) in font_variant_longhands().iter().enumerate() {
        let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, longhand) else {
            continue;
        };
        values[index] = entry.value;
        has_font_variant_state = true;
    }

    if !has_font_variant_state {
        return String::new();
    }
    serialize_font_variant_shorthand_values(&values).unwrap_or_default()
}

fn font_variant_longhand_index(property: &str) -> Option<usize> {
    font_variant_longhands()
        .iter()
        .position(|longhand| *longhand == property)
}

fn font_variant_longhand_value_from_shorthand(value: &str, index: usize) -> Option<String> {
    font_variant_longhand_values_from_shorthand(value).and_then(|values| values.get(index).cloned())
}

fn font_variant_longhand_values_from_shorthand(value: &str) -> Option<Vec<String>> {
    let value = value.trim();
    if let Some(keyword) = css_wide_keyword(value) {
        return Some(vec![keyword; font_variant_longhands().len()]);
    }
    let mut values = vec!["normal".to_owned(); font_variant_longhands().len()];
    match value.to_ascii_lowercase().as_str() {
        "normal" => {}
        "none" => values[0] = "none".to_owned(),
        "common-ligatures discretionary-ligatures" => values[0] = value.to_owned(),
        "small-caps" => values[1] = value.to_owned(),
        "historical-forms" => values[2] = value.to_owned(),
        "oldstyle-nums stacked-fractions" => values[3] = value.to_owned(),
        "ruby" => values[4] = value.to_owned(),
        "sub" | "super" => values[5] = value.to_owned(),
        "emoji" | "text" | "unicode" => values[6] = value.to_owned(),
        _ => return None,
    }
    Some(values)
}

fn serialize_font_variant_shorthand_values(values: &[String]) -> Option<String> {
    if values.len() != font_variant_longhands().len() {
        return None;
    }
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let first = values.first()?;
        return values
            .iter()
            .all(|value| value == first)
            .then(|| first.clone());
    }
    let non_normal = values
        .iter()
        .filter(|value| !value.eq_ignore_ascii_case("normal"))
        .collect::<Vec<_>>();
    if non_normal.is_empty() {
        return Some("normal".to_owned());
    }
    if values[0].eq_ignore_ascii_case("none") {
        return (non_normal.len() == 1).then(|| "none".to_owned());
    }
    Some(
        non_normal
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn inline_list_style_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "list-style") {
        return entry.value;
    }
    inline_fixed_shorthand_value(
        runtime,
        handle,
        &["list-style-position", "list-style-type", "list-style-image"],
        serialize_list_style_shorthand_components,
    )
}

fn inline_outline_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "outline") {
        return entry.value;
    }
    inline_fixed_shorthand_value(
        runtime,
        handle,
        &["outline-color", "outline-style", "outline-width"],
        |values| Some(format!("{} {} {}", values[0], values[1], values[2])),
    )
}

fn inline_fixed_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    longhands: &[&str],
    serialize: impl Fn(&[String]) -> Option<String>,
) -> String {
    let mut values = Vec::with_capacity(longhands.len());
    let mut priority = None;
    for longhand in longhands {
        let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, longhand) else {
            return String::new();
        };
        if priority.is_some_and(|current| current != entry.priority) {
            return String::new();
        }
        priority = Some(entry.priority);
        values.push(entry.value);
    }
    if values.iter().any(|value| value.is_empty()) {
        return String::new();
    }
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let Some(first) = values.first() else {
            return String::new();
        };
        if values.iter().all(|value| value == first) {
            return first.clone();
        }
        return String::new();
    }
    serialize(&values).unwrap_or_default()
}

fn serialize_list_style_shorthand_components(values: &[String]) -> Option<String> {
    let [position, list_type, image] = values else {
        return None;
    };
    let mut parts = Vec::new();
    if !position.eq_ignore_ascii_case("outside") {
        parts.push(position.as_str());
    }
    if !list_type.eq_ignore_ascii_case("disc") {
        parts.push(list_type.as_str());
    }
    if !image.eq_ignore_ascii_case("none") {
        parts.push(image.as_str());
    }
    Some(if parts.is_empty() {
        "outside disc".to_owned()
    } else {
        parts.join(" ")
    })
}

fn fixed_shorthand_component(
    runtime: &JsContextHost,
    handle: DomHandle,
    shorthand: &str,
    index: usize,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, shorthand)?;
    if css_wide_keyword(&entry.value).is_some() {
        return Some(entry.value);
    }
    let values = match shorthand {
        "list-style" => Vec::from(list_style_components_for_value(&entry.value)?),
        "outline" => outline_components(&entry.value)?,
        _ => return None,
    };
    values.get(index).cloned()
}

fn list_style_longhand_index(property: &str) -> Option<(&'static str, usize)> {
    match property {
        "list-style-position" => Some(("list-style", 0)),
        "list-style-type" => Some(("list-style", 1)),
        "list-style-image" => Some(("list-style", 2)),
        _ => None,
    }
}

fn outline_longhand_index(property: &str) -> Option<(&'static str, usize)> {
    match property {
        "outline-color" => Some(("outline", 0)),
        "outline-style" => Some(("outline", 1)),
        "outline-width" => Some(("outline", 2)),
        _ => None,
    }
}

fn list_style_components_for_value(value: &str) -> Option<[String; 3]> {
    let mut position = "outside".to_owned();
    let mut list_type = "disc".to_owned();
    let mut image = "none".to_owned();
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    for token in &tokens {
        if matches!(*token, "inside" | "outside") {
            position = (*token).to_owned();
        } else if *token == "none" {
            if tokens.len() == 1 {
                list_type = "none".to_owned();
            } else {
                image = "none".to_owned();
            }
        } else if token.starts_with("url(") {
            image = (*token).to_owned();
        } else {
            list_type = (*token).to_owned();
        }
    }
    Some([position, list_type, image])
}

fn outline_components(value: &str) -> Option<Vec<String>> {
    Some(vec![
        border_shorthand_color(value).unwrap_or_else(|| "currentcolor".to_owned()),
        border_shorthand_style(value).unwrap_or_else(|| "none".to_owned()),
        border_shorthand_width(value).unwrap_or_else(|| "medium".to_owned()),
    ])
}

fn inline_border_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let top = inline_border_side_shorthand_value(runtime, handle, BorderSide::Top);
    if top.is_empty() {
        return String::new();
    }
    let sides = [
        inline_border_side_shorthand_value(runtime, handle, BorderSide::Right),
        inline_border_side_shorthand_value(runtime, handle, BorderSide::Bottom),
        inline_border_side_shorthand_value(runtime, handle, BorderSide::Left),
    ];
    if sides.iter().all(|side| side == &top) {
        top
    } else {
        String::new()
    }
}

fn inline_border_side_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    side: BorderSide,
) -> String {
    let Some(width) = inline_style_entry_for_inline_style(runtime, handle, side.width_property())
    else {
        return String::new();
    };
    let Some(style) = inline_style_entry_for_inline_style(runtime, handle, side.style_property())
    else {
        return String::new();
    };
    let Some(color) = inline_style_entry_for_inline_style(runtime, handle, side.color_property())
    else {
        return String::new();
    };
    serialize_border_side_shorthand(&width, &style, &color).unwrap_or_default()
}

fn inline_border_side_component_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    side: BorderSide,
) -> String {
    let width = inline_style_entry_for_inline_style(runtime, handle, side.width_property())
        .or_else(|| {
            border_component_style_entry_from_component_shorthand(
                runtime,
                handle,
                "border-width",
                side.width_property(),
                side.component_index(),
            )
        });
    let style = inline_style_entry_for_inline_style(runtime, handle, side.style_property())
        .or_else(|| {
            border_component_style_entry_from_component_shorthand(
                runtime,
                handle,
                "border-style",
                side.style_property(),
                side.component_index(),
            )
        });
    let color = inline_style_entry_for_inline_style(runtime, handle, side.color_property())
        .or_else(|| {
            border_component_style_entry_from_component_shorthand(
                runtime,
                handle,
                "border-color",
                side.color_property(),
                side.component_index(),
            )
        });
    let (Some(width), Some(style), Some(color)) = (width, style, color) else {
        return String::new();
    };
    serialize_border_side_shorthand(&width, &style, &color).unwrap_or_default()
}

fn computed_border_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    let top = computed_border_side_shorthand_value(runtime, handle, BorderSide::Top, context);
    if top.is_empty() {
        return computed_style_default_value(runtime, handle, "border");
    }
    let sides = [
        computed_border_side_shorthand_value(runtime, handle, BorderSide::Right, context),
        computed_border_side_shorthand_value(runtime, handle, BorderSide::Bottom, context),
        computed_border_side_shorthand_value(runtime, handle, BorderSide::Left, context),
    ];
    if sides.iter().all(|side| side == &top) {
        top
    } else {
        String::new()
    }
}

fn computed_border_side_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    side: BorderSide,
    context: StyleComputationContext,
) -> String {
    let width =
        computed_style_property_value_with_context(runtime, handle, side.width_property(), context);
    let style =
        computed_style_property_value_with_context(runtime, handle, side.style_property(), context);
    let color =
        computed_style_property_value_with_context(runtime, handle, side.color_property(), context);
    serialize_border_side_components(&width, &style, &color).unwrap_or_default()
}

fn serialize_border_side_shorthand(
    width: &StyleEntry,
    style: &StyleEntry,
    color: &StyleEntry,
) -> Option<String> {
    if width.priority != style.priority || width.priority != color.priority {
        return None;
    }
    serialize_border_side_components(&width.value, &style.value, &color.value)
}

fn serialize_border_side_components(width: &str, style: &str, color: &str) -> Option<String> {
    let css_wide_keywords = [width, style, color]
        .iter()
        .map(|value| css_wide_keyword(value))
        .collect::<Option<Vec<_>>>();
    if [width, style, color]
        .iter()
        .any(|value| css_wide_keyword(value).is_some())
    {
        let keywords = css_wide_keywords?;
        let first = keywords.first()?.clone();
        return keywords
            .iter()
            .all(|keyword| keyword == &first)
            .then_some(first);
    }
    Some(format!("{width} {style} {color}"))
}

#[derive(Clone, Copy)]
enum BorderSide {
    Top,
    Right,
    Bottom,
    Left,
}

impl BorderSide {
    fn component_index(self) -> usize {
        match self {
            Self::Top => 0,
            Self::Right => 1,
            Self::Bottom => 2,
            Self::Left => 3,
        }
    }

    fn width_property(self) -> &'static str {
        match self {
            Self::Top => "border-top-width",
            Self::Right => "border-right-width",
            Self::Bottom => "border-bottom-width",
            Self::Left => "border-left-width",
        }
    }

    fn style_property(self) -> &'static str {
        match self {
            Self::Top => "border-top-style",
            Self::Right => "border-right-style",
            Self::Bottom => "border-bottom-style",
            Self::Left => "border-left-style",
        }
    }

    fn color_property(self) -> &'static str {
        match self {
            Self::Top => "border-top-color",
            Self::Right => "border-right-color",
            Self::Bottom => "border-bottom-color",
            Self::Left => "border-left-color",
        }
    }
}

fn border_side_shorthand_property(property: &str) -> Option<BorderSide> {
    Some(match property {
        "border-top" => BorderSide::Top,
        "border-right" => BorderSide::Right,
        "border-bottom" => BorderSide::Bottom,
        "border-left" => BorderSide::Left,
        _ => return None,
    })
}

pub(in crate::native_bridge::element::styles) fn style_property_value_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
    context: StyleComputationContext,
) -> String {
    if mode == StyleMode::Computed {
        let Some(property) = canonical_computed_cssom_query_property_name(property) else {
            return String::new();
        };
        return computed_style_property_value_with_context(runtime, handle, &property, context);
    }
    style_property_value_with_viewport_width(
        runtime,
        handle,
        mode,
        property,
        context.viewport_width(),
    )
}

fn computed_style_property_value_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
) -> String {
    ComputedStyleRead::new_with_context(runtime, handle, context).property(property)
}

fn computed_style_property_value_after_style_update(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
    prepared_inputs: Option<&FullStyleWorldSnapshot>,
    observation: Option<&dyn RetainedStyleObservation>,
    prepared_style: Option<&StyloComputedStyleSnapshot>,
) -> String {
    if !computed_style_applies(runtime, handle) {
        return String::new();
    }
    if property == "display" && element_has_hidden_attribute(runtime, handle) {
        return "none".to_owned();
    }
    let inputs = prepared_inputs;
    let resolution = if let Some(observation) = observation {
        StyleResolutionContext::observed(context, inputs, observation, handle, prepared_style)
    } else if let Some(style) = prepared_style {
        if let Some(inputs) = inputs {
            StyleResolutionContext::retained(context, inputs, handle, style)
        } else {
            StyleResolutionContext::retained_without_inputs(context, handle, style)
        }
    } else if let Some(inputs) = inputs {
        StyleResolutionContext::prepared(context, inputs)
    } else {
        StyleResolutionContext::independent(context)
    };
    if property == "direction" {
        return computed_direction_with_resolution(runtime, handle, resolution);
    }
    if matches!(property, "left" | "right" | "top" | "bottom")
        && let Some(midpoint) =
            active_css_animation_midpoint_px_with_resolution(runtime, handle, property, resolution)
    {
        return format!("{midpoint}px");
    }
    if property == "transform"
        && let Some(transform) =
            active_css_animation_transform_value_with_resolution(runtime, handle, resolution)
    {
        return transform;
    }
    if color_property_is_resolved_color(property)
        && let Some(value) =
            active_css_animation_static_value(runtime, handle, property, resolution)
    {
        return resolve_computed_color_property_value(
            runtime, handle, property, &value, resolution,
        );
    }
    if property == "animation" {
        return computed_animation_shorthand_value(runtime, handle, context);
    }
    if property == "animation-range" {
        return computed_animation_range_shorthand_value(runtime, handle, context);
    }
    if property == "transition" {
        return computed_transition_shorthand_value(runtime, handle, context);
    }
    if property == "flex-flow" {
        let direction =
            computed_style_property_value_with_context(runtime, handle, "flex-direction", context);
        let wrap =
            computed_style_property_value_with_context(runtime, handle, "flex-wrap", context);
        return format!("{direction} {wrap}");
    }
    if property == "text-decoration" {
        return computed_text_decoration_shorthand_value(runtime, handle, context);
    }
    if property == "text-emphasis" {
        return computed_text_emphasis_shorthand_value(runtime, handle, context);
    }
    if property == "font-variant" {
        return computed_font_variant_shorthand_value(runtime, handle, resolution);
    }
    if property == "border" {
        return computed_border_shorthand_value(runtime, handle, context);
    }
    if let Some(side) = border_side_shorthand_property(property) {
        return computed_border_side_shorthand_value(runtime, handle, side, context);
    }
    if property == "-webkit-text-stroke" {
        return computed_webkit_text_stroke_shorthand_value(runtime, handle, context);
    }
    if matches!(property, "text-decoration-fill" | "text-decoration-stroke")
        && let Some(value) = computed_text_decoration_paint_value(runtime, handle, property)
    {
        return value;
    }
    if matches!(
        property,
        "animation-timing-function" | "transition-timing-function"
    ) && let Some(value) = computed_timing_function_list_value(runtime, handle, property)
    {
        return value;
    }
    if matches!(property, "animation-range-start" | "animation-range-end")
        && let Some(value) = computed_animation_range_endpoint_value(runtime, handle, property)
    {
        return value;
    }
    if property == "text-decoration-line"
        && let Some(value) = computed_inline_text_decoration_line_value(runtime, handle)
    {
        return value;
    }
    if matches!(property, "background-position" | "mask-position")
        && let Some(value) = computed_axis_position_shorthand_value(
            runtime,
            handle,
            property.strip_suffix("position").unwrap_or_default(),
            context,
        )
    {
        return value;
    }
    if css_numeric_computed_property_rule(property).is_some()
        && let Some(value) = computed_css_numeric_property_value(runtime, handle, property)
    {
        return value;
    }
    if property == "zoom"
        && let Some(value) = computed_inline_zoom_value(runtime, handle, resolution)
    {
        return value;
    }

    let raw_value = if let Some(style) = prepared_style {
        style.resolved_property_value(property).and_then(|value| {
            normalize_stylo_computed_style_value_with_resolution(
                runtime, handle, property, &value, resolution,
            )
        })
    } else if let Some(inputs) = inputs {
        normalized_stylo_computed_style_value_with_inputs(
            runtime, handle, property, context, inputs,
        )
    } else {
        None
    }
    .or_else(|| computed_style_property_value_from_moli(runtime, handle, property));
    let value = raw_value
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| computed_style_default_value(runtime, handle, property));
    let value = shadow_tree_inherited_value_for_initial_stylo_value(
        runtime, handle, property, &value, resolution,
    )
    .unwrap_or(value);
    if let Some(value) = active_registered_length_custom_property_animation_value(
        runtime, handle, property, &value, resolution,
    ) {
        return value;
    }
    let value = inputs
        .map(|inputs| {
            resolve_computed_custom_function_calls(
                runtime, handle, property, &value, inputs, context,
            )
        })
        .unwrap_or(value);
    resolve_moli_computed_style_value(runtime, handle, property, &value, context, resolution)
}

fn computed_property_requires_stylesheet_sources(
    property: &str,
    style: Option<&StyloComputedStyleSnapshot>,
) -> bool {
    property.starts_with("--")
        && style
            .and_then(|style| style.property_value(property))
            .is_some_and(|value| !dashed_no_arg_function_calls(&value).is_empty())
}

fn computed_style_property_value_with_prepared_inputs(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    inputs: &FullStyleWorldSnapshot,
    context: StyleComputationContext,
    prepared_style: Option<&StyloComputedStyleSnapshot>,
) -> String {
    computed_style_property_value_after_style_update(
        runtime,
        handle,
        property,
        context,
        Some(inputs),
        None,
        prepared_style,
    )
}

fn computed_transform_matrix_value(value: &str) -> Option<String> {
    if value.trim().eq_ignore_ascii_case("none") {
        return None;
    }
    moli_geometry::parse_dom_matrix_value(value)?.css_text()
}

fn computed_style_property_value_from_moli(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    match property {
        "animation-range-start" | "animation-range-end" => {
            computed_animation_range_endpoint_value(runtime, handle, property)
        }
        "animation-delay"
        | "animation-duration"
        | "animation-iteration-count"
        | "transition-delay"
        | "transition-duration" => computed_css_numeric_property_value(runtime, handle, property),
        "animation-timing-function" | "transition-timing-function" => {
            computed_timing_function_list_value(runtime, handle, property)
        }
        "bookmark-level" => Some(computed_non_inherited_css_keyword_property_value(
            runtime, handle, property, "none",
        )),
        "bookmark-state" => Some(computed_non_inherited_css_keyword_property_value(
            runtime, handle, property, "open",
        )),
        "color-scheme" => Some(computed_inherited_css_keyword_property_value(
            runtime, handle, property, "normal",
        )),
        "forced-color-adjust" => Some(computed_inherited_css_keyword_property_value(
            runtime, handle, property, "auto",
        )),
        "print-color-adjust" => Some(computed_inherited_css_keyword_property_value(
            runtime, handle, property, "economy",
        )),
        "quotes" => Some(computed_inherited_css_keyword_property_value(
            runtime, handle, property, "auto",
        )),
        "scrollbar-color" => Some(computed_inherited_css_keyword_property_value(
            runtime, handle, property, "auto",
        )),
        "scrollbar-width" => Some(computed_non_inherited_css_keyword_property_value(
            runtime, handle, property, "auto",
        )),
        "link-parameters" => Some(computed_non_inherited_css_keyword_property_value(
            runtime, handle, property, "none",
        )),
        "text-size-adjust" => Some(computed_text_size_adjust_value(runtime, handle)),
        "transition-property" | "transition-behavior" => {
            inline_style_entry_for_inline_style(runtime, handle, property).map(|entry| entry.value)
        }
        _ => None,
    }
}

fn computed_inherited_css_keyword_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    initial: &str,
) -> String {
    let Some(value) = inline_style_property_value_for_inline_style(runtime, handle, property)
    else {
        return inherited_computed_style_value(runtime, handle, property, initial);
    };
    match css_wide_keyword(&value).as_deref() {
        Some("inherit") | Some("unset") => {
            inherited_computed_style_value(runtime, handle, property, initial)
        }
        Some(_) => initial.to_owned(),
        None => value,
    }
}

fn computed_text_size_adjust_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let Some(value) =
        inline_style_property_value_for_inline_style(runtime, handle, "text-size-adjust")
    else {
        return inherited_computed_style_value(runtime, handle, "text-size-adjust", "auto");
    };
    match css_wide_keyword(&value).as_deref() {
        Some("inherit") | Some("unset") => {
            inherited_computed_style_value(runtime, handle, "text-size-adjust", "auto")
        }
        Some(_) => "auto".to_owned(),
        None => computed_text_size_adjust_specified_value(runtime, handle, &value).unwrap_or(value),
    }
}

fn computed_text_size_adjust_specified_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto") {
        return Some("auto".to_owned());
    }
    if value.eq_ignore_ascii_case("none") {
        return Some("100%".to_owned());
    }
    let percent =
        resolve_css_percentage_only_with_context(value, css_numeric_context(runtime, handle))?;
    (percent >= 0.0).then(|| format_css_percent(percent))
}

fn computed_non_inherited_css_keyword_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    initial: &str,
) -> String {
    let Some(value) = inline_style_property_value_for_inline_style(runtime, handle, property)
    else {
        return initial.to_owned();
    };
    match css_wide_keyword(&value).as_deref() {
        Some("inherit") => inherited_computed_style_value(runtime, handle, property, initial),
        Some(_) => initial.to_owned(),
        None => value,
    }
}

fn computed_text_emphasis_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    let style =
        computed_style_property_value_with_context(runtime, handle, "text-emphasis-style", context);
    let color =
        computed_style_property_value_with_context(runtime, handle, "text-emphasis-color", context);
    format!("{style} {color}")
}

fn computed_font_variant_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    let values = font_variant_longhands()
        .iter()
        .map(|property| resolution.computed_property(runtime, handle, property))
        .collect::<Vec<_>>();
    serialize_font_variant_shorthand_values(&values).unwrap_or_default()
}

fn computed_webkit_text_stroke_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    let width = computed_style_property_value_with_context(
        runtime,
        handle,
        "-webkit-text-stroke-width",
        context,
    );
    let color = computed_style_property_value_with_context(
        runtime,
        handle,
        "-webkit-text-stroke-color",
        context,
    );
    format!("{width} {color}")
}

fn computed_text_decoration_paint_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, property)?;
    match css_wide_keyword(&entry.value).as_deref() {
        Some("inherit") => Some(inherited_computed_style_value(
            runtime,
            handle,
            property,
            "match-text",
        )),
        Some(_) => Some("match-text".to_owned()),
        None => Some(entry.value),
    }
}

fn inline_text_decoration_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let Some((entries, shared_css_wide_keyword)) =
        inline_shorthand_entries(runtime, handle, text_decoration_shorthand_longhands(), &[])
    else {
        return String::new();
    };
    if let Some(keyword) = shared_css_wide_keyword {
        return keyword;
    }
    serialize_text_decoration_shorthand(
        &entries[0].value,
        &entries[1].value,
        &entries[2].value,
        &entries[3].value,
        text_decoration_value_is_currentcolor(&entries[3].value),
    )
}

fn computed_text_decoration_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    let line = computed_style_property_value_with_context(
        runtime,
        handle,
        "text-decoration-line",
        context,
    );
    let thickness = computed_style_property_value_with_context(
        runtime,
        handle,
        "text-decoration-thickness",
        context,
    );
    let style = computed_style_property_value_with_context(
        runtime,
        handle,
        "text-decoration-style",
        context,
    );
    let color = computed_style_property_value_with_context(
        runtime,
        handle,
        "text-decoration-color",
        context,
    );
    let color_is_currentcolor =
        computed_text_decoration_color_is_currentcolor(runtime, handle, context);
    serialize_text_decoration_shorthand(&line, &thickness, &style, &color, color_is_currentcolor)
}

fn serialize_text_decoration_shorthand(
    line: &str,
    thickness: &str,
    style: &str,
    color: &str,
    color_is_currentcolor: bool,
) -> String {
    let line = text_decoration_component_or_initial(line, "none");
    let thickness = text_decoration_component_or_initial(thickness, "auto");
    let style = text_decoration_component_or_initial(style, "solid");
    let defaults =
        line == "none" && thickness == "auto" && style == "solid" && color_is_currentcolor;

    let mut values = Vec::new();
    if defaults || line != "none" {
        values.push(line);
    }
    if thickness != "auto" {
        values.push(thickness);
    }
    if style != "solid" {
        values.push(style);
    }
    if !color_is_currentcolor {
        values.push(text_decoration_component_or_initial(color, "currentcolor"));
    }
    values.join(" ")
}

fn computed_text_decoration_color_is_currentcolor(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> bool {
    inline_style_entry_for_inline_style(runtime, handle, "text-decoration-color")
        .map(|entry| text_decoration_value_is_currentcolor(&entry.value))
        .or_else(|| {
            raw_stylo_computed_style_value_with_context(
                runtime,
                handle,
                "text-decoration-color",
                context,
            )
            .map(|value| text_decoration_value_is_currentcolor(&value))
        })
        .unwrap_or(true)
}

fn text_decoration_value_is_currentcolor(value: &str) -> bool {
    text_decoration_component_or_initial(value, "currentcolor").eq_ignore_ascii_case("currentcolor")
}

fn text_decoration_component_or_initial<'a>(value: &'a str, initial: &'static str) -> &'a str {
    let value = value.trim();
    if value.is_empty() || css_wide_keyword(value).is_some() {
        initial
    } else {
        value
    }
}

fn computed_inline_text_decoration_line_value(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, "text-decoration-line")?;
    match css_wide_keyword(&entry.value).as_deref() {
        Some("inherit") => Some(inherited_computed_style_value(
            runtime,
            handle,
            "text-decoration-line",
            "none",
        )),
        Some(_) => Some("none".to_owned()),
        None => Some(entry.value),
    }
}

fn computed_inline_zoom_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, "zoom")?;
    match css_wide_keyword(&entry.value).as_deref() {
        Some("inherit") => Some(inherited_computed_style_value(runtime, handle, "zoom", "1")),
        Some(_) => Some("1".to_owned()),
        None => resolve_computed_zoom_with_resolution(runtime, handle, &entry.value, resolution),
    }
}

fn computed_animation_range_endpoint_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, property)?;
    let context = css_numeric_context(runtime, handle);
    let kind = match property {
        "animation-range-start" => AnimationRangeEndpointKind::Start,
        "animation-range-end" => AnimationRangeEndpointKind::End,
        _ => return None,
    };
    let values = top_level_comma_separated_component_values(&entry.value)
        .unwrap_or_else(|| vec![entry.value]);
    let resolved = values
        .into_iter()
        .map(|value| computed_single_animation_range_endpoint(&value, kind, context))
        .collect::<Option<Vec<_>>>()?;
    (!resolved.is_empty()).then(|| resolved.join(", "))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnimationRangeEndpointKind {
    Start,
    End,
}

fn computed_single_animation_range_endpoint(
    value: &str,
    kind: AnimationRangeEndpointKind,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("normal") {
        return Some("normal".to_owned());
    }
    let lowered = value.to_ascii_lowercase();
    let (name, offset) = if let Some(name) = animation_range_name_prefix(&lowered) {
        let offset = value[name.len()..].trim();
        (Some(name), offset)
    } else {
        (None, value)
    };
    if offset.is_empty() {
        return name.map(str::to_owned);
    }
    let serialized_offset = computed_animation_range_offset(offset, context)?;
    Some(match name {
        Some(name) if animation_range_offset_is_default_for_endpoint(&serialized_offset, kind) => {
            name.to_owned()
        }
        Some(name) => format!("{name} {serialized_offset}"),
        None => serialized_offset,
    })
}

fn animation_range_offset_is_default_for_endpoint(
    offset: &str,
    kind: AnimationRangeEndpointKind,
) -> bool {
    match kind {
        AnimationRangeEndpointKind::Start => offset == "0%",
        AnimationRangeEndpointKind::End => offset == "100%",
    }
}

fn computed_animation_range_offset(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    if let Some(percent) = resolve_css_percentage_only(value) {
        return Some(format_css_percent(percent));
    }
    if let Some(px) = resolve_css_length_only(value, context) {
        return Some(format_css_px(px));
    }
    if value.contains('%') {
        return Some(value.to_owned());
    }
    let px = moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::LengthPercentage {
            basis: 0.0,
            unitless: moli_css_parse::UnitlessLength::ZeroOnly,
        },
        context,
    )?
    .px_length()?;
    Some(format_css_px(px))
}

fn resolve_css_percentage_only(value: &str) -> Option<f64> {
    resolve_css_percentage_only_with_context(
        value,
        moli_css_parse::CssNumericContext::supports_probe(),
    )
}

fn resolve_css_percentage_only_with_context(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<f64> {
    let value = value.trim();
    if !value.contains('%') {
        return None;
    }
    let basis_100 = moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::LengthPercentage {
            basis: 100.0,
            unitless: moli_css_parse::UnitlessLength::ZeroOnly,
        },
        context,
    )?
    .px_length()?;
    let basis_200 = moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::LengthPercentage {
            basis: 200.0,
            unitless: moli_css_parse::UnitlessLength::ZeroOnly,
        },
        context,
    )?
    .px_length()?;
    ((basis_200 - (basis_100 * 2.0)).abs() < 1e-9).then_some(basis_100)
}

fn resolve_css_length_only(value: &str, context: moli_css_parse::CssNumericContext) -> Option<f64> {
    if value.contains('%') {
        return None;
    }
    moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::PxLength(moli_css_parse::UnitlessLength::ZeroOnly),
        context,
    )?
    .px_length()
}

fn computed_css_numeric_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let rule = css_numeric_computed_property_rule(property)?;
    let entry = inline_style_entry_for_inline_style(runtime, handle, property)?;
    let context = css_numeric_context(runtime, handle);
    let values = top_level_comma_separated_component_values(&entry.value)
        .unwrap_or_else(|| vec![entry.value]);
    let resolved = values
        .into_iter()
        .map(|value| match rule {
            CssNumericComputedPropertyRule::TimeList { non_negative } => {
                let seconds = moli_css_parse::resolve_css_numeric(
                    &value,
                    moli_css_parse::CssNumericKind::Time,
                    context,
                )?
                .time_seconds()?;
                (!non_negative || seconds >= 0.0).then(|| format_css_seconds(seconds))
            }
            CssNumericComputedPropertyRule::AnimationDurationList => {
                if value.eq_ignore_ascii_case("auto") {
                    return Some(resolve_computed_animation_duration(runtime, handle, &value));
                }
                let seconds = moli_css_parse::resolve_css_numeric(
                    &value,
                    moli_css_parse::CssNumericKind::Time,
                    context,
                )?
                .time_seconds()?;
                (seconds >= 0.0).then(|| format_css_seconds(seconds))
            }
            CssNumericComputedPropertyRule::AnimationIterationCountList => {
                if value.eq_ignore_ascii_case("infinite") {
                    return Some("infinite".to_owned());
                }
                let number = moli_css_parse::resolve_css_numeric(
                    &value,
                    moli_css_parse::CssNumericKind::Number,
                    context,
                )?
                .number()?;
                (number >= 0.0).then(|| format_css_number(number))
            }
        })
        .collect::<Option<Vec<_>>>()?;
    (!resolved.is_empty()).then(|| resolved.join(", "))
}

#[derive(Clone, Copy)]
enum CssNumericComputedPropertyRule {
    TimeList { non_negative: bool },
    AnimationDurationList,
    AnimationIterationCountList,
}

fn css_numeric_computed_property_rule(property: &str) -> Option<CssNumericComputedPropertyRule> {
    Some(match property {
        "animation-delay" | "transition-delay" => CssNumericComputedPropertyRule::TimeList {
            non_negative: false,
        },
        "animation-duration" => CssNumericComputedPropertyRule::AnimationDurationList,
        "transition-duration" => CssNumericComputedPropertyRule::TimeList { non_negative: true },
        "animation-iteration-count" => CssNumericComputedPropertyRule::AnimationIterationCountList,
        _ => return None,
    })
}

fn css_numeric_context(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> moli_css_parse::CssNumericContext {
    let read = ComputedStyleRead::new(runtime, handle);
    css_numeric_context_with_viewport_and_resolution(
        runtime,
        handle,
        read.context.viewport(),
        read.resolution_context(),
    )
}

fn css_numeric_context_with_viewport_and_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    viewport: StyleViewport,
    resolution: StyleResolutionContext<'_>,
) -> moli_css_parse::CssNumericContext {
    let width = nearest_size_container_width(runtime, handle, resolution).unwrap_or(100.0);
    let font_size = inline_font_size_px(runtime, handle)
        .or_else(|| computed_font_size_px_with_resolution(runtime, handle, resolution))
        .unwrap_or(16.0);
    let root_font_size = runtime
        .dom_host()
        .owner_document_handle(handle)
        .and_then(|document| {
            runtime
                .dom_host()
                .document_element_handle_for_document(document)
        })
        .and_then(|document_element| {
            inline_font_size_px(runtime, document_element).or_else(|| {
                computed_font_size_px_with_resolution(runtime, document_element, resolution)
            })
        })
        .unwrap_or(16.0);
    let line_height =
        computed_line_height_px_with_resolution(runtime, handle, resolution).unwrap_or(font_size);
    let viewport_width = viewport
        .width
        .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_width);
    let viewport_height = viewport
        .height
        .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_height);
    moli_css_parse::CssNumericContext {
        container_lengths: Some(moli_css_parse::ContainerQueryLengthContext {
            width_px: width,
            height_px: width,
            inline_size_px: width,
            block_size_px: width,
        }),
        font_size_px: Some(font_size),
        root_font_size_px: Some(root_font_size),
        line_height_px: Some(line_height),
        viewport_width_px: Some(viewport_width),
        viewport_height_px: Some(viewport_height),
        // Stylo 0.20 resolves tree-counting functions lazily from its computed
        // value context. Do not duplicate that work for every renderer numeric
        // context, most of which never contains sibling-index()/sibling-count().
        sibling_index: None,
        sibling_count: None,
    }
}

fn nearest_size_container_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    let mut current = flat_tree_element_parent(runtime, handle);
    let mut visited = HashSet::new();
    while let Some(candidate) = current {
        if !visited.insert(candidate) {
            return None;
        }
        if element_is_size_container(runtime, candidate, resolution) {
            return inline_width_px_with_resolution(runtime, candidate, resolution);
        }
        current = flat_tree_element_parent(runtime, candidate);
    }
    None
}

fn element_is_size_container(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> bool {
    let container = resolution.computed_property(runtime, handle, "container");
    if container
        .split_once('/')
        .is_some_and(|(_, ty)| container_type_is_size_container(ty))
    {
        return true;
    }
    let ty = resolution.computed_property(runtime, handle, "container-type");
    container_type_is_size_container(&ty)
}

fn format_css_seconds(seconds: f64) -> String {
    format!("{}s", format_css_number(seconds))
}

fn format_css_percent(percent: f64) -> String {
    format!("{}%", format_css_number(percent))
}

fn format_css_number(value: f64) -> String {
    let value = normalize_css_number_for_serialization(value);
    format_css_number_exact(value)
}

fn format_css_number_exact(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let mut serialized = format!("{value:.12}");
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

fn normalize_css_number_for_serialization(value: f64) -> f64 {
    let rounded_integer = value.round();
    // PDB/Stylo values can carry f32 roundoff into the f64 CSSOM boundary.
    // Scale the integer tolerance with f32 precision, but cap it so genuine
    // fractional values remain observable.
    let integer_tolerance = (value.abs() * f64::from(f32::EPSILON)).clamp(1e-6, 1e-5);
    if (value - rounded_integer).abs() < integer_tolerance {
        return rounded_integer;
    }
    for scale in [10.0, 100.0, 1_000.0, 10_000.0, 100_000.0, 1_000_000.0] {
        let rounded = (value * scale).round() / scale;
        if (value - rounded).abs() < 2e-6 {
            return rounded;
        }
    }
    value
}

fn computed_animation_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    serialize_animation_shorthand_from_longhands([
        animation_longhand_components(runtime, handle, context, "animation-duration", "0s"),
        animation_longhand_components(
            runtime,
            handle,
            context,
            "animation-timing-function",
            "ease",
        ),
        animation_longhand_components(runtime, handle, context, "animation-delay", "0s"),
        animation_longhand_components(runtime, handle, context, "animation-iteration-count", "1"),
        animation_longhand_components(runtime, handle, context, "animation-direction", "normal"),
        animation_longhand_components(runtime, handle, context, "animation-fill-mode", "none"),
        animation_longhand_components(runtime, handle, context, "animation-play-state", "running"),
        animation_longhand_components(runtime, handle, context, "animation-name", "none"),
    ])
}

fn inline_animation_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let Some((entries, shared_css_wide_keyword)) = inline_shorthand_entries(
        runtime,
        handle,
        animation_shorthand_longhands(),
        &[
            "animation-timeline",
            "animation-range-start",
            "animation-range-end",
        ],
    ) else {
        return String::new();
    };
    if let Some(keyword) = shared_css_wide_keyword {
        return keyword;
    }

    let mut longhands: [Vec<String>; 8] = Default::default();
    for (index, entry) in entries
        .iter()
        .take(animation_shorthand_longhands().len())
        .enumerate()
    {
        longhands[index] = top_level_comma_separated_component_values(&entry.value)
            .unwrap_or_else(|| vec![entry.value.clone()]);
    }
    serialize_animation_shorthand_from_longhands(longhands)
}

fn inline_animation_range_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let Some((entries, shared_css_wide_keyword)) = inline_shorthand_entries(
        runtime,
        handle,
        &["animation-range-start", "animation-range-end"],
        &[],
    ) else {
        return String::new();
    };
    if let Some(keyword) = shared_css_wide_keyword {
        return keyword;
    }
    serialize_animation_range_shorthand(&entries[0].value, &entries[1].value)
}

fn computed_animation_range_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    let start = computed_style_property_value_with_context(
        runtime,
        handle,
        "animation-range-start",
        context,
    );
    let end =
        computed_style_property_value_with_context(runtime, handle, "animation-range-end", context);
    serialize_animation_range_shorthand(&start, &end)
}

pub(crate) fn serialize_animation_range_shorthand(start: &str, end: &str) -> String {
    let starts = top_level_comma_separated_component_values(start)
        .unwrap_or_else(|| vec![start.trim().to_owned()]);
    let ends = top_level_comma_separated_component_values(end)
        .unwrap_or_else(|| vec![end.trim().to_owned()]);
    if starts.is_empty() || starts.len() != ends.len() {
        return String::new();
    }
    starts
        .iter()
        .zip(ends.iter())
        .map(|(start, end)| serialize_single_animation_range(start, end))
        .collect::<Vec<_>>()
        .join(", ")
}

fn serialize_single_animation_range(start: &str, end: &str) -> String {
    let start = start.trim();
    let end = end.trim();
    if start.is_empty() || end.is_empty() {
        return String::new();
    }
    if start.eq_ignore_ascii_case("normal") && end.eq_ignore_ascii_case("normal") {
        return "normal".to_owned();
    }
    if let Some(start_name) = animation_range_name_only(start)
        && animation_range_name_only(end).is_some_and(|end_name| end_name == start_name)
    {
        return start.to_owned();
    }
    if animation_range_is_length_percentage_only(start)
        && (end.eq_ignore_ascii_case("normal") || animation_range_is_default_end_offset(end))
    {
        return start.to_owned();
    }
    if let Some(start_name) = animation_range_name_with_offset(start)
        && animation_range_name_only(end).is_some_and(|end_name| end_name == start_name)
    {
        start.to_owned()
    } else {
        format!("{start} {end}")
    }
}

fn animation_range_name_only(value: &str) -> Option<&'static str> {
    let name = animation_range_name_prefix(value)?;
    (value[name.len()..].trim().is_empty()).then_some(name)
}

fn animation_range_name_with_offset(value: &str) -> Option<&'static str> {
    let name = animation_range_name_prefix(value)?;
    (!value[name.len()..].trim().is_empty()).then_some(name)
}

fn animation_range_name_prefix(value: &str) -> Option<&'static str> {
    [
        "entry-crossing",
        "exit-crossing",
        "cover",
        "contain",
        "entry",
        "exit",
    ]
    .into_iter()
    .find(|name| {
        value == *name
            || value
                .strip_prefix(name)
                .is_some_and(|rest| rest.starts_with(char::is_whitespace))
    })
}

fn animation_range_is_length_percentage_only(value: &str) -> bool {
    !value.eq_ignore_ascii_case("normal") && animation_range_name_prefix(value).is_none()
}

fn animation_range_is_default_end_offset(value: &str) -> bool {
    value == "100%" || value == "calc(100%)"
}

pub(crate) fn serialize_animation_shorthand_from_longhands(longhands: [Vec<String>; 8]) -> String {
    if let Some(keyword) = shared_css_wide_keyword_for_longhands(&longhands) {
        return keyword;
    }
    let animation_count = longhands
        .iter()
        .map(Vec::len)
        .max()
        .filter(|count| *count > 0)
        .unwrap_or(1);
    (0..animation_count)
        .map(|index| {
            serialize_single_computed_animation(
                animation_value_at(&longhands[0], index, "0s"),
                animation_value_at(&longhands[1], index, "ease"),
                animation_value_at(&longhands[2], index, "0s"),
                animation_value_at(&longhands[3], index, "1"),
                animation_value_at(&longhands[4], index, "normal"),
                animation_value_at(&longhands[5], index, "none"),
                animation_value_at(&longhands[6], index, "running"),
                animation_value_at(&longhands[7], index, "none"),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn animation_longhand_components(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
    property: &str,
    initial: &str,
) -> Vec<String> {
    let value = computed_style_property_value_with_context(runtime, handle, property, context);
    let value = if value.is_empty() || css_wide_keyword(&value).is_some() {
        initial.to_owned()
    } else {
        value
    };
    top_level_comma_separated_component_values(&value).unwrap_or_else(|| vec![value])
}

fn animation_value_at<'a>(values: &'a [String], index: usize, initial: &'a str) -> &'a str {
    values
        .get(index)
        .or_else(|| values.last())
        .map(String::as_str)
        .unwrap_or(initial)
}

fn serialize_single_computed_animation(
    duration: &str,
    timing_function: &str,
    delay: &str,
    iteration_count: &str,
    direction: &str,
    fill_mode: &str,
    play_state: &str,
    name: &str,
) -> String {
    let duration = if duration == "auto" { "0s" } else { duration };
    let has_duration = duration != "0s";
    let has_timing_function = timing_function != "ease";
    let has_delay = delay != "0s";
    let has_iteration_count = iteration_count != "1";
    let has_direction = direction != "normal";
    let has_fill_mode = fill_mode != "none";
    let has_play_state = play_state != "running";
    let has_name = name != "none";
    let mut components = Vec::new();

    if has_duration || has_delay {
        components.push(duration);
    }
    if has_timing_function || animation_timing_keyword_name_requires_disambiguation(name) {
        components.push(timing_function);
    }
    if has_delay {
        components.push(delay);
    }
    if has_iteration_count {
        components.push(iteration_count);
    }
    if has_direction || animation_direction_keyword_name_requires_disambiguation(name) {
        components.push(direction);
    }
    if has_fill_mode || animation_fill_mode_keyword_name_requires_disambiguation(name) {
        components.push(fill_mode);
    }
    if has_play_state || animation_play_state_keyword_name_requires_disambiguation(name) {
        components.push(play_state);
    }
    if has_name || components.is_empty() {
        components.push(name);
    }

    components.join(" ")
}

fn animation_timing_keyword_name_requires_disambiguation(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "ease" | "linear" | "ease-in" | "ease-out" | "ease-in-out" | "step-start" | "step-end"
    )
}

fn animation_direction_keyword_name_requires_disambiguation(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "normal" | "reverse" | "alternate" | "alternate-reverse"
    )
}

fn animation_fill_mode_keyword_name_requires_disambiguation(name: &str) -> bool {
    if name.eq_ignore_ascii_case("none") {
        return false;
    }
    matches!(
        name.to_ascii_lowercase().as_str(),
        "none" | "forwards" | "backwards" | "both"
    )
}

fn animation_play_state_keyword_name_requires_disambiguation(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "running" | "paused")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedTransitionComponent {
    property: String,
    duration: String,
    timing_function: String,
    delay: String,
    behavior: String,
}

pub(crate) fn parse_transition_shorthand_entries(value: &str) -> Option<[Vec<String>; 5]> {
    let transitions = parse_transition_shorthand_components(value)?;
    Some([
        transitions
            .iter()
            .map(|transition| transition.property.clone())
            .collect(),
        transitions
            .iter()
            .map(|transition| transition.duration.clone())
            .collect(),
        transitions
            .iter()
            .map(|transition| transition.timing_function.clone())
            .collect(),
        transitions
            .iter()
            .map(|transition| transition.delay.clone())
            .collect(),
        transitions
            .iter()
            .map(|transition| transition.behavior.clone())
            .collect(),
    ])
}

fn parse_transition_shorthand_components(value: &str) -> Option<Vec<ParsedTransitionComponent>> {
    let layers =
        top_level_comma_separated_component_values(value).filter(|layers| !layers.is_empty())?;
    layers
        .into_iter()
        .map(|layer| parse_single_transition(&layer))
        .collect()
}

fn parse_single_transition(value: &str) -> Option<ParsedTransitionComponent> {
    let tokens = box_shorthand_value_components(value)?;
    if tokens.iter().any(|token| css_wide_keyword(token).is_some()) {
        return (tokens.len() == 1).then(|| ParsedTransitionComponent {
            property: tokens[0].clone(),
            duration: tokens[0].clone(),
            timing_function: tokens[0].clone(),
            delay: tokens[0].clone(),
            behavior: tokens[0].clone(),
        });
    }
    let mut transition = ParsedTransitionComponent {
        property: "all".to_owned(),
        duration: "0s".to_owned(),
        timing_function: "ease".to_owned(),
        delay: "0s".to_owned(),
        behavior: "normal".to_owned(),
    };
    let mut seen_property = false;
    let mut seen_duration = false;
    let mut seen_delay = false;
    let mut seen_timing_function = false;
    let mut seen_behavior = false;
    for token in tokens {
        if let Some(time) = normalize_transition_time_token(&token) {
            if !seen_duration {
                let seconds = css_time_seconds(&token)?;
                if seconds < 0.0 {
                    return None;
                }
                transition.duration = time;
                seen_duration = true;
                continue;
            }
            if !seen_delay {
                transition.delay = time;
                seen_delay = true;
                continue;
            }
            return None;
        }
        if let Some(timing_function) = normalize_timing_function_value(&token) {
            if seen_timing_function {
                return None;
            }
            transition.timing_function = timing_function;
            seen_timing_function = true;
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            "normal" | "allow-discrete" if !seen_behavior => {
                transition.behavior = token.to_ascii_lowercase();
                seen_behavior = true;
                continue;
            }
            _ => {}
        }
        if seen_property || !transition_property_token_is_valid(&token) {
            return None;
        }
        transition.property = transition_property_token_serialization(&token)?;
        seen_property = true;
    }
    if transition.property.eq_ignore_ascii_case("none")
        && (seen_duration || seen_delay || seen_timing_function || seen_behavior)
    {
        return None;
    }
    Some(transition)
}

fn normalize_transition_time_token(value: &str) -> Option<String> {
    css_time_seconds(value).map(format_css_seconds)
}

fn css_time_seconds(value: &str) -> Option<f64> {
    moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::Time,
        moli_css_parse::CssNumericContext::supports_probe(),
    )?
    .time_seconds()
}

fn transition_property_token_is_valid(value: &str) -> bool {
    transition_property_token_serialization(value).is_some()
}

fn transition_property_token_serialization(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let ident = input
        .parse_entirely(|input| {
            input
                .expect_ident_cloned()
                .map_err(|_| input.new_custom_error::<(), ()>(()))
        })
        .ok()?;
    let lowered = ident.to_ascii_lowercase();
    if css_wide_keyword(&lowered).is_some() || lowered == "default" {
        return None;
    }
    if lowered == "all" || lowered == "none" {
        return Some(lowered);
    }
    let mut serialized = String::new();
    serialize_identifier(&ident, &mut serialized).ok()?;
    Some(serialized)
}

pub(crate) fn normalize_transition_property_list(value: &str) -> Option<String> {
    let layers =
        top_level_comma_separated_component_values(value).filter(|layers| !layers.is_empty())?;
    let properties = layers
        .into_iter()
        .map(|layer| transition_property_token_serialization(&layer))
        .collect::<Option<Vec<_>>>()?;
    if properties.len() > 1 && properties.iter().any(|layer| layer == "none") {
        return None;
    }
    Some(properties.join(", "))
}

pub(crate) fn normalize_transition_behavior_list(value: &str) -> Option<String> {
    let layers =
        top_level_comma_separated_component_values(value).filter(|layers| !layers.is_empty())?;
    layers
        .into_iter()
        .map(|layer| {
            let layer = layer.to_ascii_lowercase();
            matches!(layer.as_str(), "normal" | "allow-discrete").then_some(layer)
        })
        .collect::<Option<Vec<_>>>()
        .map(|layers| layers.join(", "))
}

pub(crate) fn normalize_transition_timing_function_list(value: &str) -> Option<String> {
    let layers = top_level_comma_separated_raw_component_values(value)?;
    layers
        .iter()
        .map(|layer| normalize_timing_function_value(layer))
        .collect::<Option<Vec<_>>>()
        .map(|layers| layers.join(", "))
}

fn normalize_timing_function_value(value: &str) -> Option<String> {
    let value = value.trim();
    let lowered = value.to_ascii_lowercase();
    match lowered.as_str() {
        "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" => return Some(lowered),
        "step-start" => return Some("steps(1, start)".to_owned()),
        "step-end" => return Some("steps(1)".to_owned()),
        _ => {}
    }
    if let Some(inner) = css_function_inner(value, "cubic-bezier") {
        return normalize_cubic_bezier_timing_function_value(inner);
    }
    if let Some(inner) = css_function_inner(value, "steps") {
        let arguments = top_level_comma_separated_component_values(inner)?;
        if arguments.is_empty() || arguments.len() > 2 {
            return None;
        }
        let steps = normalize_steps_count_for_specified_value(&arguments[0])?;
        let position = arguments
            .get(1)
            .map(|position| position.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "jump-end".to_owned());
        if position == "jump-none"
            && let Ok(number) = steps.parse::<f64>()
            && number <= 1.0
        {
            return None;
        }
        match position.as_str() {
            "start" => {}
            "end" | "jump-end" => {
                if arguments.len() == 1 || position == "end" || position == "jump-end" {
                    return Some(format!("steps({steps})"));
                }
            }
            "jump-start" | "jump-both" => {}
            "jump-none" => {}
            _ => return None,
        }
        return Some(format!(
            "steps({steps}, {})",
            match position.as_str() {
                "end" => "jump-end",
                other => other,
            }
        ));
    }
    if css_function_inner(value, "linear").is_some() {
        let parsed = parse_linear_timing_function_value(value)?;
        if parsed.force_computed_serialization {
            return computed_linear_timing_function_value_from_parsed(
                &parsed,
                moli_css_parse::CssNumericContext::supports_probe(),
            );
        }
        return Some(specified_linear_timing_function_value(&parsed));
    }
    None
}

struct SpecifiedTimingNumber {
    serialized: String,
    value: f64,
    literal: bool,
}

fn normalize_cubic_bezier_timing_function_value(inner: &str) -> Option<String> {
    let arguments = top_level_comma_separated_component_values(inner)?;
    if arguments.len() != 4 {
        return None;
    }
    let numbers = arguments
        .iter()
        .map(|argument| normalize_timing_number_specified_value(argument))
        .collect::<Option<Vec<_>>>()?;
    for index in [0, 2] {
        if numbers[index].literal && !(0.0..=1.0).contains(&numbers[index].value) {
            return None;
        }
    }
    Some(format!(
        "cubic-bezier({}, {}, {}, {})",
        numbers[0].serialized, numbers[1].serialized, numbers[2].serialized, numbers[3].serialized
    ))
}

fn normalize_timing_number_specified_value(value: &str) -> Option<SpecifiedTimingNumber> {
    let trimmed = value.trim();
    if let Some(number) = moli_css_parse::parse_number(trimmed) {
        return Some(SpecifiedTimingNumber {
            serialized: format_css_number(number),
            value: number,
            literal: true,
        });
    }
    let serialized = moli_css_parse::normalize_cssom_component_value_serialization(trimmed)?;
    let value = match computed_timing_number_value(
        &serialized,
        moli_css_parse::CssNumericContext::supports_probe(),
    ) {
        Some(value) => value,
        None if dynamic_timing_number_specified_value_is_supported(&serialized) => {
            return Some(SpecifiedTimingNumber {
                serialized,
                value: 0.0,
                literal: false,
            });
        }
        None => return None,
    };
    let serialized = if css_static_calc_expression_can_be_folded(&serialized) {
        format!("calc({})", format_css_number(value))
    } else {
        serialized
    };
    Some(SpecifiedTimingNumber {
        serialized,
        value,
        literal: false,
    })
}

fn timing_number_has_dynamic_math(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("sign(") || lower.contains("sibling-index(") || lower.contains("sibling-count(")
}

fn dynamic_timing_number_specified_value_is_supported(value: &str) -> bool {
    let value = value.trim();
    if value.eq_ignore_ascii_case("sibling-index()")
        || value.eq_ignore_ascii_case("sibling-count()")
    {
        return true;
    }
    if css_function_inner(value, "sign").is_some() {
        return true;
    }
    css_function_inner(value, "calc")
        .is_some_and(|_| timing_number_has_dynamic_math(value) && balanced_css_function(value))
}

fn balanced_css_function(value: &str) -> bool {
    moli_css_parse::balanced_function_len(value).is_some_and(|len| len == value.len())
}

struct ParsedLinearTimingFunction {
    stops: Vec<ParsedLinearStop>,
    force_computed_serialization: bool,
}

struct ParsedLinearStop {
    output_raw: String,
    output: String,
    offsets: Vec<ParsedLinearStopOffset>,
}

struct ParsedLinearStopOffset {
    raw: String,
    specified: String,
}

fn parse_linear_timing_function_value(value: &str) -> Option<ParsedLinearTimingFunction> {
    let inner = css_function_inner(value, "linear")?;
    let raw_stops = top_level_comma_separated_raw_component_values(inner)?;
    if raw_stops.len() < 2 {
        return None;
    }
    let mut force_computed_serialization = false;
    let stops = raw_stops
        .into_iter()
        .map(|raw_stop| {
            let components = top_level_whitespace_separated_raw_component_values(&raw_stop)?;
            if components.is_empty() || components.len() > 3 {
                return None;
            }
            let (output, force_computed_output) =
                normalize_linear_stop_output_value(&components[0])?;
            force_computed_serialization |= force_computed_output;
            let offsets = components
                .iter()
                .skip(1)
                .map(|component| {
                    Some(ParsedLinearStopOffset {
                        raw: component.clone(),
                        specified: normalize_linear_stop_percentage_specified_value(component)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(ParsedLinearStop {
                output_raw: components[0].clone(),
                output,
                offsets,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(ParsedLinearTimingFunction {
        stops,
        force_computed_serialization,
    })
}

fn normalize_linear_stop_output_value(value: &str) -> Option<(String, bool)> {
    if css_calc_nan_number_is_zero(value) {
        return Some(("0".to_owned(), true));
    }
    let value = normalize_timing_number_specified_value(value)?;
    Some((value.serialized, false))
}

fn normalize_linear_stop_percentage_specified_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed
        .strip_suffix('%')
        .and_then(moli_css_parse::parse_number)
        .is_some_and(f64::is_finite)
    {
        let percent = trimmed
            .strip_suffix('%')
            .and_then(moli_css_parse::parse_number)?;
        return Some(format!("{}%", format_css_number_exact(percent)));
    }
    let serialized = moli_css_parse::normalize_cssom_component_value_serialization(trimmed)?;
    let percent = computed_linear_stop_percentage_value(
        &serialized,
        moli_css_parse::CssNumericContext::supports_probe(),
    )?;
    if css_static_calc_expression_can_be_folded(&serialized) {
        Some(format!("calc({})", format_css_percent(percent)))
    } else {
        Some(serialized)
    }
}

fn specified_linear_timing_function_value(parsed: &ParsedLinearTimingFunction) -> String {
    let stops = parsed
        .stops
        .iter()
        .map(|stop| {
            let mut components = Vec::with_capacity(stop.offsets.len() + 1);
            components.push(stop.output.clone());
            components.extend(stop.offsets.iter().map(|offset| offset.specified.clone()));
            components.join(" ")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("linear({stops})")
}

fn normalize_steps_count_for_specified_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if css_steps_count_expression_is_valid(trimmed) {
        let serialized = moli_css_parse::normalize_cssom_component_value_serialization(trimmed)
            .unwrap_or_else(|| trimmed.to_owned());
        if css_static_calc_expression_can_be_folded(&serialized) {
            let number = moli_css_parse::resolve_css_numeric(
                trimmed,
                moli_css_parse::CssNumericKind::Number,
                moli_css_parse::CssNumericContext::supports_probe(),
            )?
            .number()?;
            return Some(format!("calc({})", format_css_number(number)));
        }
        return Some(serialized);
    }
    let number = normalize_css_integer_token(value)?;
    (!number.starts_with('-') && number != "0").then_some(number)
}

fn css_steps_count_expression_is_valid(value: &str) -> bool {
    let value = value.trim();
    if value.eq_ignore_ascii_case("sibling-index()")
        || value.eq_ignore_ascii_case("sibling-count()")
    {
        return true;
    }
    css_function_inner(value, "calc").is_some_and(css_steps_count_calc_expression_is_valid)
}

fn css_steps_count_calc_expression_is_valid(value: &str) -> bool {
    let mut rest = value.trim();
    while !rest.is_empty() {
        let trimmed = rest.trim_start();
        rest = trimmed;
        if let Some(next) = rest.strip_prefix("sibling-index()") {
            rest = next;
            continue;
        }
        if let Some(next) = rest.strip_prefix("sibling-count()") {
            rest = next;
            continue;
        }
        if rest.to_ascii_lowercase().starts_with("sign(")
            && let Some(len) = moli_css_parse::balanced_function_len(rest)
        {
            rest = &rest[len..];
            continue;
        }
        if let Some(len) = moli_css_parse::number_len(rest) {
            rest = &rest[len..];
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            return true;
        };
        if matches!(ch, '+' | '-' | '*' | '/' | '(' | ')') {
            rest = &rest[ch.len_utf8()..];
            continue;
        }
        return false;
    }
    true
}

fn computed_timing_function_list_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, property)?;
    let context = css_numeric_context(runtime, handle);
    let values = top_level_comma_separated_raw_component_values(&entry.value)
        .unwrap_or_else(|| vec![entry.value]);
    let resolved = values
        .into_iter()
        .map(|value| computed_timing_function_value(&value, context))
        .collect::<Option<Vec<_>>>()?;
    (!resolved.is_empty()).then(|| resolved.join(", "))
}

fn computed_timing_function_value(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    let value = value.trim();
    match value.to_ascii_lowercase().as_str() {
        "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" => {
            return Some(value.to_owned());
        }
        "step-start" => return Some("steps(1, start)".to_owned()),
        "step-end" => return Some("steps(1)".to_owned()),
        _ => {}
    }
    if let Some(inner) = css_function_inner(value, "cubic-bezier") {
        return computed_cubic_bezier_timing_function_value(inner, context);
    }
    if let Some(inner) = css_function_inner(value, "steps") {
        let arguments = top_level_comma_separated_component_values(inner)?;
        if arguments.is_empty() || arguments.len() > 2 {
            return None;
        }
        let mut steps = moli_css_parse::resolve_css_numeric(
            &arguments[0],
            moli_css_parse::CssNumericKind::Number,
            context,
        )?
        .number()?;
        steps = steps.round();
        let position = arguments
            .get(1)
            .map(|position| position.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "jump-end".to_owned());
        match position.as_str() {
            "start" => {
                if steps < 1.0 {
                    steps = 1.0;
                }
                Some(format!("steps({}, start)", format_css_number(steps)))
            }
            "end" | "jump-end" => {
                if steps < 1.0 {
                    steps = 1.0;
                }
                Some(format!("steps({})", format_css_number(steps)))
            }
            "jump-start" | "jump-both" => {
                if steps < 1.0 {
                    steps = 1.0;
                }
                Some(format!("steps({}, {position})", format_css_number(steps)))
            }
            "jump-none" => {
                if steps < 2.0 {
                    steps = 2.0;
                }
                Some(format!("steps({}, jump-none)", format_css_number(steps)))
            }
            _ => None,
        }
    } else if css_function_inner(value, "linear").is_some() {
        computed_linear_timing_function_value(value, context)
    } else {
        normalize_timing_function_value(value)
    }
}

fn computed_cubic_bezier_timing_function_value(
    inner: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    let arguments = top_level_comma_separated_component_values(inner)?;
    if arguments.len() != 4 {
        return None;
    }
    let mut numbers = arguments
        .iter()
        .map(|argument| computed_timing_number_value(argument, context))
        .collect::<Option<Vec<_>>>()?;
    numbers[0] = numbers[0].clamp(0.0, 1.0);
    numbers[2] = numbers[2].clamp(0.0, 1.0);
    Some(format!(
        "cubic-bezier({}, {}, {}, {})",
        format_css_number(numbers[0]),
        format_css_number(numbers[1]),
        format_css_number(numbers[2]),
        format_css_number(numbers[3])
    ))
}

fn computed_linear_timing_function_value(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    let parsed = parse_linear_timing_function_value(value)?;
    computed_linear_timing_function_value_from_parsed(&parsed, context)
}

fn computed_linear_timing_function_value_from_parsed(
    parsed: &ParsedLinearTimingFunction,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    let mut points = Vec::new();
    for stop in &parsed.stops {
        let output = if css_calc_nan_number_is_zero(&stop.output_raw) {
            0.0
        } else {
            computed_timing_number_value(&stop.output_raw, context)?
        };
        let output = format_css_number(output);
        if stop.offsets.is_empty() {
            points.push(ComputedLinearPoint {
                output,
                offset: None,
            });
        } else {
            for offset in &stop.offsets {
                points.push(ComputedLinearPoint {
                    output: output.clone(),
                    offset: Some(computed_linear_stop_percentage_value(&offset.raw, context)?),
                });
            }
        }
    }
    if points.len() < 2 {
        return None;
    }
    let mut offsets = points.iter().map(|point| point.offset).collect::<Vec<_>>();
    assign_linear_stop_offsets(&mut offsets)?;
    let stops = points
        .into_iter()
        .zip(offsets)
        .map(|(point, offset)| {
            format!(
                "{} {}",
                point.output,
                format_css_linear_percent(offset.expect("linear stop offsets should be assigned"))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("linear({stops})"))
}

struct ComputedLinearPoint {
    output: String,
    offset: Option<f64>,
}

fn assign_linear_stop_offsets(offsets: &mut [Option<f64>]) -> Option<()> {
    if offsets.len() < 2 {
        return None;
    }
    let specified = offsets
        .iter()
        .enumerate()
        .filter_map(|(index, offset)| offset.map(|_| index))
        .collect::<Vec<_>>();
    if specified.is_empty() {
        let denominator = (offsets.len() - 1) as f64;
        for (index, offset) in offsets.iter_mut().enumerate() {
            *offset = Some(index as f64 * 100.0 / denominator);
        }
        return Some(());
    }

    let first = specified[0];
    if first > 0 {
        let end = offsets[first]?;
        for (index, offset) in offsets.iter_mut().enumerate().take(first) {
            *offset = Some(index as f64 * end / first as f64);
        }
    }

    for pair in specified.windows(2) {
        let start_index = pair[0];
        let end_index = pair[1];
        let start = offsets[start_index]?;
        let end = offsets[end_index]?;
        let span = (end_index - start_index) as f64;
        for (index, offset) in offsets
            .iter_mut()
            .enumerate()
            .take(end_index)
            .skip(start_index + 1)
        {
            let progress = (index - start_index) as f64 / span;
            *offset = Some(start + (end - start) * progress);
        }
    }

    let last = *specified.last()?;
    if last + 1 < offsets.len() {
        let start = offsets[last]?;
        let end = start.max(100.0);
        let span = (offsets.len() - 1 - last) as f64;
        for (index, offset) in offsets.iter_mut().enumerate().skip(last + 1) {
            let progress = (index - last) as f64 / span;
            *offset = Some(start + (end - start) * progress);
        }
    }
    Some(())
}

fn computed_timing_number_value(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<f64> {
    moli_css_parse::resolve_css_numeric(value, moli_css_parse::CssNumericKind::Number, context)?
        .number()
}

fn computed_linear_stop_percentage_value(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<f64> {
    if let Some(percent) = value
        .trim()
        .strip_suffix('%')
        .and_then(moli_css_parse::parse_number)
    {
        return Some(percent);
    }
    moli_css_parse::resolve_css_numeric(value, moli_css_parse::CssNumericKind::Percentage, context)?
        .percentage()
}

fn css_calc_nan_number_is_zero(value: &str) -> bool {
    let compact = value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    compact.eq_ignore_ascii_case("calc(0/0)")
}

fn css_static_calc_expression_can_be_folded(value: &str) -> bool {
    if css_function_inner(value.trim(), "calc").is_none() {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    !lower.contains("sign(")
        && !lower.contains("sibling-index(")
        && !lower.contains("sibling-count(")
        && !lower.contains("var(")
        && !lower.contains("env(")
}

fn format_css_linear_percent(percent: f64) -> String {
    if percent == 0.0 {
        return "0%".to_owned();
    }
    let rounded = (percent * 1_000_000.0).round() / 1_000_000.0;
    let mut serialized = format!("{rounded:.6}");
    while serialized.ends_with('0') {
        serialized.pop();
    }
    if serialized.ends_with('.') {
        serialized.pop();
    }
    format!("{serialized}%")
}

fn css_function_inner<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let lower = value.to_ascii_lowercase();
    let prefix = format!("{name}(");
    if !lower.starts_with(&prefix) || !value.ends_with(')') {
        return None;
    }
    moli_css_parse::balanced_function_len(value)
        .filter(|len| *len == value.len())
        .map(|_| &value[prefix.len()..value.len() - 1])
}

fn top_level_comma_separated_raw_component_values(value: &str) -> Option<Vec<String>> {
    split_top_level_raw_component_values(value, RawComponentSeparator::Comma)
}

fn top_level_whitespace_separated_raw_component_values(value: &str) -> Option<Vec<String>> {
    split_top_level_raw_component_values(value, RawComponentSeparator::Whitespace)
}

enum RawComponentSeparator {
    Comma,
    Whitespace,
}

fn split_top_level_raw_component_values(
    value: &str,
    separator: RawComponentSeparator,
) -> Option<Vec<String>> {
    let mut components = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => {
                depth += 1;
                start.get_or_insert(index);
            }
            ')' => {
                depth = depth.checked_sub(1)?;
                start.get_or_insert(index);
            }
            ',' if depth == 0 && matches!(separator, RawComponentSeparator::Comma) => {
                let component = value[start.unwrap_or(0)..index].trim();
                if component.is_empty() {
                    return None;
                }
                components.push(component.to_owned());
                start = None;
            }
            ch if ch.is_whitespace()
                && depth == 0
                && matches!(separator, RawComponentSeparator::Whitespace) =>
            {
                if let Some(component_start) = start.take() {
                    let component = value[component_start..index].trim();
                    if !component.is_empty() {
                        components.push(component.to_owned());
                    }
                }
            }
            ch if ch.is_whitespace() => {}
            _ => {
                start.get_or_insert(index);
            }
        }
    }
    if depth != 0 {
        return None;
    }
    let component = value[start.unwrap_or(value.len())..].trim();
    if !component.is_empty() {
        components.push(component.to_owned());
    }
    (!components.is_empty()).then_some(components)
}

fn inline_transition_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    transition_shorthand_from_longhands(runtime, handle, None)
}

fn computed_transition_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    transition_shorthand_from_longhands(runtime, handle, Some(context))
}

fn transition_shorthand_from_longhands(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: Option<StyleComputationContext>,
) -> String {
    let mut longhands: [Vec<String>; 5] = Default::default();
    if let Some(context) = context {
        for (index, longhand) in transition_shorthand_longhands().iter().enumerate() {
            let value =
                computed_style_property_value_with_context(runtime, handle, longhand, context);
            let value = if value.is_empty() {
                computed_style_default_value(runtime, handle, longhand)
            } else {
                value
            };
            longhands[index] =
                top_level_comma_separated_component_values(&value).unwrap_or_else(|| vec![value]);
        }
    } else {
        let Some((entries, shared_css_wide_keyword)) =
            inline_shorthand_entries(runtime, handle, transition_shorthand_longhands(), &[])
        else {
            return String::new();
        };
        if let Some(keyword) = shared_css_wide_keyword {
            return keyword;
        }
        for (index, entry) in entries.iter().enumerate() {
            longhands[index] = top_level_comma_separated_component_values(&entry.value)
                .unwrap_or_else(|| vec![entry.value.clone()]);
        }
    }
    serialize_transition_shorthand_from_longhands(longhands)
}

fn inline_shorthand_entries(
    runtime: &JsContextHost,
    handle: DomHandle,
    longhands: &[&str],
    reset_only_longhands: &[&str],
) -> Option<(Vec<StyleEntry>, Option<String>)> {
    let mut entries = Vec::with_capacity(longhands.len() + reset_only_longhands.len());
    let mut priority = None;
    for longhand in longhands.iter().chain(reset_only_longhands.iter()) {
        let value = inline_longhand_property_value_for_shorthand(runtime, handle, longhand)?;
        let entry_priority =
            inline_longhand_property_priority_for_shorthand(runtime, handle, longhand)?;
        if priority.is_some_and(|current| current != entry_priority) {
            return None;
        }
        priority = Some(entry_priority);
        entries.push(StyleEntry {
            name: (*longhand).to_owned(),
            value,
            priority: entry_priority,
        });
    }

    let css_wide_keywords = entries
        .iter()
        .map(|entry| css_wide_keyword(&entry.value))
        .collect::<Option<Vec<_>>>();
    if entries
        .iter()
        .any(|entry| css_wide_keyword(&entry.value).is_some())
    {
        let keywords = css_wide_keywords?;
        let first = keywords.first()?.clone();
        if keywords.iter().all(|keyword| keyword == &first) {
            return Some((entries, Some(first)));
        }
        return None;
    }

    Some((entries, None))
}

fn inline_longhand_property_value_for_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let property = canonical_style_property_name(property);
    if let Some(state) = runtime.element_inline_style_declaration_state(handle)
        && let Some(value) = inline_state_property_value_with_pdb(state, &property)
        && !value.is_empty()
    {
        return Some(value);
    }
    inline_style_entry_for_inline_style(runtime, handle, &property)
        .map(|entry| entry.value)
        .filter(|value| !value.is_empty())
}

fn inline_longhand_property_priority_for_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<bool> {
    let property = canonical_style_property_name(property);
    if let Some(state) = runtime.element_inline_style_declaration_state(handle)
        && let Some(priority) = inline_state_property_priority_with_pdb(state, &property)
    {
        return Some(priority);
    }
    inline_style_entry_for_inline_style(runtime, handle, &property).map(|entry| entry.priority)
}

pub(crate) fn serialize_transition_shorthand_from_longhands(longhands: [Vec<String>; 5]) -> String {
    if let Some(keyword) = shared_css_wide_keyword_for_longhands(&longhands) {
        return keyword;
    }
    let transition_count = longhands
        .iter()
        .map(Vec::len)
        .max()
        .filter(|count| *count > 0)
        .unwrap_or(1);
    (0..transition_count)
        .map(|index| {
            serialize_single_transition(
                transition_value_at(&longhands[0], index, "all"),
                transition_value_at(&longhands[1], index, "0s"),
                transition_value_at(&longhands[2], index, "ease"),
                transition_value_at(&longhands[3], index, "0s"),
                transition_value_at(&longhands[4], index, "normal"),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn shared_css_wide_keyword_for_longhands<const N: usize>(
    longhands: &[Vec<String>; N],
) -> Option<String> {
    let keyword = longhands
        .first()?
        .as_slice()
        .first()
        .and_then(|value| css_wide_keyword(value))?;
    longhands
        .iter()
        .all(|values| matches!(values.as_slice(), [value] if value == &keyword))
        .then_some(keyword)
}

fn transition_value_at<'a>(values: &'a [String], index: usize, initial: &'a str) -> &'a str {
    values
        .get(index)
        .or_else(|| values.last())
        .map(String::as_str)
        .unwrap_or(initial)
}

fn serialize_single_transition(
    property: &str,
    duration: &str,
    timing_function: &str,
    delay: &str,
    behavior: &str,
) -> String {
    if css_wide_keyword(property).is_some()
        && property == duration
        && property == timing_function
        && property == delay
        && property == behavior
    {
        return property.to_owned();
    }
    if property == "none"
        && duration == "0s"
        && timing_function == "ease"
        && delay == "0s"
        && behavior == "normal"
    {
        return "none".to_owned();
    }
    let mut components = Vec::new();
    if property != "all" {
        components.push(property);
    }
    if duration != "0s" || delay != "0s" {
        components.push(duration);
    }
    if timing_function != "ease" {
        components.push(timing_function);
    }
    if delay != "0s" {
        components.push(delay);
    }
    if behavior != "normal" {
        components.push(behavior);
    }
    if components.is_empty() {
        components.push("all");
    }
    components.join(" ")
}

fn shadow_tree_inherited_value_for_initial_stylo_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let initial = match property {
        "color" => "rgb(0, 0, 0)",
        "font-size" => "16px",
        _ => return None,
    };
    if runtime.dom_host().containing_shadow_root(handle).is_none()
        || value != initial
        || inline_style_entry_for_inline_style(runtime, handle, property).is_some()
    {
        return None;
    }
    inherited_style_parent(runtime, handle)
        .map(|parent| resolution.computed_property(runtime, parent, property))
        .filter(|value| !value.is_empty() && value != initial)
}

fn resolve_computed_custom_function_calls(
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

fn dashed_no_arg_function_calls(value: &str) -> Vec<String> {
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
struct CustomCssFunction {
    result: Option<String>,
    container_results: Vec<CustomCssFunctionContainerResult>,
}

#[derive(Clone, Debug)]
struct CustomCssFunctionContainerResult {
    container_name: String,
    width_px: f64,
    result: String,
}

struct CustomFunctionRuleText {
    name: String,
    block: String,
}

struct CustomFunctionContainerRuleText {
    css_text: String,
    block: String,
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

fn collect_custom_functions_from_css(
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

fn custom_function_container_rule_texts(block: &str) -> Vec<CustomFunctionContainerRuleText> {
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

fn flat_tree_element_parent(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
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

fn container_type_is_size_container(value: &str) -> bool {
    value
        .split_whitespace()
        .any(|ty| matches!(ty, "size" | "inline-size"))
}

fn inline_width_px(runtime: &JsContextHost, handle: DomHandle) -> Option<f64> {
    let read = ComputedStyleRead::new(runtime, handle);
    inline_width_px_with_resolution(runtime, handle, read.resolution_context())
}

fn inline_width_px_with_resolution(
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

fn element_has_hidden_attribute(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.has_attribute("hidden"))
}

fn resolve_moli_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> String {
    if property == "accent-color" && value.eq_ignore_ascii_case("auto") {
        return "auto".to_owned();
    }
    if color_property_is_resolved_color(property) {
        return normalize_computed_color(value);
    }
    if matches!(property, "box-shadow" | "text-shadow") {
        let current_color = resolution.computed_property(runtime, handle, "color");
        return normalize_computed_color_functions(value, Some(&current_color));
    }
    if property == "background-image" {
        return resolve_computed_background_image(runtime, handle, value);
    }
    if matches!(property, "transform" | "-webkit-transform")
        && let Some(transform) = computed_transform_matrix_value(value)
    {
        return transform;
    }
    if property == "animation-duration" {
        return resolve_computed_animation_duration(runtime, handle, value);
    }
    if property == "zoom"
        && let Some(zoom) =
            resolve_computed_zoom_with_resolution(runtime, handle, value, resolution)
    {
        return zoom;
    }
    if property == "font-family" {
        return normalize_cssom_font_family_value(value).unwrap_or_else(|| value.to_owned());
    }
    if matches!(property, "grid-template-columns" | "grid-template-rows")
        && let Some(tracks) = resolved_grid_template_tracks(runtime, handle, property)
    {
        return tracks;
    }
    if matches!(property, "width" | "height")
        && let Some(size) = used_size_from_layout_snapshot(runtime, handle)
    {
        let value = if property == "width" {
            size.width
        } else {
            size.height
        };
        return format_non_negative_used_css_px(f64::from(value));
    }
    if property == "width"
        && let Some(width) =
            resolve_computed_width_with_inline_fallback(runtime, handle, value, context, resolution)
    {
        return width;
    }
    if property == "height"
        && let Some(height) = resolve_computed_height_with_inline_fallback(
            runtime, handle, value, context, resolution,
        )
    {
        return height;
    }
    if property == "line-height"
        && let Some(line_height) =
            resolve_computed_line_height_with_resolution(runtime, handle, value, resolution)
    {
        return line_height;
    }
    // Horizontal used-value resolution recursively reads the containing block
    // and the element's own width from this exact retained observation.
    if matches!(property, "margin-left" | "margin-right")
        && let Some(margin) = resolve_computed_horizontal_auto_margin(
            runtime, handle, property, value, context, resolution,
        )
    {
        return margin;
    }
    if matches!(property, "margin-left" | "margin-right")
        && let Some(margin) = resolve_computed_horizontal_margin_with_inline_fallback(
            runtime, handle, property, value, context, resolution,
        )
    {
        return margin;
    }
    if matches!(property, "left" | "right" | "top" | "bottom")
        && let Some(inset) = resolve_computed_inset(runtime, handle, property, value, resolution)
    {
        return inset;
    }
    if matches!(property, "min-width" | "min-height") && (value.is_empty() || value == "auto") {
        return resolve_computed_auto_min_size(runtime, handle, resolution);
    }
    value.to_owned()
}

fn used_size_from_layout_snapshot(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<moli_layout::LayoutSize> {
    if !runtime.layout_policy().uses_real_layout() || !runtime.dom_host().is_connected(handle) {
        return None;
    }
    let document = runtime.layout_document_for_source(handle)?;
    // Reading a CSSOM size never creates or refreshes layout. Use the sizing
    // basis frozen with the existing geometry, or keep the style-only path.
    runtime
        .with_latest_layout_tree_for_document(document, |tree| tree.used_size_for_source(handle))
        .flatten()
}

fn resolved_grid_template_tracks(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    if !runtime.layout_policy().uses_real_layout() {
        return None;
    }
    let grid = observable_used_grid_tracks(
        runtime,
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )
    .ok()??;
    let tracks = match property {
        "grid-template-columns" => &grid.columns,
        "grid-template-rows" => &grid.rows,
        _ => return None,
    };

    serialize_used_grid_track_list(tracks)
}

fn serialize_used_grid_track_list(
    tracks: &moli_layout::LayoutResolvedGridTrackList,
) -> Option<String> {
    if tracks.track_count() != tracks.used_track_sizes.len() {
        return None;
    }
    if tracks.used_track_sizes.is_empty() {
        return Some("none".to_owned());
    }
    if tracks.explicit_line_names.len() != tracks.explicit_track_count.saturating_add(1) {
        return None;
    }
    let mut components = Vec::with_capacity(
        tracks.used_track_sizes.len().saturating_add(
            tracks
                .explicit_line_names
                .iter()
                .filter(|names| !names.is_empty())
                .count(),
        ),
    );
    let mut size_index = 0usize;
    for _ in 0..tracks.negative_implicit_track_count {
        components.push(format_non_negative_used_css_px(f64::from(
            *tracks.used_track_sizes.get(size_index)?,
        )));
        size_index += 1;
    }
    for (track_index, names) in tracks.explicit_line_names.iter().enumerate() {
        if !names.is_empty() {
            let mut serialized = String::from("[");
            for (index, name) in names.iter().enumerate() {
                if index > 0 {
                    serialized.push(' ');
                }
                serialize_identifier(name.as_ref(), &mut serialized)
                    .expect("serializing an identifier into String cannot fail");
            }
            serialized.push(']');
            components.push(serialized);
        }
        if track_index < tracks.explicit_track_count {
            components.push(format_non_negative_used_css_px(f64::from(
                *tracks.used_track_sizes.get(size_index)?,
            )));
            size_index += 1;
        }
    }
    for _ in 0..tracks.positive_implicit_track_count {
        components.push(format_non_negative_used_css_px(f64::from(
            *tracks.used_track_sizes.get(size_index)?,
        )));
        size_index += 1;
    }
    (size_index == tracks.used_track_sizes.len()).then(|| components.join(" "))
}

fn computed_axis_position_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property_prefix: &str,
    context: StyleComputationContext,
) -> Option<String> {
    let axis_value = |property| {
        normalized_stylo_computed_style_value(runtime, handle, property, context)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| computed_style_default_value(runtime, handle, property))
    };
    let horizontal_property = format!("{property_prefix}position-x");
    let vertical_property = format!("{property_prefix}position-y");
    let horizontal = axis_value(&horizontal_property);
    let vertical = axis_value(&vertical_property);
    let horizontal_layers =
        top_level_comma_separated_component_values(&horizontal).unwrap_or_else(|| vec![horizontal]);
    let vertical_layers =
        top_level_comma_separated_component_values(&vertical).unwrap_or_else(|| vec![vertical]);
    if horizontal_layers.len() != vertical_layers.len() {
        return None;
    }
    Some(
        horizontal_layers
            .iter()
            .zip(vertical_layers.iter())
            .map(|(horizontal, vertical)| format!("{} {}", horizontal.trim(), vertical.trim()))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn resolve_computed_animation_duration(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
) -> String {
    if !value
        .split(',')
        .any(|component| component.trim().eq_ignore_ascii_case("auto"))
    {
        return value.to_owned();
    }
    let initial_timeline = animation_timeline_is_initial_auto(runtime, handle);
    value
        .split(',')
        .map(|component| {
            let component = component.trim();
            if initial_timeline && component.eq_ignore_ascii_case("auto") {
                "0s"
            } else {
                component
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn resolve_computed_zoom_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("normal") {
        return Some("1".to_owned());
    }
    let zoom = resolve_css_zoom_numeric(
        value,
        css_numeric_context_with_viewport_and_resolution(
            runtime,
            handle,
            resolution.computation.viewport(),
            resolution,
        ),
    )?;
    if zoom < 0.0 {
        return None;
    }
    if zoom == 0.0 {
        return Some("1".to_owned());
    }
    Some(format_css_number(zoom))
}

fn resolve_css_zoom_numeric(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<f64> {
    moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::LengthPercentage {
            basis: 1.0,
            unitless: moli_css_parse::UnitlessLength::Any,
        },
        context,
    )
    .and_then(moli_css_parse::CssNumericValue::px_length)
    .or_else(|| {
        moli_css_parse::resolve_css_numeric(value, moli_css_parse::CssNumericKind::Number, context)
            .and_then(moli_css_parse::CssNumericValue::number)
    })
}

fn animation_timeline_is_initial_auto(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "animation-timeline")
    else {
        return true;
    };
    let timelines = top_level_comma_separated_component_values(&entry.value)
        .unwrap_or_else(|| vec![entry.value]);
    matches!(
        timelines.as_slice(),
        [timeline] if timeline.trim().eq_ignore_ascii_case("auto")
    )
}

pub(in crate::native_bridge::element::styles) fn style_property_value_for_pseudo_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    pseudo_element: &str,
    property: &str,
    context: StyleComputationContext,
) -> String {
    if !computed_style_applies(runtime, handle) {
        return String::new();
    }
    let property = canonical_style_property_name(property);
    let value = normalized_stylo_computed_pseudo_style_value(
        runtime,
        handle,
        pseudo_element,
        &property,
        context,
    );
    match property.as_str() {
        "left" | "right" | "top" | "bottom" => value.unwrap_or_default(),
        "background-color" => value
            .map(|value| normalize_computed_color(&value))
            .unwrap_or_else(|| computed_style_default_value(runtime, handle, &property)),
        "accent-color" => value
            .map(|value| {
                if value.eq_ignore_ascii_case("auto") {
                    "auto".to_owned()
                } else {
                    normalize_computed_color(&value)
                }
            })
            .unwrap_or_else(|| computed_style_default_value(runtime, handle, &property)),
        "color" | "caret-color" | "outline-color" => value
            .map(|value| normalize_computed_color(&value))
            .unwrap_or_else(|| {
                inherited_computed_style_value(runtime, handle, &property, "rgb(0, 0, 0)")
            }),
        "width" => value
            .filter(|value| !value.is_empty())
            .map(|value| resolve_computed_pseudo_width(runtime, handle, &value))
            .unwrap_or_else(|| "auto".to_owned()),
        "height" | "min-width" | "min-height" => value
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "auto".to_owned()),
        "text-decoration-thickness" | "text-underline-offset" => value
            .filter(|value| !value.is_empty())
            .and_then(|value| {
                resolve_computed_text_decoration_length(runtime, handle, &property, &value)
            })
            .unwrap_or_else(|| computed_style_default_value(runtime, handle, &property)),
        "position" => value
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "static".to_owned()),
        "display" => value
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default_pseudo_display(runtime, handle, pseudo_element)),
        "content" => match value.as_deref() {
            Some("normal") | None => "none".to_owned(),
            Some(value) => value.to_owned(),
        },
        _ if color_property_is_resolved_color(&property) => value
            .map(|value| normalize_computed_color(&value))
            .unwrap_or_else(|| normalize_computed_color("currentcolor")),
        _ => value.unwrap_or_default(),
    }
}

fn resolve_computed_text_decoration_length(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto")
        || (property == "text-decoration-thickness" && value.eq_ignore_ascii_case("from-font"))
    {
        return Some(value.to_ascii_lowercase());
    }
    resolve_computed_font_relative_length(runtime, handle, value)
        .or_else(|| parse_css_px(value).map(format_css_px))
}

fn resolve_computed_pseudo_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
) -> String {
    if !value.contains('%') {
        return value.to_owned();
    }
    let Some(basis) = computed_pseudo_width_basis(runtime, handle) else {
        return value.to_owned();
    };
    moli_css_parse::resolve_length_percentage(
        value,
        basis,
        moli_css_parse::UnitlessLength::ZeroOnly,
    )
    .map(format_css_px)
    .unwrap_or_else(|| value.to_owned())
}

fn computed_pseudo_width_basis(runtime: &JsContextHost, handle: DomHandle) -> Option<f64> {
    let width = style_property_value(runtime, handle, StyleMode::Computed, "width");
    if let Some(px) = width.strip_suffix("px")
        && let Some(width) = moli_css_parse::parse_number(px)
    {
        return Some(width);
    }
    let parent = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)?;
    runtime.dom_host().node(parent).and_then(Node::as_element)?;
    computed_pseudo_width_basis(runtime, parent)
}

fn default_pseudo_display(
    runtime: &JsContextHost,
    handle: DomHandle,
    pseudo_element: &str,
) -> String {
    if matches!(pseudo_element, "before" | "after") {
        let parent_display = style_property_value(runtime, handle, StyleMode::Computed, "display");
        if matches!(
            parent_display.as_str(),
            "flex" | "inline-flex" | "grid" | "inline-grid"
        ) {
            return "block".to_owned();
        }
    }
    "inline".to_owned()
}

fn border_width_property_index(property: &str) -> Option<usize> {
    Some(match property {
        "border-width" => 4,
        "border-top-width" => 0,
        "border-right-width" => 1,
        "border-bottom-width" => 2,
        "border-left-width" => 3,
        _ => return None,
    })
}

fn border_width_longhands() -> &'static [&'static str] {
    &[
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
    ]
}

fn border_width_from_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    border_width_property_index(property)?;
    inline_style_entry_for_inline_style(runtime, handle, "border")
        .and_then(|entry| border_shorthand_width(&entry.value))
}

fn border_component_from_component_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    shorthand: &str,
    index: usize,
) -> Option<String> {
    inline_style_entry_for_inline_style(runtime, handle, shorthand)
        .and_then(|entry| box_shorthand_component(&entry.value, index))
}

fn border_component_style_entry_from_component_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    shorthand: &str,
    longhand: &str,
    index: usize,
) -> Option<StyleEntry> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, shorthand)?;
    let value = box_shorthand_component(&entry.value, index)?;
    Some(StyleEntry {
        name: longhand.to_owned(),
        value,
        priority: entry.priority,
    })
}

fn border_color_property(property: &str) -> bool {
    matches!(
        property,
        "border-color"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "border-block-start-color"
            | "border-block-end-color"
            | "border-inline-start-color"
            | "border-inline-end-color"
    )
}

fn border_color_property_index(property: &str) -> Option<usize> {
    Some(match property {
        "border-top-color" => 0,
        "border-right-color" => 1,
        "border-bottom-color" => 2,
        "border-left-color" => 3,
        _ => return None,
    })
}

fn border_color_longhands() -> &'static [&'static str] {
    &[
        "border-top-color",
        "border-right-color",
        "border-bottom-color",
        "border-left-color",
    ]
}

fn border_color_from_shorthand(runtime: &JsContextHost, handle: DomHandle) -> Option<String> {
    inline_style_entry_for_inline_style(runtime, handle, "border")
        .and_then(|entry| border_shorthand_color(&entry.value))
}

fn border_style_property(property: &str) -> bool {
    matches!(
        property,
        "border-style"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
    )
}

fn border_style_property_index(property: &str) -> Option<usize> {
    Some(match property {
        "border-top-style" => 0,
        "border-right-style" => 1,
        "border-bottom-style" => 2,
        "border-left-style" => 3,
        _ => return None,
    })
}

fn border_style_longhands() -> &'static [&'static str] {
    &[
        "border-top-style",
        "border-right-style",
        "border-bottom-style",
        "border-left-style",
    ]
}

fn border_style_from_shorthand(runtime: &JsContextHost, handle: DomHandle) -> Option<String> {
    inline_style_entry_for_inline_style(runtime, handle, "border")
        .and_then(|entry| border_shorthand_style(&entry.value))
}

fn color_property_is_resolved_color(property: &str) -> bool {
    property == "color"
        || property == "accent-color"
        || property == "background-color"
        || property == "caret-color"
        || property == "outline-color"
        || property == "text-decoration-color"
        || property == "text-emphasis-color"
        || property == "-webkit-text-fill-color"
        || property == "-webkit-text-stroke-color"
        || border_color_property(property)
}

fn resolve_computed_inset(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if let Some(length) = resolve_computed_font_relative_length(runtime, handle, value) {
        return Some(length);
    }
    if let Some(length) =
        resolve_computed_inset_length_percentage(runtime, handle, property, value, resolution)
    {
        return Some(length);
    }
    resolve_computed_auto_inset(runtime, handle, property, value, resolution)
}

fn resolve_computed_auto_min_size(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    if display_none_ancestor(runtime, handle, resolution) {
        return "0px".to_owned();
    }
    let aspect_ratio = resolution.raw_property(runtime, handle, "aspect-ratio");
    if !aspect_ratio.is_empty() && aspect_ratio != "auto" {
        return "auto".to_owned();
    }
    if let Some(parent) = flat_tree_element_parent(runtime, handle) {
        let parent_display = resolution.computed_property(runtime, parent, "display");
        if matches!(
            parent_display.as_str(),
            "flex" | "inline-flex" | "grid" | "inline-grid"
        ) {
            return "auto".to_owned();
        }
    }
    "0px".to_owned()
}

fn display_none_ancestor(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> bool {
    let mut current = Some(handle);
    while let Some(candidate) = current {
        if resolution.computed_property(runtime, candidate, "display") == "none" {
            return true;
        }
        current = flat_tree_element_parent(runtime, candidate);
    }
    false
}

fn resolve_computed_font_relative_length(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    let (raw_number, font_size) = if let Some(number) = value.strip_suffix("rem") {
        (
            number,
            runtime
                .dom_host()
                .document_element_handle()
                .and_then(|document_element| computed_font_size_px(runtime, document_element))
                .unwrap_or(16.0),
        )
    } else if let Some(number) = value.strip_suffix("em") {
        (
            number,
            computed_font_size_px(runtime, handle).unwrap_or(16.0),
        )
    } else {
        return None;
    };
    let multiplier = moli_css_parse::parse_number(raw_number)?;
    Some(format_css_px(multiplier * font_size))
}

fn resolve_computed_inset_length_percentage(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let value = value.trim();
    if !value.contains('%') && !value.to_ascii_lowercase().starts_with("calc(") {
        return None;
    }
    let position = computed_position_with_resolution(runtime, handle, resolution);
    let basis = match position.as_str() {
        "relative" | "sticky" | "absolute" | "fixed" => {
            computed_inset_containing_block_size(runtime, handle, &position, property)?
        }
        _ => return None,
    };
    let resolved = moli_css_parse::resolve_length_percentage(
        value,
        basis,
        moli_css_parse::UnitlessLength::ZeroOnly,
    )?;
    Some(format_css_px(resolved))
}

fn computed_position(runtime: &JsContextHost, handle: DomHandle) -> String {
    let read = ComputedStyleRead::new(runtime, handle);
    computed_position_with_resolution(runtime, handle, read.resolution_context())
}

fn computed_position_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    let position = resolution.computed_property(runtime, handle, "position");
    if position.is_empty() {
        "static".to_owned()
    } else {
        position
    }
}

fn logical_inset_style_entry(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<StyleEntry> {
    let logical = physical_inset_logical_source(runtime, handle, property)?;
    inline_style_entry_for_inline_style(runtime, handle, logical)
}

fn inset_shorthand_style_entry(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<StyleEntry> {
    let shorthand_index = match property {
        "top" => 0,
        "right" => 1,
        "bottom" => 2,
        "left" => 3,
        _ => return None,
    };
    let mut entry = inline_style_entry_for_inline_style(runtime, handle, "inset")?;
    entry.name = property.to_owned();
    entry.value = box_shorthand_component(&entry.value, shorthand_index)?;
    Some(entry)
}

fn physical_inset_logical_source(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<&'static str> {
    if !matches!(property, "left" | "right") {
        return None;
    }
    let writing_mode = raw_stylo_computed_style_value(runtime, handle, "writing-mode");
    if !writing_mode.is_empty() && writing_mode != "horizontal-tb" {
        return None;
    }
    let direction = computed_direction(runtime, handle);
    match (property, direction.as_str()) {
        ("left", "rtl") | ("right", "ltr") => Some("inset-inline-end"),
        ("left", _) | ("right", _) => Some("inset-inline-start"),
        _ => None,
    }
}

fn computed_direction(runtime: &JsContextHost, handle: DomHandle) -> String {
    let read = ComputedStyleRead::new(runtime, handle);
    computed_direction_with_resolution(runtime, handle, read.resolution_context())
}

fn computed_direction_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "direction") {
        let value = entry.value.to_ascii_lowercase();
        if matches!(value.as_str(), "ltr" | "rtl") {
            return value;
        }
    }
    let direction = resolution.raw_property(runtime, handle, "direction");
    if direction.eq_ignore_ascii_case("rtl") {
        return "rtl".to_owned();
    }
    html_directionality(runtime.dom_host(), handle)
        .as_str()
        .to_owned()
}

fn raw_stylo_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> String {
    ComputedStyleRead::new(runtime, handle)
        .raw_primary_property(property)
        .unwrap_or_default()
}

fn raw_stylo_computed_style_value_with_inputs(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    inputs: &FullStyleWorldSnapshot,
    context: StyleComputationContext,
) -> String {
    let read_document = context.resolved_read_document(runtime, handle);
    runtime
        .computed_style_snapshot_from_stylo_after_style_update(
            handle,
            inputs,
            read_document,
            context.viewport,
        )
        .and_then(|style| style.property_value(property))
        .unwrap_or_default()
}

fn raw_stylo_computed_style_value_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
) -> Option<String> {
    ComputedStyleRead::new_with_context(runtime, handle, context).raw_primary_property(property)
}

fn computed_inset_containing_block_size(
    runtime: &JsContextHost,
    handle: DomHandle,
    position: &str,
    property: &str,
) -> Option<f64> {
    let containing_block = computed_inset_containing_block(runtime, handle, position)?;
    let rect = computed_style_geometry_rect(runtime, containing_block)?;
    let vertical = matches!(property, "top" | "bottom");
    let mut size = if vertical { rect.height } else { rect.width };
    if position == "absolute"
        && !vertical
        && let Some(inline_width) = inline_text_containing_block_width(runtime, containing_block)
        && inline_width > size
    {
        size = inline_width;
    }
    if matches!(position, "relative" | "sticky") {
        size -= computed_box_component_px(
            runtime,
            containing_block,
            if vertical {
                "padding-top"
            } else {
                "padding-left"
            },
            "padding",
            if vertical { 0 } else { 3 },
        );
        size -= computed_box_component_px(
            runtime,
            containing_block,
            if vertical {
                "padding-bottom"
            } else {
                "padding-right"
            },
            "padding",
            if vertical { 2 } else { 1 },
        );
    }
    Some(size)
}

fn inline_text_containing_block_width(
    runtime: &JsContextHost,
    containing_block: DomHandle,
) -> Option<f64> {
    let display = style_property_value(runtime, containing_block, StyleMode::Computed, "display");
    if display != "inline" {
        return None;
    }
    let text = runtime.dom_host().text_content(containing_block)?;
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let font_size = computed_font_size_px(runtime, containing_block).unwrap_or(16.0);
    Some((collapsed.chars().count() as f64 + 1.0) * font_size)
}

fn computed_inset_containing_block(
    runtime: &JsContextHost,
    handle: DomHandle,
    position: &str,
) -> Option<DomHandle> {
    match position {
        "absolute" => flat_ancestor_chain(runtime, handle)
            .into_iter()
            .find(|ancestor| {
                !matches!(
                    computed_position(runtime, *ancestor).as_str(),
                    "static" | ""
                )
            }),
        "fixed" => flat_ancestor_chain(runtime, handle)
            .into_iter()
            .find(|ancestor| {
                let transform =
                    style_property_value(runtime, *ancestor, StyleMode::Computed, "transform");
                !transform.is_empty() && transform != "none"
            }),
        "sticky" => flat_ancestor_chain(runtime, handle)
            .into_iter()
            .find(|ancestor| {
                let overflow =
                    style_property_value(runtime, *ancestor, StyleMode::Computed, "overflow");
                !matches!(overflow.as_str(), "" | "visible" | "clip")
            })
            .or_else(|| runtime.dom_host().node(handle).and_then(Node::parent_node)),
        _ => runtime.dom_host().node(handle).and_then(Node::parent_node),
    }
}

fn flat_ancestor_chain(runtime: &JsContextHost, handle: DomHandle) -> Vec<DomHandle> {
    let mut ancestors = Vec::new();
    let mut current = runtime.dom_host().node(handle).and_then(Node::parent_node);
    while let Some(handle) = current {
        ancestors.push(handle);
        current = runtime.dom_host().node(handle).and_then(Node::parent_node);
    }
    ancestors
}

fn computed_box_component_px(
    runtime: &JsContextHost,
    handle: DomHandle,
    longhand: &str,
    shorthand: &str,
    shorthand_index: usize,
) -> f64 {
    parse_css_px(&style_property_value(
        runtime,
        handle,
        StyleMode::Computed,
        longhand,
    ))
    .or_else(|| {
        let shorthand = style_property_value(runtime, handle, StyleMode::Computed, shorthand);
        box_shorthand_component(&shorthand, shorthand_index).and_then(|value| parse_css_px(&value))
    })
    .unwrap_or(0.0)
}

fn resolve_computed_auto_inset(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if !value.is_empty() && !value.eq_ignore_ascii_case("auto") {
        return None;
    }
    let position = computed_position_with_resolution(runtime, handle, resolution);
    if !matches!(position.as_str(), "relative" | "absolute" | "fixed") {
        return None;
    }
    if (value.is_empty() || value.eq_ignore_ascii_case("auto"))
        && position == "absolute"
        && property == "left"
        && (computed_inset_containing_block(runtime, handle, &position).is_some_and(|container| {
            style_property_value(runtime, container, StyleMode::Computed, "display") == "grid"
        }) || grid_column_is_positioned(runtime, handle))
    {
        return Some("0px".to_owned());
    }
    let opposite = opposite_inset_property(property)?;
    let opposite_value = resolution.raw_property(runtime, handle, opposite);
    if value.is_empty()
        && position != "relative"
        && (opposite_value.is_empty() || opposite_value.eq_ignore_ascii_case("auto"))
    {
        return None;
    }
    if opposite_value.is_empty() || opposite_value.eq_ignore_ascii_case("auto") {
        return Some(resolve_computed_both_auto_inset(
            runtime, handle, property, &position,
        ));
    }
    let opposite = parse_css_px(&opposite_value)
        .or_else(|| {
            resolve_computed_font_relative_length(runtime, handle, &opposite_value)
                .and_then(|value| parse_css_px(&value))
        })
        .or_else(|| {
            resolve_computed_inset_length_percentage(
                runtime,
                handle,
                opposite,
                &opposite_value,
                resolution,
            )
            .and_then(|value| parse_css_px(&value))
        })?;
    if position == "relative" {
        return Some(format_css_px(-opposite));
    }
    let containing_block_size =
        computed_inset_containing_block_size(runtime, handle, &position, property)?;
    let own_size = computed_auto_inset_own_size(runtime, handle, property).unwrap_or(0.0);
    Some(format_css_px(containing_block_size - opposite - own_size))
}

fn grid_column_is_positioned(runtime: &JsContextHost, handle: DomHandle) -> bool {
    ["grid-column-start", "grid-column-end"]
        .into_iter()
        .map(|property| raw_stylo_computed_style_value(runtime, handle, property))
        .any(|value| !value.is_empty() && value != "auto")
}

fn opposite_inset_property(property: &str) -> Option<&'static str> {
    match property {
        "top" => Some("bottom"),
        "right" => Some("left"),
        "bottom" => Some("top"),
        "left" => Some("right"),
        _ => None,
    }
}

fn resolve_computed_both_auto_inset(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    position: &str,
) -> String {
    if position == "relative" {
        return "0px".to_owned();
    }
    let Some(containing_block) = computed_inset_containing_block(runtime, handle, position) else {
        return "0px".to_owned();
    };
    let Some(rect) = computed_style_geometry_rect(runtime, containing_block) else {
        return "0px".to_owned();
    };
    let containing_block_size = if matches!(property, "top" | "bottom") {
        rect.height
    } else {
        rect.width
    };
    let static_offset = static_position_offset(runtime, containing_block, property, position);
    if inset_uses_start_static_position(runtime, containing_block, property) {
        format_css_px(static_offset)
    } else {
        format_css_px(containing_block_size - static_offset)
    }
}

fn static_position_offset(
    runtime: &JsContextHost,
    containing_block: DomHandle,
    property: &str,
    position: &str,
) -> f64 {
    if let Some(offset) = runtime_static_position_offset(runtime, containing_block, property) {
        return offset;
    }
    match (position, matches!(property, "top" | "bottom")) {
        ("fixed", _) => 0.0,
        (_, true) => 15.0,
        (_, false) => 30.0,
    }
}

fn runtime_static_position_offset(
    runtime: &JsContextHost,
    containing_block: DomHandle,
    property: &str,
) -> Option<f64> {
    let parent = runtime
        .dom_host()
        .node(containing_block)
        .and_then(Node::parent_node)?;
    let rects = observable_bounding_client_rects(
        runtime,
        &[parent, containing_block],
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )
    .ok()?;
    let [parent_rect, containing_rect] = rects.as_slice() else {
        return None;
    };
    Some(if matches!(property, "top" | "bottom") {
        (containing_rect.top - parent_rect.top).abs()
    } else {
        (containing_rect.left - parent_rect.left).abs()
    })
    .filter(|offset| *offset > 0.0)
}

fn computed_style_geometry_rect(runtime: &JsContextHost, handle: DomHandle) -> Option<ClientRect> {
    observable_bounding_client_rect(
        runtime,
        handle,
        moli_layout::LayoutFlushReason::SynchronousGeometry,
    )
    .ok()
}

fn inset_uses_start_static_position(
    runtime: &JsContextHost,
    containing_block: DomHandle,
    property: &str,
) -> bool {
    let writing_mode = style_property_value(
        runtime,
        containing_block,
        StyleMode::Computed,
        "writing-mode",
    );
    let direction = computed_direction(runtime, containing_block);
    let writing_mode = if writing_mode.is_empty() {
        "horizontal-tb"
    } else {
        writing_mode.as_str()
    };
    let direction = if direction == "rtl" { "rtl" } else { "ltr" };
    match (writing_mode, direction, property) {
        ("horizontal-tb", _, "top") => true,
        ("horizontal-tb", _, "bottom") => false,
        ("horizontal-tb", "ltr", "left") => true,
        ("horizontal-tb", "ltr", "right") => false,
        ("horizontal-tb", "rtl", "left") => false,
        ("horizontal-tb", "rtl", "right") => true,
        ("vertical-lr", _, "left") => true,
        ("vertical-lr", _, "right") => false,
        ("vertical-rl", _, "left") => false,
        ("vertical-rl", _, "right") => true,
        ("vertical-lr" | "vertical-rl", "ltr", "top") => true,
        ("vertical-lr" | "vertical-rl", "ltr", "bottom") => false,
        ("vertical-lr" | "vertical-rl", "rtl", "top") => false,
        ("vertical-lr" | "vertical-rl", "rtl", "bottom") => true,
        (_, _, "top" | "left") => true,
        _ => false,
    }
}

fn computed_auto_inset_own_size(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<f64> {
    let size_property = if matches!(property, "top" | "bottom") {
        "height"
    } else {
        "width"
    };
    let value = style_property_value(runtime, handle, StyleMode::Computed, size_property);
    parse_css_px(&value).or_else(|| {
        resolve_computed_font_relative_length(runtime, handle, &value)
            .and_then(|value| parse_css_px(&value))
    })
}

fn resolve_computed_horizontal_auto_margin(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if !value.eq_ignore_ascii_case("auto") {
        return None;
    }
    let parent_width =
        containing_block_width_with_resolution(runtime, handle, context, resolution, 0)?;
    let own_width = parse_css_px(&resolution.computed_property(runtime, handle, "width"))?;
    let other_property = if property == "margin-left" {
        "margin-right"
    } else {
        "margin-left"
    };
    let other = resolution.raw_property(runtime, handle, other_property);
    let other_margin = if other.eq_ignore_ascii_case("auto") {
        None
    } else {
        parse_css_px(&other).or(Some(0.0))
    };
    let available = parent_width - own_width - other_margin.unwrap_or(0.0);
    let resolved = if other_margin.is_none() {
        available / 2.0
    } else {
        available
    };
    Some(format_css_px(resolved))
}

fn resolve_computed_horizontal_margin(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if value.eq_ignore_ascii_case("auto") {
        return None;
    }
    let parent_width =
        containing_block_width_with_resolution(runtime, handle, context, resolution, 0)?;
    let resolved = resolve_length_percentage_with_context(
        value,
        parent_width,
        css_numeric_context_with_viewport_and_resolution(
            runtime,
            handle,
            context.viewport(),
            resolution,
        ),
    )?;
    Some(format_css_px(resolved))
}

fn resolve_computed_horizontal_margin_with_inline_fallback(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if computed_length_percentage_value_needs_moli_context(value) {
        inline_style_entry_for_inline_style(runtime, handle, property)
            .and_then(|entry| {
                resolve_computed_horizontal_margin(
                    runtime,
                    handle,
                    &entry.value,
                    context,
                    resolution,
                )
            })
            .or_else(|| {
                resolve_computed_horizontal_margin(runtime, handle, value, context, resolution)
            })
    } else {
        resolve_computed_horizontal_margin(runtime, handle, value, context, resolution).or_else(
            || {
                inline_style_entry_for_inline_style(runtime, handle, property).and_then(|entry| {
                    resolve_computed_horizontal_margin(
                        runtime,
                        handle,
                        &entry.value,
                        context,
                        resolution,
                    )
                })
            },
        )
    }
}

fn resolve_computed_line_height_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("normal") || parse_css_px(value).is_some() {
        return None;
    }
    let font_size =
        computed_font_size_px_with_resolution(runtime, handle, resolution).unwrap_or(16.0);
    if let Some(percent) = parse_css_percent(value) {
        return Some(format_css_px(font_size * percent / 100.0));
    }
    let multiplier = moli_css_parse::parse_number(value)?;
    Some(format_css_px(font_size * multiplier))
}

fn computed_line_height_px_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    let value = resolution.raw_property(runtime, handle, "line-height");
    if let Some(px) = parse_css_px(&value) {
        return Some(px);
    }
    resolve_computed_line_height_with_resolution(runtime, handle, &value, resolution)
        .and_then(|value| parse_css_px(&value))
}

fn computed_font_size_px(runtime: &JsContextHost, handle: DomHandle) -> Option<f64> {
    let read = ComputedStyleRead::new(runtime, handle);
    computed_font_size_px_with_resolution(runtime, handle, read.resolution_context())
}

fn inline_font_size_px(runtime: &JsContextHost, handle: DomHandle) -> Option<f64> {
    parse_css_px(&style_property_value(
        runtime,
        handle,
        StyleMode::Inline,
        "font-size",
    ))
}

fn computed_font_size_px_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    computed_font_size_px_with_resolution_and_depth(runtime, handle, resolution, 0)
}

fn computed_font_size_px_with_resolution_and_depth(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
    depth: usize,
) -> Option<f64> {
    if depth > 32 {
        return None;
    }
    parse_css_px(&resolution.computed_property(runtime, handle, "font-size")).or_else(|| {
        inherited_style_parent(runtime, handle)
            .filter(|parent| *parent != handle)
            .and_then(|parent| {
                computed_font_size_px_with_resolution_and_depth(
                    runtime,
                    parent,
                    resolution,
                    depth + 1,
                )
            })
    })
}

fn resolve_computed_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if element_has_no_used_width(runtime, handle, resolution) {
        return None;
    }
    let viewport = context.viewport();
    let parent_width =
        containing_block_width_with_resolution(runtime, handle, context, resolution, 0)?;
    let resolved = resolve_length_percentage_with_context(
        value,
        parent_width,
        css_numeric_context_with_viewport_and_resolution(runtime, handle, viewport, resolution),
    )?;
    Some(format_non_negative_used_css_px(resolved))
}

fn resolve_computed_width_with_inline_fallback(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if computed_length_percentage_value_needs_moli_context(value) {
        inline_style_entry_for_inline_style(runtime, handle, "width")
            .and_then(|entry| {
                resolve_computed_width(runtime, handle, &entry.value, context, resolution)
            })
            .or_else(|| resolve_computed_width(runtime, handle, value, context, resolution))
    } else {
        resolve_computed_width(runtime, handle, value, context, resolution).or_else(|| {
            inline_style_entry_for_inline_style(runtime, handle, "width").and_then(|entry| {
                resolve_computed_width(runtime, handle, &entry.value, context, resolution)
            })
        })
    }
}

fn element_has_no_used_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> bool {
    matches!(
        resolution
            .computed_property(runtime, handle, "display")
            .as_str(),
        "none" | "contents" | "inline"
    )
}

fn element_has_no_used_height(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> bool {
    matches!(
        resolution
            .computed_property(runtime, handle, "display")
            .as_str(),
        "none" | "contents" | "inline"
    )
}

fn containing_block_width_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
    depth: usize,
) -> Option<f64> {
    if depth > 32 {
        return None;
    }
    let parent = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)?;
    if let Some(width) = raw_computed_width_px(runtime, parent, resolution) {
        return Some(width);
    }
    let parent_is_viewport_box = runtime.dom_host().node(parent).is_some_and(|node| {
        node.is_html_element_named("body") || node.is_html_element_named("html")
    });
    if let Some(percent) = raw_computed_width_percent(runtime, parent, resolution) {
        let viewport_width = context.viewport_width();
        let parent_parent_width = if parent_is_viewport_box {
            viewport_width
                .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_width)
        } else {
            containing_block_width_with_resolution(runtime, parent, context, resolution, depth + 1)?
        };
        return Some(parent_parent_width * percent / 100.0);
    }
    if parent_is_viewport_box {
        return Some(
            context
                .viewport_width()
                .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_width),
        );
    }
    containing_block_width_with_resolution(runtime, parent, context, resolution, depth + 1)
}

fn resolve_computed_height(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if element_has_no_used_height(runtime, handle, resolution) {
        return None;
    }
    if value.eq_ignore_ascii_case("auto") {
        return None;
    }
    let viewport = context.viewport();
    let parent_height = containing_block_height(runtime, handle, context, resolution, 0)?;
    let resolved = resolve_length_percentage_with_context(
        value,
        parent_height,
        css_numeric_context_with_viewport_and_resolution(runtime, handle, viewport, resolution),
    )?;
    Some(format_non_negative_used_css_px(resolved))
}

fn resolve_computed_height_with_inline_fallback(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if computed_length_percentage_value_needs_moli_context(value) {
        inline_style_entry_for_inline_style(runtime, handle, "height")
            .and_then(|entry| {
                resolve_computed_height(runtime, handle, &entry.value, context, resolution)
            })
            .or_else(|| resolve_computed_height(runtime, handle, value, context, resolution))
    } else {
        resolve_computed_height(runtime, handle, value, context, resolution).or_else(|| {
            inline_style_entry_for_inline_style(runtime, handle, "height").and_then(|entry| {
                resolve_computed_height(runtime, handle, &entry.value, context, resolution)
            })
        })
    }
}

fn computed_length_percentage_value_needs_moli_context(value: &str) -> bool {
    value.contains('%')
}

fn resolve_length_percentage_with_context(
    value: &str,
    basis: f64,
    context: moli_css_parse::CssNumericContext,
) -> Option<f64> {
    moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::LengthPercentage {
            basis,
            unitless: moli_css_parse::UnitlessLength::ZeroOnly,
        },
        context,
    )?
    .px_length()
}

fn containing_block_height(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
    depth: usize,
) -> Option<f64> {
    if depth > 32 {
        return None;
    }
    let parent = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)?;
    if let Some(height) = raw_computed_height_px(runtime, parent, resolution) {
        return Some(height);
    }
    let parent_is_viewport_box = runtime.dom_host().node(parent).is_some_and(|node| {
        node.is_html_element_named("body") || node.is_html_element_named("html")
    });
    if let Some(percent) = raw_computed_height_percent(runtime, parent, resolution) {
        let viewport_height = context
            .viewport()
            .height
            .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_height);
        let parent_parent_height = if parent_is_viewport_box {
            viewport_height
        } else {
            containing_block_height(runtime, parent, context, resolution, depth + 1)?
        };
        return Some(parent_parent_height * percent / 100.0);
    }
    if parent_is_viewport_box {
        return Some(
            context
                .viewport()
                .height
                .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_height),
        );
    }
    containing_block_height(runtime, parent, context, resolution, depth + 1)
}

fn raw_computed_width_px(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    parse_css_px(&resolution.raw_property(runtime, handle, "width"))
}

fn raw_computed_width_percent(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    parse_css_percent(&resolution.raw_property(runtime, handle, "width"))
}

fn raw_computed_height_px(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    parse_css_px(&resolution.raw_property(runtime, handle, "height"))
}

fn raw_computed_height_percent(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    parse_css_percent(&resolution.raw_property(runtime, handle, "height"))
}

fn parse_css_px(value: &str) -> Option<f64> {
    let value = value.trim();
    if value == "0" {
        return Some(0.0);
    }
    value
        .strip_suffix("px")?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_css_percent(value: &str) -> Option<f64> {
    value
        .trim()
        .strip_suffix('%')?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn format_css_px(value: f64) -> String {
    format!("{}px", format_css_numeric_literal(value))
}

/// Matches Blink's `CSSNumericLiteralValue` serialization, which uses `%g`
/// with six significant digits for finite non-integer dimensions.
fn format_css_numeric_literal(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if !value.is_finite() {
        return value.to_string();
    }

    // Formatting in scientific notation first gives us both the six-digit
    // rounding and the post-rounding exponent without reimplementing floating
    // point decimal conversion.
    let scientific = format!("{value:.5e}");
    let (mantissa, exponent) = scientific
        .rsplit_once('e')
        .expect("finite f64 scientific formatting must contain an exponent");
    let exponent = exponent
        .parse::<i32>()
        .expect("f64 scientific formatting must contain a decimal exponent");
    if (-4..6).contains(&exponent) {
        let fractional_digits = usize::try_from((5 - exponent).max(0)).unwrap_or(0);
        let mut serialized = format!("{value:.fractional_digits$}");
        trim_decimal_zeros(&mut serialized);
        return serialized;
    }

    let mut mantissa = mantissa.to_owned();
    trim_decimal_zeros(&mut mantissa);
    format!("{mantissa}e{exponent:+03}")
}

fn trim_decimal_zeros(serialized: &mut String) {
    if !serialized.contains('.') {
        return;
    }
    while serialized.ends_with('0') {
        serialized.pop();
    }
    if serialized.ends_with('.') {
        serialized.pop();
    }
}

fn format_non_negative_used_css_px(value: f64) -> String {
    if value.is_nan() || value.is_sign_negative() {
        return "0px".to_owned();
    }
    if value == f64::INFINITY {
        return format!("{}px", i64::MAX);
    }
    format_css_px(value)
}

fn resolve_computed_background_image(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
) -> String {
    resolve_background_image_url(value, &style_base_url(runtime, handle))
}

fn resolve_background_image_url(value: &str, base_url: &url::Url) -> String {
    resolve_css_url_function(value, base_url)
}

fn compress_box_shorthand_value(value: &str) -> String {
    box_shorthand_value_components(value)
        .and_then(|values| compress_box_components(&values))
        .unwrap_or_else(|| value.to_owned())
}

fn compress_box_components(values: &[String]) -> Option<String> {
    match values {
        [start, end] if start == end => Some(start.clone()),
        [start, end] => Some(format!("{start} {end}")),
        [top, right, bottom, left] if top == right && top == bottom && top == left => {
            Some(top.clone())
        }
        [top, right, bottom, left] if top == bottom && right == left => {
            Some(format!("{top} {right}"))
        }
        [top, right, bottom, left] if right == left => Some(format!("{top} {right} {bottom}")),
        [top, right, bottom, left] => Some(format!("{top} {right} {bottom} {left}")),
        _ => None,
    }
}

fn normalize_computed_color(value: &str) -> String {
    let value = value.trim();
    if let Some((red, green, blue)) = system_color_rgb(value) {
        return format!("rgb({red}, {green}, {blue})");
    }
    if let Some((red, green, blue)) = css_named_color_rgb(value) {
        return format!("rgb({red}, {green}, {blue})");
    }
    if value.eq_ignore_ascii_case("currentcolor") {
        return "rgb(0, 0, 0)".to_owned();
    }
    if value.eq_ignore_ascii_case("transparent") {
        return "rgba(0, 0, 0, 0)".to_owned();
    }
    if let Some((red, green, blue)) = css_hex_color_rgb(value) {
        return format!("rgb({red}, {green}, {blue})");
    }
    value.to_owned()
}

fn resolve_computed_color_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> String {
    if value.eq_ignore_ascii_case("currentcolor") && property != "color" {
        return resolution.computed_property(runtime, handle, "color");
    }
    normalize_computed_color(value)
}

fn normalize_computed_color_functions(value: &str, current_color: Option<&str>) -> String {
    top_level_comma_separated_component_values(value)
        .map(|layers| {
            layers
                .into_iter()
                .map(|layer| normalize_computed_color_function_layer(&layer, current_color))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| normalize_computed_color_function_layer(value, current_color))
}

fn normalize_computed_color_function_layer(value: &str, current_color: Option<&str>) -> String {
    let Some(components) = box_shorthand_value_components(value) else {
        return value.to_owned();
    };
    let mut color_component_index = None;
    let mut normalized = components
        .into_iter()
        .enumerate()
        .map(|(index, component)| {
            system_color_rgb(&component)
                .map(|(red, green, blue)| {
                    color_component_index.get_or_insert(index);
                    format!("rgb({red}, {green}, {blue})")
                })
                .or_else(|| {
                    component.eq_ignore_ascii_case("currentcolor").then(|| {
                        color_component_index.get_or_insert(index);
                        current_color.unwrap_or(&component).to_owned()
                    })
                })
                .unwrap_or(component)
        })
        .collect::<Vec<_>>();
    if let Some(index) = color_component_index
        && index > 0
        && index < normalized.len()
    {
        let color = normalized.remove(index);
        normalized.insert(0, color);
    }
    normalized.join(" ")
}

fn css_hex_color_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?;
    if !hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    match hex.len() {
        3 => {
            let mut chars = hex.chars();
            let red = chars.next()?.to_digit(16)? as u8;
            let green = chars.next()?.to_digit(16)? as u8;
            let blue = chars.next()?.to_digit(16)? as u8;
            Some((red * 17, green * 17, blue * 17))
        }
        6 => {
            let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((red, green, blue))
        }
        _ => None,
    }
}

fn css_named_color_rgb(value: &str) -> Option<(u8, u8, u8)> {
    Some(match value.to_ascii_lowercase().as_str() {
        "aliceblue" => (240, 248, 255),
        "antiquewhite" => (250, 235, 215),
        "aqua" => (0, 255, 255),
        "aquamarine" => (127, 255, 212),
        "azure" => (240, 255, 255),
        "beige" => (245, 245, 220),
        "bisque" => (255, 228, 196),
        "black" => (0, 0, 0),
        "blanchedalmond" => (255, 235, 205),
        "blue" => (0, 0, 255),
        "blueviolet" => (138, 43, 226),
        "brown" => (165, 42, 42),
        "burlywood" => (222, 184, 135),
        "cadetblue" => (95, 158, 160),
        "chartreuse" => (127, 255, 0),
        "chocolate" => (210, 105, 30),
        "coral" => (255, 127, 80),
        "cornflowerblue" => (100, 149, 237),
        "cornsilk" => (255, 248, 220),
        "crimson" => (220, 20, 60),
        "cyan" => (0, 255, 255),
        "darkblue" => (0, 0, 139),
        "darkcyan" => (0, 139, 139),
        "darkgoldenrod" => (184, 134, 11),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "darkgreen" => (0, 100, 0),
        "darkkhaki" => (189, 183, 107),
        "darkmagenta" => (139, 0, 139),
        "darkolivegreen" => (85, 107, 47),
        "darkorange" => (255, 140, 0),
        "darkorchid" => (153, 50, 204),
        "darkred" => (139, 0, 0),
        "darksalmon" => (233, 150, 122),
        "darkseagreen" => (143, 188, 143),
        "darkslateblue" => (72, 61, 139),
        "darkslategray" | "darkslategrey" => (47, 79, 79),
        "darkturquoise" => (0, 206, 209),
        "darkviolet" => (148, 0, 211),
        "deeppink" => (255, 20, 147),
        "deepskyblue" => (0, 191, 255),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "dodgerblue" => (30, 144, 255),
        "firebrick" => (178, 34, 34),
        "floralwhite" => (255, 250, 240),
        "forestgreen" => (34, 139, 34),
        "fuchsia" => (255, 0, 255),
        "gainsboro" => (220, 220, 220),
        "ghostwhite" => (248, 248, 255),
        "gold" => (255, 215, 0),
        "goldenrod" => (218, 165, 32),
        "gray" | "grey" => (128, 128, 128),
        "green" => (0, 128, 0),
        "greenyellow" => (173, 255, 47),
        "honeydew" => (240, 255, 240),
        "hotpink" => (255, 105, 180),
        "indianred" => (205, 92, 92),
        "indigo" => (75, 0, 130),
        "ivory" => (255, 255, 240),
        "khaki" => (240, 230, 140),
        "lavender" => (230, 230, 250),
        "lavenderblush" => (255, 240, 245),
        "lawngreen" => (124, 252, 0),
        "lemonchiffon" => (255, 250, 205),
        "lightblue" => (173, 216, 230),
        "lightcoral" => (240, 128, 128),
        "lightcyan" => (224, 255, 255),
        "lightgoldenrodyellow" => (250, 250, 210),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "lightgreen" => (144, 238, 144),
        "lightpink" => (255, 182, 193),
        "lightsalmon" => (255, 160, 122),
        "lightseagreen" => (32, 178, 170),
        "lightskyblue" => (135, 206, 250),
        "lightslategray" | "lightslategrey" => (119, 136, 153),
        "lightsteelblue" => (176, 196, 222),
        "lightyellow" => (255, 255, 224),
        "lime" => (0, 255, 0),
        "limegreen" => (50, 205, 50),
        "linen" => (250, 240, 230),
        "magenta" => (255, 0, 255),
        "maroon" => (128, 0, 0),
        "mediumaquamarine" => (102, 205, 170),
        "mediumblue" => (0, 0, 205),
        "mediumorchid" => (186, 85, 211),
        "mediumpurple" => (147, 112, 219),
        "mediumseagreen" => (60, 179, 113),
        "mediumslateblue" => (123, 104, 238),
        "mediumspringgreen" => (0, 250, 154),
        "mediumturquoise" => (72, 209, 204),
        "mediumvioletred" => (199, 21, 133),
        "midnightblue" => (25, 25, 112),
        "mintcream" => (245, 255, 250),
        "mistyrose" => (255, 228, 225),
        "moccasin" => (255, 228, 181),
        "navajowhite" => (255, 222, 173),
        "navy" => (0, 0, 128),
        "oldlace" => (253, 245, 230),
        "olive" => (128, 128, 0),
        "olivedrab" => (107, 142, 35),
        "orange" => (255, 165, 0),
        "orangered" => (255, 69, 0),
        "orchid" => (218, 112, 214),
        "palegoldenrod" => (238, 232, 170),
        "palegreen" => (152, 251, 152),
        "paleturquoise" => (175, 238, 238),
        "palevioletred" => (219, 112, 147),
        "papayawhip" => (255, 239, 213),
        "peachpuff" => (255, 218, 185),
        "peru" => (205, 133, 63),
        "pink" => (255, 192, 203),
        "plum" => (221, 160, 221),
        "powderblue" => (176, 224, 230),
        "purple" => (128, 0, 128),
        "rebeccapurple" => (102, 51, 153),
        "red" => (255, 0, 0),
        "rosybrown" => (188, 143, 143),
        "royalblue" => (65, 105, 225),
        "saddlebrown" => (139, 69, 19),
        "salmon" => (250, 128, 114),
        "sandybrown" => (244, 164, 96),
        "seagreen" => (46, 139, 87),
        "seashell" => (255, 245, 238),
        "sienna" => (160, 82, 45),
        "silver" => (192, 192, 192),
        "skyblue" => (135, 206, 235),
        "slateblue" => (106, 90, 205),
        "slategray" | "slategrey" => (112, 128, 144),
        "snow" => (255, 250, 250),
        "springgreen" => (0, 255, 127),
        "steelblue" => (70, 130, 180),
        "tan" => (210, 180, 140),
        "teal" => (0, 128, 128),
        "thistle" => (216, 191, 216),
        "tomato" => (255, 99, 71),
        "turquoise" => (64, 224, 208),
        "violet" => (238, 130, 238),
        "wheat" => (245, 222, 179),
        "white" => (255, 255, 255),
        "whitesmoke" => (245, 245, 245),
        "yellow" => (255, 255, 0),
        "yellowgreen" => (154, 205, 50),
        _ => return None,
    })
}

pub(in crate::native_bridge::element::styles) fn style_property_priority(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> String {
    let Some(property) = canonical_specified_cssom_query_property_name(property) else {
        return String::new();
    };
    if let Some(state) = runtime.element_inline_style_declaration_state(handle)
        && let Some(priority) = inline_state_property_priority_with_pdb(state, &property)
    {
        return if priority {
            "important".to_owned()
        } else {
            String::new()
        };
    }
    let entries = style_entries(runtime, handle);
    if let Some(priority) = style_entries_property_priority_with_pdb(&entries, &property) {
        return if priority {
            "important".to_owned()
        } else {
            String::new()
        };
    }
    if let Some(entry) = inline_style_entry(runtime, handle, &property) {
        return if entry.priority {
            "important".to_owned()
        } else {
            String::new()
        };
    }
    if let Some(longhands) = shorthand_longhands(&property) {
        let mut priority = None;
        for longhand in longhands {
            let Some(entry) = inline_style_entry(runtime, handle, longhand) else {
                return String::new();
            };
            if priority.is_some_and(|current| current != entry.priority) {
                return String::new();
            }
            priority = Some(entry.priority);
        }
        if priority == Some(true) {
            return "important".to_owned();
        }
    }
    String::new()
}

fn canonical_specified_cssom_query_property_name(property: &str) -> Option<String> {
    if property.starts_with("--") && !moli_css_parse::is_cssom_custom_property_name(property) {
        return None;
    }
    let property = canonical_style_property_name(property);
    if !property.starts_with("--") && !known_style_property(&property) {
        return None;
    }
    Some(if property == "-webkit-transform" {
        "transform".to_owned()
    } else {
        property
    })
}

fn canonical_computed_cssom_query_property_name(property: &str) -> Option<String> {
    if property.starts_with("--") {
        return moli_css_parse::is_cssom_custom_property_name(property)
            .then(|| property.to_owned());
    }
    let property = canonical_style_property_name(property);
    let property = if property == "-webkit-transform" {
        "transform".to_owned()
    } else {
        property
    };
    computed_property_is_queryable(&property).then_some(property)
}

pub(in crate::native_bridge::element::styles) fn style_css_text_for_computed(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> String {
    let _ = (runtime, handle);
    String::new()
}

fn specified_color_value_is_valid(value: &str) -> bool {
    let value = value.trim();
    if moli_css_parse::css_value_may_contain_env_function(value) {
        return moli_css_parse::css_declaration_value_has_valid_env_functions(value);
    }
    matches!(
        value.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "revert-rule"
    ) || specified_color_component_value_is_valid(value)
}

fn specified_color_component_value_is_valid(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("transparent")
        || value.eq_ignore_ascii_case("currentcolor")
        || ident_is_system_color(value)
        || css_named_color_rgb(value).is_some()
        || css_hex_color_rgb(value).is_some()
        || ((value.starts_with("rgb(") || value.starts_with("rgba(")) && value.ends_with(')'))
}

pub(in crate::native_bridge::element::styles) fn style_property_names_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    context: StyleComputationContext,
) -> Vec<String> {
    if mode == StyleMode::Computed {
        return super::super::computed_names::computed_property_names(runtime, handle, context);
    }
    if let Some(state) = runtime.element_inline_style_declaration_state(handle) {
        return state.property_names();
    }
    style_entries(runtime, handle)
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

pub(in crate::native_bridge::element::styles) fn style_property_count_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    context: StyleComputationContext,
) -> usize {
    if mode == StyleMode::Computed {
        return super::super::computed_names::computed_property_count(runtime, handle, context);
    }
    style_property_names_with_context(runtime, handle, mode, context).len()
}

pub(in crate::native_bridge::element::styles) fn style_property_name_at_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    context: StyleComputationContext,
    index: usize,
) -> Option<String> {
    if mode == StyleMode::Computed {
        return super::super::computed_names::computed_property_name_at(
            runtime, handle, context, index,
        );
    }
    style_property_names_with_context(runtime, handle, mode, context)
        .get(index)
        .cloned()
}

pub(in crate::native_bridge::element::styles) fn style_property_index_exists_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    context: StyleComputationContext,
    index: usize,
) -> bool {
    if mode == StyleMode::Computed {
        return super::super::computed_names::computed_property_name_at(
            runtime, handle, context, index,
        )
        .is_some();
    }
    index < style_property_count_with_context(runtime, handle, mode, context)
}

pub(in crate::native_bridge::element::styles) fn computed_style_applies(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    runtime
        .dom_host()
        .containing_shadow_root(handle)
        .is_none_or(|shadow_root| !runtime.shadow_root_is_disconnected_for_style(shadow_root))
}

#[cfg(test)]
mod tests {
    use super::super::style_world::connected_shadow_roots_for_test;
    use super::{
        KEYFRAME_NESTING_DEPTH_LIMIT, animation_shorthand_names, box_shorthand_component,
        collect_custom_functions_from_css, compress_box_shorthand_value,
        custom_function_container_rule_texts, format_css_number, format_css_px,
        keyframe_has_supported_animation_values, keyframe_property_values,
        normalize_computed_color_functions, normalize_css_integer_token, normalize_style_value,
        simple_var_function_parts,
    };
    use crate::dom::native::{DomHost, NativeDom};
    use crate::native_bridge::DomHandle;
    use std::collections::HashMap;

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ))
    }

    fn connect_for_test(host: &mut DomHost, parent: DomHandle, child: DomHandle) {
        assert!(host.append_child_without_mutation_effects(parent, child));
    }

    fn nested_keyframes(depth: usize) -> String {
        let mut css = "@keyframes anim { from { left: 0px; } to { left: 10px; } }".to_owned();
        for _ in 0..depth {
            css = format!("@media all {{ {css} }}");
        }
        css
    }

    #[test]
    fn css_number_serialization_discards_bounded_f32_integer_noise() {
        assert_eq!(format_css_number(120.000005), "120");
        assert_eq!(format_css_number(-120.000005), "-120");
        assert_eq!(format_css_number(120.00005), "120.00005");
    }

    #[test]
    fn css_pixel_serialization_matches_blink_six_significant_digits() {
        assert_eq!(format_css_px(33.328_125), "33.3281px");
        assert_eq!(format_css_px(0.015_625), "0.015625px");
        assert_eq!(format_css_px(999_999.0), "999999px");
        assert_eq!(format_css_px(1_000_000.0), "1e+06px");
    }

    #[test]
    fn connected_shadow_roots_for_document_excludes_child_document_roots() {
        let mut host = test_host();
        let document = host.document_handle();
        let active_host = host.create_element("section");
        connect_for_test(&mut host, document, active_host);
        let active_root = host
            .attach_shadow_root(active_host, "open")
            .expect("active document host should accept shadow root");

        let child_document = host.create_detached_html_document();
        let child_host = host.create_parser_element_without_attributes_for_document(
            child_document,
            "article".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        connect_for_test(&mut host, child_document, child_host);
        let child_root = host
            .attach_shadow_root(child_host, "open")
            .expect("child document host should accept shadow root");
        host.mark_subtree_connected_preserving_owner_document(child_document);

        assert_eq!(
            connected_shadow_roots_for_test(&host, document),
            vec![active_root]
        );
        assert_eq!(
            connected_shadow_roots_for_test(&host, child_document),
            vec![child_root]
        );
    }

    #[test]
    fn box_shorthand_component_keeps_function_whitespace_internal() {
        assert_eq!(
            box_shorthand_component("calc(10px + 5px) auto", 0).as_deref(),
            Some("calc(10px + 5px)")
        );
        assert_eq!(
            box_shorthand_component("calc(10px + 5px) auto", 1).as_deref(),
            Some("auto")
        );
        assert_eq!(
            box_shorthand_component("calc(50% + 2px) 4px", 0).as_deref(),
            Some("calc(50% + 2px)")
        );
        assert_eq!(
            box_shorthand_component("4px calc(50% + 2px)", 1).as_deref(),
            Some("calc(50% + 2px)")
        );
    }

    #[test]
    fn font_shorthand_slash_normalization_stays_renderer_local() {
        assert_eq!(
            normalize_style_value("font", "10px/1 Ahem"),
            "10px / 1 Ahem"
        );
        assert_eq!(
            normalize_style_value("font", "var(--font/size) / var(--line/height) Ahem"),
            "var(--font/size) / var(--line/height) Ahem"
        );
    }

    #[test]
    fn content_value_normalization_stays_renderer_local() {
        assert_eq!(normalize_style_value("content", "'string'"), r#""string""#);
        assert_eq!(
            normalize_style_value("content", "url(http://localhost/)"),
            r#"url("http://localhost/")"#
        );
        assert_eq!(
            normalize_style_value("content", "counter(par-num, decimal)"),
            "counter(par-num)"
        );
        assert_eq!(
            normalize_style_value("content", "attr( |bar )"),
            "attr( |bar )"
        );
    }

    #[test]
    fn font_family_value_normalization_stays_renderer_local() {
        assert_eq!(
            normalize_style_value("font-family", "'Lucida Grande'"),
            "Lucida Grande"
        );
        assert_eq!(normalize_style_value("font-family", "'34J'"), r#""34J""#);
        assert_eq!(
            normalize_style_value("font-family", "'serif'"),
            r#""serif""#
        );
        assert_eq!(normalize_style_value("font-family", "'A  B'"), r#""A  B""#);
    }

    #[test]
    fn css_integer_token_normalization_stays_renderer_local() {
        assert_eq!(
            normalize_css_integer_token("1111111111111111111111111").as_deref(),
            Some("1111111111111111111111111")
        );
        assert_eq!(normalize_css_integer_token("+0012").as_deref(), Some("12"));
        assert_eq!(normalize_css_integer_token("-000").as_deref(), Some("0"));
        assert_eq!(normalize_css_integer_token("1.0"), None);
        assert_eq!(normalize_css_integer_token("1px"), None);
    }

    #[test]
    fn simple_var_function_projection_stays_renderer_local() {
        assert_eq!(
            simple_var_function_parts("var(--x)")
                .filter(|parts| parts.fallback.is_none())
                .map(|parts| parts.name)
                .as_deref(),
            Some("--x")
        );
        assert_eq!(
            simple_var_function_parts("var(--x, red)")
                .and_then(|parts| parts.fallback)
                .as_deref(),
            Some("red")
        );
        assert_eq!(
            simple_var_function_parts(" var( --x ) ")
                .filter(|parts| parts.fallback.is_none())
                .map(|parts| parts.name)
                .as_deref(),
            Some("--x")
        );
        assert!(
            simple_var_function_parts("var(--x, 1)").is_some_and(|parts| parts.fallback.is_some())
        );
        assert_eq!(simple_var_function_parts("var(x)"), None);
        assert_eq!(simple_var_function_parts("calc(var(--x))"), None);

        assert_eq!(normalize_style_value("color", "var(--x)"), "var(--x)");
        assert_eq!(normalize_style_value("z-index", "var(--z)"), "var(--z)");
    }

    #[test]
    fn animation_shorthand_name_projection_stays_renderer_local() {
        assert_eq!(
            animation_shorthand_names("1s linear infinite alternate anim"),
            vec!["anim"]
        );
        assert_eq!(
            animation_shorthand_names("spin 1s ease, 200ms fade-in forwards"),
            vec!["spin", "fade-in"]
        );
        assert!(animation_shorthand_names("none").is_empty());
    }

    #[test]
    fn anchor_size_normalization_stays_renderer_local() {
        assert_eq!(
            normalize_style_value(
                "width",
                "anchor-size(width, anchor-size(--foo height, 10px))"
            ),
            "anchor-size(width, anchor-size(--foo height, 10px))"
        );
        assert_eq!(
            normalize_style_value("width", "anchor-size(width --target)"),
            "anchor-size(--target width)"
        );
    }

    #[test]
    fn custom_css_function_parser_extracts_container_results() {
        let mut functions = HashMap::new();
        collect_custom_functions_from_css(
            r#"
            @function --b() {
              @container --cont (width = 5px) { result: 5px; }
              @container --cont (width = 10px) { result: 10px; }
            }
            "#,
            &mut functions,
        );
        let function = functions.get("--b").expect("custom function");
        assert_eq!(function.container_results.len(), 2);
        assert_eq!(function.container_results[0].container_name, "--cont");
        assert_eq!(function.container_results[0].width_px, 5.0);
        assert_eq!(function.container_results[0].result, "5px");
    }

    #[test]
    fn custom_css_function_container_projection_uses_rule_local_css_text() {
        let rules = custom_function_container_rule_texts(
            r#"
            @container --cont (width = 5px) { result: 5px; }
            @container --next (width = 10px) { result: 10px; }
            "#,
        );
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[0].css_text,
            "@container --cont (width = 5px) {result: 5px;}"
        );
        assert!(
            !rules[0].css_text.contains("--next"),
            "custom-function container projection must not capture trailing rules"
        );
    }

    #[test]
    fn compress_box_shorthand_value_keeps_function_components_intact() {
        assert_eq!(
            compress_box_shorthand_value("calc(5px + 5px) 1px calc(5px + 5px) 1px"),
            "calc(5px + 5px) 1px"
        );
        assert_eq!(
            compress_box_shorthand_value(
                "calc(5px + 5px) calc(5px + 5px) calc(5px + 5px) calc(5px + 5px)"
            ),
            "calc(5px + 5px)"
        );
    }

    #[test]
    fn normalize_computed_color_functions_replaces_only_full_system_color_tokens() {
        assert_eq!(
            normalize_computed_color_functions("1px 1px MenuText", None),
            "rgb(0, 0, 0) 1px 1px"
        );
        assert_eq!(
            normalize_computed_color_functions("1px 1px menutext", None),
            "rgb(0, 0, 0) 1px 1px"
        );
        assert_eq!(
            normalize_computed_color_functions("1px 1px NotMenuText", None),
            "1px 1px NotMenuText"
        );
        assert_eq!(
            normalize_computed_color_functions("1px 1px menutext, 2px 2px linktext", None),
            "rgb(0, 0, 0) 1px 1px, rgb(0, 0, 238) 2px 2px"
        );
        assert_eq!(
            normalize_computed_color_functions(
                "1px 1px color-mix(in srgb, rgb(0,0,0), white)",
                None
            ),
            "1px 1px color-mix(in srgb, rgb(0,0,0), white)"
        );
        assert_eq!(
            normalize_computed_color_functions(
                "1px 1px currentcolor, 2px 2px LinkText",
                Some("rgb(10, 20, 30)")
            ),
            "rgb(10, 20, 30) 1px 1px, rgb(0, 0, 238) 2px 2px"
        );
    }

    #[test]
    fn keyframe_supported_animation_scan_has_depth_limit() {
        let names = vec!["anim".to_owned()];

        assert!(keyframe_has_supported_animation_values(
            &nested_keyframes(KEYFRAME_NESTING_DEPTH_LIMIT),
            &names
        ));
        assert!(!keyframe_has_supported_animation_values(
            &nested_keyframes(KEYFRAME_NESTING_DEPTH_LIMIT + 1),
            &names
        ));
    }

    #[test]
    fn keyframe_animation_scan_uses_stylo_nested_rule_snapshots() {
        let names = vec!["move".to_owned()];
        let css = r#"
            @media all {
              @supports (display: block) {
                @keyframes move {
                  from { left: 0px; }
                  to { left: 20px; }
                }
              }
            }
        "#;

        assert!(keyframe_has_supported_animation_values(css, &names));
        assert_eq!(
            keyframe_property_values(css, &names, "left"),
            Some(("0px".to_owned(), "20px".to_owned()))
        );
    }

    #[test]
    fn keyframe_animation_scan_uses_pdb_declaration_values() {
        let names = vec!["fade".to_owned()];
        let css = r#"
            @keyframes fade {
              from {
                background-color: rgb(0 128 0 / 50%);
                width: calc(7px * up);
              }
              to {
                background-color: rgb(0 128 0 / 50%);
                width: calc(10px + 1vmin + 10%);
              }
            }
        "#;

        assert!(keyframe_has_supported_animation_values(css, &names));
        assert_eq!(
            keyframe_property_values(css, &names, "background-color"),
            Some((
                "rgba(0, 128, 0, 0.5)".to_owned(),
                "rgba(0, 128, 0, 0.5)".to_owned()
            ))
        );
        assert_eq!(keyframe_property_values(css, &names, "width"), None);
    }
}

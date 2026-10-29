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
            ClientRect, observable_bounding_client_rects, observable_used_grid_tracks,
            read_bounding_client_rect,
        },
        style_engine::{
            ComputedRenderedStyleFacts, FullStyleWorldSnapshot, StyleViewport,
            StylesheetResourceSnapshot, StyloComputedStyleSnapshot, StyloStylesheetSource,
            computed_property_is_queryable,
        },
    },
};
use cssparser::{Parser, Token, serialize_identifier, serialize_string};
use moli_selector::html_directionality;
use std::collections::{HashMap, HashSet};
use style::{properties::ComputedValues, servo_arc::Arc as ServoArc, stylesheets::CssRuleType};

use super::super::super::super::JsContextHost;
use super::observation::{
    ComputedStyleRead, PreparedAnonymousStyleContext, RetainedStyleObservation,
    StyleComputationContext, StyleObservation,
};
use super::sizing::used_size_from_layout_snapshot;
use super::style_world::style_base_url;
use super::{
    StyleMode, all_shorthand_applies_to, animation_shorthand_longhands, css_wide_keyword,
    font_variant_longhands, inline_state_property_priority_with_pdb,
    inline_state_property_value_with_pdb, known_style_property, normalize_css_integer_token,
    shorthand_longhands, style_entries, style_entries_property_priority_with_pdb,
    style_entries_property_value_with_pdb, text_decoration_shorthand_longhands,
    transition_shorthand_longhands,
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
                true,
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
    pub(in crate::native_bridge::element) fn handle_for_animation(&self) -> DomHandle {
        self.handle
    }
    pub(crate) fn new(runtime: &'a JsContextHost, handle: DomHandle) -> Self {
        StyleObservation::new_for_element(runtime, handle, None).read(handle)
    }

    pub(in crate::native_bridge::element::styles) fn new_with_context(
        runtime: &'a JsContextHost,
        handle: DomHandle,
        context: StyleComputationContext,
    ) -> Self {
        StyleObservation::new_for_element(runtime, handle, Some(context)).read(handle)
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

    pub(crate) fn rendered_style_facts(&self) -> Option<ComputedRenderedStyleFacts> {
        Some(
            StyloComputedStyleSnapshot::from_primary(self.computed_values()?)
                .rendered_style_facts(),
        )
    }

    /// Retain the before-change computed style without serializing every
    /// potentially transitioned longhand for every element.
    pub(in crate::native_bridge::element) fn style_snapshot(
        &self,
    ) -> Option<&StyloComputedStyleSnapshot> {
        self.stylo_style.as_ref()
    }

    pub(in crate::native_bridge::element) fn project_animation_samples(
        &self,
        samples: &[crate::animation_model::EffectSample],
    ) -> Option<StyloComputedStyleSnapshot> {
        let base = self.style_snapshot()?.computed_values();
        Some(self.project_animation_samples_from(&base, samples))
    }

    pub(in crate::native_bridge::element) fn project_animation_samples_from(
        &self,
        base: &ServoArc<ComputedValues>,
        samples: &[crate::animation_model::EffectSample],
    ) -> StyloComputedStyleSnapshot {
        if samples.is_empty() {
            return StyloComputedStyleSnapshot::from_primary(base.clone());
        }
        let parent = self
            .runtime
            .animation_parent_style(self.handle)
            .map(|(handle, base)| {
                self.observation_inputs
                    .animation_style(handle, &base, self.sampled_at)
            });
        StyloComputedStyleSnapshot::from_primary(
            self.runtime.project_animation_values_with_parent(
                self.context
                    .resolved_read_document(self.runtime, self.handle),
                self.handle,
                base,
                samples,
                parent.as_ref(),
            ),
        )
    }

    pub(crate) fn computed_values(&self) -> Option<ServoArc<ComputedValues>> {
        let base = self.stylo_style.as_ref()?.computed_values();
        if !self.sample_animations {
            return Some(base);
        }
        Some(
            self.sampled_values
                .get_or_init(|| {
                    self.observation_inputs
                        .animation_style(self.handle, &base, self.sampled_at)
                })
                .clone(),
        )
    }

    pub(crate) fn accessibility_style(
        &self,
    ) -> Option<moli_dom::accessibility::AccessibilityStyle> {
        Some(
            StyloComputedStyleSnapshot::from_primary(self.computed_values()?).accessibility_style(),
        )
    }

    /// Transfers the observation's owned Stylo handles into box construction.
    /// The only reference-count increments happened when the canonical
    /// `ElementStyles` crossed the renderer borrow boundary.
    pub(crate) fn into_element_computed_values(
        mut self,
    ) -> Option<(
        ServoArc<ComputedValues>,
        Option<ServoArc<ComputedValues>>,
        Option<ServoArc<ComputedValues>>,
    )> {
        let sampled = self.computed_values()?;
        let (canonical, before, after) = self.stylo_style.take()?.into_element_computed_values();
        if ServoArc::ptr_eq(&canonical, &sampled) {
            return Some((sampled, before, after));
        }
        let before = before.map(|style| self.sample_pseudo_inheritance(style, &sampled));
        let after = after.map(|style| self.sample_pseudo_inheritance(style, &sampled));
        Some((sampled, before, after))
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
            .map(|snapshot| {
                let values = snapshot.computed_values();
                self.computed_values().map_or_else(
                    || values.clone(),
                    |primary| self.sample_pseudo_inheritance(values.clone(), &primary),
                )
            })
    }

    fn sample_pseudo_inheritance(
        &self,
        pseudo: ServoArc<ComputedValues>,
        primary: &ServoArc<ComputedValues>,
    ) -> ServoArc<ComputedValues> {
        if !self.sample_animations
            || self
                .stylo_style
                .as_ref()
                .is_some_and(|base| ServoArc::ptr_eq(&base.computed_values(), primary))
        {
            return pseudo;
        }
        self.runtime.project_animation_values_with_parent(
            self.context
                .resolved_read_document(self.runtime, self.handle),
            self.handle,
            &pseudo,
            &[],
            Some(primary),
        )
    }

    pub(crate) fn prepare_anonymous_style_context(
        &self,
    ) -> Option<PreparedAnonymousStyleContext<'a>> {
        self.stylo_style.as_ref()?;
        let read_document = self
            .context
            .resolved_read_document(self.runtime, self.handle);
        Some(PreparedAnonymousStyleContext::new(
            self.runtime,
            read_document,
        ))
    }

    fn property_in_prepared_scope(&self, property: &str) -> String {
        let stylesheet_query_snapshot =
            computed_property_requires_stylesheet_sources(property, self.stylo_style.as_ref())
                .then(|| self.observation_inputs.stylesheet_query_snapshot());
        // Declaration reconciliation compares unanimated before/after styles.
        // Ordinary CSSOM reads instead share layout's sampled inheritance chain.
        let reconciling =
            crate::native_bridge::element::animation_owner_for_node(self.runtime, self.handle)
                .and_then(|owner| self.runtime.animation_transition_state().document(owner))
                .is_some_and(|document| document.reconciling_style);
        let sampled = (self.sample_animations && !reconciling)
            .then(|| self.computed_values())
            .flatten()
            .map(StyloComputedStyleSnapshot::from_primary);
        computed_style_property_value_after_style_update(
            self.runtime,
            self.handle,
            property,
            self.context,
            stylesheet_query_snapshot.as_deref(),
            Some(self.observation_inputs.as_ref()),
            sampled.as_ref().or(self.stylo_style.as_ref()),
            // The main CSSOM path already sampled the complete inheritance
            // chain. Reconciliation must retain its unanimated baseline too.
            !reconciling && sampled.is_none(),
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
        self.pseudo_computed_values(pseudo_element)
            .and_then(|style| {
                StyloComputedStyleSnapshot::from_primary(style).resolved_property_value(&property)
            })
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

    pub(crate) fn properties(&self) -> Vec<(String, String)> {
        self.property_names()
            .into_iter()
            .map(|name| {
                let value = self.property_in_prepared_scope(&name);
                (name, value)
            })
            .collect()
    }
}

mod specified_values;
use self::specified_values::*;
mod computed_properties;
use self::computed_properties::*;
mod custom_functions;
use self::custom_functions::*;
mod computed_values;
use self::computed_values::*;
mod cssom_accessors;
pub(in crate::native_bridge::element::styles) use self::computed_properties::style_property_value_with_context;
pub(crate) use self::computed_properties::{
    normalize_transition_behavior_list, normalize_transition_property_list,
    normalize_transition_timing_function_list, parse_transition_shorthand_entries,
};
pub(crate) use self::computed_properties::{
    serialize_animation_range_shorthand, serialize_animation_shorthand_from_longhands,
    serialize_transition_shorthand_from_longhands,
};
pub(in crate::native_bridge::element::styles) use self::computed_values::style_property_value_for_pseudo_with_context;
use self::cssom_accessors::*;
pub(in crate::native_bridge::element::styles) use self::cssom_accessors::{
    computed_style_applies, style_css_text_for_computed, style_property_count_with_context,
    style_property_index_exists_with_context, style_property_name_at_with_context,
    style_property_names_with_context, style_property_priority,
};
pub(super) use self::specified_values::computed_style_default_value;
pub(in crate::native_bridge::element::styles) use self::specified_values::normalize_style_value_with_base;
pub(crate) use self::specified_values::{
    css_animation_effect_keyframes, raw_inline_style_property_value, style_property_value,
};

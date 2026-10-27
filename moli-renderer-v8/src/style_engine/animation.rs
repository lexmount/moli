//! Project sampled effects onto a derived style. Canonical before-change style
//! stays unanimated; layout and paint receive the sampled, typed Stylo values.
use super::MoliStyleEngine;
use crate::{document_runtime::DomHandle, dom::native::DomHost};
use style::{
    context::{CascadeInputs, TreeCountingCaches},
    dom::{AttributeTracker, DummyElementContext, TElement, TNode},
    properties::{
        CASCADE_PROPERTY, ComputedValues, Importance, PrioritaryPropertyId, PropertyDeclaration,
        PropertyDeclarationBlock, PropertyDeclarationId, StyleBuilder,
        animated_properties::AnimationValue, cascade::FirstLineReparenting,
    },
    rule_cache::RuleCacheConditions,
    rule_tree::{CascadeLevel, CascadeOrigin},
    servo_arc::Arc,
    shared_lock::StylesheetGuards,
    stylesheets::{container_rule::ContainerSizeQuery, layer_rule::LayerOrder},
    values::{
        animated::{Animate, Procedure},
        computed::{Context, ToComputedValue},
    },
};

#[derive(Clone, Copy)]
pub(crate) struct TransitionValuePair<'a> {
    pub(crate) property: &'a str,
    pub(crate) from: &'a str,
    pub(crate) to: &'a str,
    pub(crate) allow_discrete: bool,
}

// Different properties follow Stylo's dependency graph; effects on the same
// property keep their composite order. Cache ranks rather than parsing and
// serializing another set of declarations on every sample.
fn animation_property_order(property: &str) -> usize {
    use style::properties::{PrioritaryPropertyId, PropertyId};
    static RANKS: std::sync::LazyLock<Vec<usize>> = std::sync::LazyLock::new(|| {
        fn rank(id: PrioritaryPropertyId, ranks: &mut [usize]) -> usize {
            if ranks[id as usize] != 0 {
                return ranks[id as usize];
            }
            let dependencies = id.dependencies();
            let value = PrioritaryPropertyId::each()
                .filter(|dependency| dependencies.contains(*dependency))
                .map(|dependency| rank(dependency, ranks))
                .max()
                .unwrap_or(0)
                + 1;
            ranks[id as usize] = value;
            value
        }
        let mut ranks = vec![0; PrioritaryPropertyId::each().count()];
        for id in PrioritaryPropertyId::each() {
            rank(id, &mut ranks);
        }
        ranks
    });
    if property.starts_with("--") {
        return 0;
    }
    let Ok(PropertyId::NonCustom(id)) = PropertyId::parse_enabled_for_all_content(property) else {
        return usize::MAX;
    };
    id.as_longhand()
        .and_then(PrioritaryPropertyId::from_longhand)
        .map_or(usize::MAX, |id| RANKS[id as usize])
}

impl MoliStyleEngine {
    pub(crate) fn can_sample_animation_property(
        &self,
        document: DomHandle,
        property: &str,
    ) -> bool {
        if !property.starts_with("--") {
            return true;
        }
        let Ok(style::properties::PropertyId::Custom(name)) =
            style::properties::PropertyId::parse_enabled_for_all_content(property)
        else {
            return false;
        };
        // Use the retained Stylist for both @property and registerProperty().
        self.world_for_document(document)
            .document_state
            .with_retained_style_system(|retained| {
                !retained
                    .stylist
                    .get_custom_property_registration(&name)
                    .is_universal()
            })
    }

    pub(crate) fn animation_parent_style(
        &self,
        host: &DomHost,
        handle: DomHandle,
    ) -> Option<(DomHandle, Arc<ComputedValues>)> {
        self.dom_adapter.with_bound_host(host, |adapter| {
            let parent = adapter.element(host, handle)?.traversal_parent()?;
            let values = parent.borrow_data()?.styles.primary().clone();
            Some((DomHandle::new(parent.as_node().debug_id()), values))
        })
    }

    pub(crate) fn transition_values_are_eligible(
        &self,
        host: &DomHost,
        document: DomHandle,
        handle: DomHandle,
        base: &Arc<ComputedValues>,
        pair: TransitionValuePair<'_>,
    ) -> bool {
        let TransitionValuePair {
            property,
            from,
            to,
            allow_discrete,
        } = pair;
        let world = self.world_for_document(document);
        self.dom_adapter.with_bound_host(host, |adapter| {
            world.document_state.with_retained_style_system(|retained| {
                let _thread = super::computed::StyloLayoutThreadStateGuard::enter();
                let stylist = &retained.stylist;
                let parent = adapter
                    .element(host, handle)
                    .and_then(|e| e.traversal_parent())
                    .and_then(|e| e.borrow_data().map(|data| data.styles.primary().clone()));
                let builder = StyleBuilder::for_derived_style(
                    stylist.device(),
                    Some(stylist),
                    base,
                    parent.as_deref(),
                );
                let mut conditions = RuleCacheConditions::default();
                let mut trees = TreeCountingCaches::default();
                let mut context = Context::new_for_animation(
                    builder,
                    stylist.quirks_mode(),
                    &mut conditions,
                    ContainerSizeQuery::none(),
                    &DummyElementContext {},
                    &mut trees,
                );
                let parse = |value: &str| {
                    let mut block = moli_css_parse::CssDeclarationBlock::default();
                    block.set_property(property, value, false);
                    block.into_inner()
                };
                let from = parse(from);
                let to = parse(to);
                let mut attributes = AttributeTracker::new_dummy();
                let mut found = false;
                for (left, _) in from.declaration_importance_iter() {
                    let id = left.id();
                    // Stylo's generic Custom id is conservatively discrete;
                    // registered typed properties instead use their descriptor.
                    let discrete = match id {
                        PropertyDeclarationId::Custom(name) => stylist
                            .get_custom_property_registration(name)
                            .is_universal(),
                        _ => id.is_discrete_animatable(),
                    };
                    if !id.is_animatable() || (!allow_discrete && discrete) {
                        return false;
                    }
                    let Some((right, _)) =
                        to.declaration_importance_iter().find(|(v, _)| v.id() == id)
                    else {
                        return false;
                    };
                    let Some((left, right)) = AnimationValue::from_declaration(
                        left,
                        &mut context,
                        base,
                        stylist.device().default_computed_values(),
                        &mut attributes,
                    )
                    .zip(AnimationValue::from_declaration(
                        right,
                        &mut context,
                        base,
                        stylist.device().default_computed_values(),
                        &mut attributes,
                    )) else {
                        return false;
                    };
                    if !allow_discrete && !left.interpolable_with(&right) {
                        return false;
                    }
                    found = true;
                }
                found
            })
        })
    }

    pub(crate) fn project_animation_values(
        &self,
        host: &DomHost,
        document: DomHandle,
        handle: DomHandle,
        base: &Arc<ComputedValues>,
        values: &[moli_animation::PropertySample],
        parent_override: Option<&Arc<ComputedValues>>,
    ) -> Arc<ComputedValues> {
        if values.is_empty() && parent_override.is_none() {
            return base.clone();
        }
        let world = self.world_for_document(document);
        self.dom_adapter.with_bound_host(host, |adapter| {
            world.document_state.with_retained_style_system(|retained| {
                let _thread = super::computed::StyloLayoutThreadStateGuard::enter();
                let stylist = &retained.stylist;
                let parent = adapter
                    .element(host, handle)
                    .and_then(|e| e.traversal_parent())
                    .and_then(|e| e.borrow_data().map(|data| data.styles.primary().clone()));
                let parent_changed = parent_override.is_some_and(|sampled| {
                    parent
                        .as_ref()
                        .is_none_or(|base| !Arc::ptr_eq(base, sampled))
                });
                if values.is_empty() && !parent_changed {
                    return base.clone();
                }
                let parent = parent_override.or(parent.as_ref());
                let mut conditions = RuleCacheConditions::default();
                let mut trees = TreeCountingCaches::default();
                // Re-cascade from the retained rule node, without changing the
                // canonical baseline. This handles inherited values, explicit
                // overrides and dependencies such as em/currentColor together.
                let inherited_base = if parent_changed {
                    let guard = adapter.shared_lock().read();
                    stylist.cascade_style_and_visited(
                        adapter.element(host, handle),
                        None,
                        &CascadeInputs::new_from_style(base),
                        &StylesheetGuards::same(&guard),
                        parent.map(|v| &**v),
                        parent.map(|v| &**v),
                        FirstLineReparenting::No,
                        &Default::default(),
                        None,
                        &mut conditions,
                        &mut trees,
                    )
                } else {
                    base.clone()
                };
                let base = &inherited_base;
                let builder = StyleBuilder::for_derived_style(
                    stylist.device(),
                    Some(stylist),
                    base,
                    parent.map(|v| &**v),
                );
                let mut context = Context::new_for_animation(
                    builder,
                    stylist.quirks_mode(),
                    &mut conditions,
                    ContainerSizeQuery::none(),
                    &DummyElementContext {},
                    &mut trees,
                );
                let mut attributes = AttributeTracker::new_dummy();
                let mut sampled = PropertyDeclarationBlock::new();
                let mut recascade_dependencies = false;
                let uses_underlying = values.iter().any(|v| v.from.is_none() || v.to.is_none());
                let mut composited = std::collections::HashMap::new();
                let order = values
                    .windows(2)
                    .any(|pair| {
                        animation_property_order(&pair[0].property)
                            > animation_property_order(&pair[1].property)
                    })
                    .then(|| {
                        let mut ordered = values.iter().collect::<Vec<_>>();
                        ordered.sort_by_key(|sample| animation_property_order(&sample.property));
                        ordered
                    });
                for index in 0..values.len() {
                    let sample = order
                        .as_ref()
                        .map_or(&values[index], |ordered| ordered[index]);
                    let parse = |value: &str| {
                        let mut block = moli_css_parse::CssDeclarationBlock::default();
                        block.set_property(&sample.property, value, false);
                        block.into_inner()
                    };
                    let from = sample.from.as_deref().map(parse);
                    let to = sample.to.as_deref().map(parse);
                    // At an extrapolated neutral endpoint both sides can be
                    // implicit. Only the property ids of this seed are used.
                    let seed;
                    let ids = if let Some(block) = from.as_ref().or(to.as_ref()) {
                        block
                    } else {
                        seed = parse("initial");
                        &seed
                    };
                    for (declaration, _) in ids.declaration_importance_iter() {
                        let id = declaration.id();
                        let mut resolve = |block: Option<&PropertyDeclarationBlock>| {
                            if let Some(block) = block {
                                let (value, _) = block
                                    .declaration_importance_iter()
                                    .find(|(v, _)| v.id() == id)?;
                                AnimationValue::from_declaration(
                                    value,
                                    &mut context,
                                    base,
                                    stylist.device().default_computed_values(),
                                    &mut attributes,
                                )
                            } else {
                                composited
                                    .get(&id.to_owned())
                                    .cloned()
                                    .or_else(|| AnimationValue::from_computed_values(id, base))
                            }
                        };
                        let Some((left, right)) = resolve(from.as_ref()).zip(resolve(to.as_ref()))
                        else {
                            continue;
                        };
                        let computed = if sample.progress == 0.0 {
                            left
                        } else if sample.progress == 1.0 {
                            right
                        } else {
                            left.animate(
                                &right,
                                Procedure::Interpolate {
                                    progress: sample.progress,
                                },
                            )
                            .unwrap_or(if sample.progress < 0.5 { left } else { right })
                        };
                        if uses_underlying {
                            composited.insert(id.to_owned(), computed.clone());
                        }
                        let declaration = computed.uncompute();
                        let declaration = &declaration;
                        if matches!(
                            declaration,
                            PropertyDeclaration::CSSWideKeyword(_)
                                | PropertyDeclaration::WithVariables(_)
                        ) {
                            continue;
                        }
                        if let PropertyDeclaration::Custom(custom) = declaration
                            && let style::properties::CustomDeclarationValue::Parsed(value) =
                                &custom.value
                        {
                            recascade_dependencies = true;
                            let registration =
                                stylist.get_custom_property_registration(&custom.name);
                            let value = value.to_computed_value(&context);
                            context
                                .builder
                                .substitution_functions
                                .custom_properties
                                .insert(registration, &custom.name, value);
                        }
                        if let PropertyDeclarationId::Longhand(id) = declaration.id() {
                            recascade_dependencies |=
                                PrioritaryPropertyId::from_longhand(id).is_some();
                            // SAFETY: id() selects this resolved declaration's exact variant.
                            unsafe {
                                CASCADE_PROPERTY[id as usize](declaration, &mut context);
                            }
                        }
                        sampled.push(declaration.clone(), Importance::Normal);
                    }
                }
                if !recascade_dependencies {
                    return context.builder.build();
                }
                // Sampling font/zoom/writing-mode or a custom property changes
                // other specified values on this element. Feed the sampled
                // declarations back into Stylo's dependency-ordered cascade;
                // mutating the derived struct alone leaves e.g. width:2em at
                // the unanimated font size. Keep the canonical rule node intact.
                drop(context);
                let sampled = Arc::new(adapter.shared_lock().wrap(sampled));
                let guard = adapter.shared_lock().read();
                let guards = StylesheetGuards::same(&guard);
                let mut inputs = CascadeInputs::new_from_style(base);
                let rules = inputs
                    .rules
                    .take()
                    .unwrap_or_else(|| stylist.rule_tree().root().clone());
                inputs.rules = Some(
                    stylist
                        .rule_tree()
                        .update_rule_at_level(
                            CascadeLevel::new(CascadeOrigin::Transitions),
                            LayerOrder::root(),
                            Some(sampled.borrow_arc()),
                            &rules,
                            &guards,
                            &mut false,
                        )
                        .unwrap_or(rules),
                );
                let sampled_style = stylist.cascade_style_and_visited(
                    adapter.element(host, handle),
                    None,
                    &inputs,
                    &guards,
                    parent.map(|v| &**v),
                    parent.map(|v| &**v),
                    FirstLineReparenting::No,
                    &Default::default(),
                    None,
                    &mut conditions,
                    &mut trees,
                );
                // Observations run serially on the Document owner. Temporary
                // animation rule nodes enter Stylo's free list when the sample
                // is dropped; periodically collect them even without a new
                // DOM/style reconciliation. Live sampled styles retain theirs.
                stylist.rule_tree().maybe_gc();
                sampled_style
            })
        })
    }
}

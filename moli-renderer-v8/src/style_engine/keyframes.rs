//! CSS animation inputs leave this adapter with decoded names, cyclically
//! matched timing, physical property identities and effective interval easing.
//! Specified declarations remain native so environment-dependent values are
//! resolved when sampled, rather than frozen during normalization.
use super::MoliStyleEngine;
use crate::{animation_model::CssTiming, document_runtime::DomHandle, dom::native::DomHost};
use moli_animation::{CssTime, FillMode};
use style::{
    Atom,
    dom::TElement,
    properties::{
        CSSWideKeyword, Importance, LonghandId, PropertyDeclaration, PropertyDeclarationBlock,
        PropertyDeclarationId,
    },
    stylesheets::keyframes_rule::KeyframesStepValue,
};
use style_traits::ToCss;

pub(crate) struct CssAnimationDeclaration {
    pub(crate) name: String,
    pub(crate) timing: CssTiming,
}

pub(crate) fn css_animation_declarations(
    values: &style::properties::ComputedValues,
) -> Vec<CssAnimationDeclaration> {
    use style::values::computed::{AnimationFillMode, AnimationPlayState};
    let ui = values.get_ui();
    ui.animation_name_iter()
        .enumerate()
        // `none` consumes a timing-list position even though it has no effect.
        .filter_map(|(index, name)| {
            let name = name.as_atom()?.to_string();
            Some(CssAnimationDeclaration {
                name,
                timing: CssTiming {
                    delay: CssTime::from_seconds(f64::from(
                        ui.animation_delay_mod(index).seconds(),
                    ))
                    .unwrap_or_default(),
                    duration: moli_animation::duration(f64::from(
                        ui.animation_duration_mod(index).seconds(),
                    )),
                    iterations: f64::from(ui.animation_iteration_count_mod(index).0),
                    paused: ui.animation_play_state_mod(index) == AnimationPlayState::Paused,
                    fill: match ui.animation_fill_mode_mod(index) {
                        AnimationFillMode::None => FillMode::None,
                        AnimationFillMode::Backwards => FillMode::Backwards,
                        AnimationFillMode::Forwards => FillMode::Forwards,
                        AnimationFillMode::Both => FillMode::Both,
                    },
                    easing: ui.animation_timing_function_mod(index).to_css_string(),
                    direction: ui.animation_direction_mod(index).to_css_string(),
                },
            })
        })
        .collect()
}

pub(crate) struct ResolvedKeyframe {
    pub(crate) offset: f64,
    pub(crate) declarations: Vec<PropertyDeclaration>,
    pub(crate) easing: String,
}

impl MoliStyleEngine {
    /// The caller has observed the element's canonical style. Its retained
    /// Stylist owns media/supports, cascade layers, tree scopes and name lookup.
    pub(crate) fn resolved_animation_keyframes(
        &self,
        host: &DomHost,
        handle: DomHandle,
        name: &str,
        default_easing: &str,
    ) -> Option<Vec<ResolvedKeyframe>> {
        let document = host.owner_document_handle(handle)?;
        let world = self.document_worlds.active_world(document)?;
        self.dom_adapter.with_bound_host(host, |adapter| {
            super::computed::install_shadow_cascade_data_for_resolution(&world, adapter);
            let element = adapter.element(host, handle)?;
            let writing_mode = element.borrow_data()?.styles.primary().writing_mode;
            let guard = adapter.shared_lock().read();
            world.document_state.with_retained_style_system(|retained| {
                let animation = retained
                    .stylist
                    .lookup_keyframes(&Atom::from(name), element)?;
                let mut merged: Vec<(f64, String, PropertyDeclarationBlock)> = Vec::new();
                for step in &animation.steps {
                    let KeyframesStepValue::Declarations { block } = &step.value else {
                        continue;
                    };
                    let offset = f64::from(step.start_offset.percentage.0);
                    let block = block.read_with(&guard);
                    let easing = block
                        .normal_declaration_iter()
                        .find_map(|declaration| match declaration {
                            PropertyDeclaration::AnimationTimingFunction(value) => {
                                value.0.first().map(ToCss::to_css_string)
                            }
                            PropertyDeclaration::CSSWideKeyword(value)
                                if declaration.id()
                                    == PropertyDeclarationId::Longhand(
                                        LonghandId::AnimationTimingFunction,
                                    ) =>
                            {
                                let inherited = (value.keyword == CSSWideKeyword::Inherit)
                                    .then(|| element.traversal_parent())
                                    .flatten()
                                    .and_then(|parent| {
                                        parent.borrow_data().map(|data| {
                                            data.styles
                                                .primary()
                                                .get_ui()
                                                .animation_timing_function_mod(0)
                                                .to_css_string()
                                        })
                                    });
                                Some(inherited.unwrap_or_else(|| {
                                    retained
                                        .stylist
                                        .device()
                                        .default_computed_values()
                                        .get_ui()
                                        .animation_timing_function_mod(0)
                                        .to_css_string()
                                }))
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| default_easing.to_owned());
                    // CSS blocks cascade only within the same effective easing
                    // group. Preserve the group's earliest position, including
                    // when another easing group appears between its blocks.
                    let index = merged
                        .iter()
                        .enumerate()
                        .rev()
                        .take_while(|(_, (previous, _, _))| *previous == offset)
                        .find(|(_, (_, previous, _))| *previous == easing)
                        .map(|(index, _)| index)
                        .unwrap_or_else(|| {
                            merged.push((offset, easing, PropertyDeclarationBlock::new()));
                            merged.len() - 1
                        });
                    let destination = &mut merged[index].2;
                    for declaration in block.normal_declaration_iter() {
                        // Resolve logical aliases before cascading, so an
                        // explicit physical endpoint shares the same track and
                        // same-frame declaration order remains authoritative.
                        destination.push(declaration.to_physical(writing_mode), Importance::Normal);
                    }
                }
                Some(
                    merged
                        .into_iter()
                        .map(|(offset, easing, block)| ResolvedKeyframe {
                            offset,
                            easing,
                            declarations: block
                                .declarations()
                                .iter()
                                .filter(|declaration| declaration.id().is_animatable())
                                .cloned()
                                .collect(),
                        })
                        .collect(),
                )
            })
        })
    }
}

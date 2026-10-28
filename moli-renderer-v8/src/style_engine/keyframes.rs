//! Query the retained cascade instead of reimplementing stylesheet applicability.
use super::MoliStyleEngine;
use crate::{document_runtime::DomHandle, dom::native::DomHost};
use style::{
    Atom,
    properties::{Importance, PropertyDeclaration, PropertyDeclarationBlock},
    stylesheets::keyframes_rule::KeyframesStepValue,
};
use style_traits::ToCss;

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
                        .find_map(|declaration| {
                            if let PropertyDeclaration::AnimationTimingFunction(value) = declaration
                            {
                                value.0.first().map(ToCss::to_css_string)
                            } else {
                                None
                            }
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
                        destination.push(declaration.clone(), Importance::Normal);
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

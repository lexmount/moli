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
    pub(crate) easing: Option<String>,
}

impl MoliStyleEngine {
    /// The caller has observed the element's canonical style. Its retained
    /// Stylist owns media/supports, cascade layers, tree scopes and name lookup.
    pub(crate) fn resolved_animation_keyframes(
        &self,
        host: &DomHost,
        handle: DomHandle,
        name: &str,
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
                let mut merged: Vec<(f64, PropertyDeclarationBlock)> = Vec::new();
                for step in &animation.steps {
                    let KeyframesStepValue::Declarations { block } = &step.value else {
                        continue;
                    };
                    let offset = f64::from(step.start_offset.percentage.0);
                    if merged
                        .last()
                        .is_none_or(|(previous, _)| *previous != offset)
                    {
                        merged.push((offset, PropertyDeclarationBlock::new()));
                    }
                    let destination = &mut merged.last_mut().unwrap().1;
                    for declaration in block.read_with(&guard).normal_declaration_iter() {
                        destination.push(declaration.clone(), Importance::Normal);
                    }
                }
                Some(
                    merged
                        .into_iter()
                        .map(|(offset, block)| ResolvedKeyframe {
                            offset,
                            easing: block.declarations().iter().find_map(|declaration| {
                                if let PropertyDeclaration::AnimationTimingFunction(value) =
                                    declaration
                                {
                                    value.0.first().map(ToCss::to_css_string)
                                } else {
                                    None
                                }
                            }),
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

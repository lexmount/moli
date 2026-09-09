use selectors::sink::Push;
use style::{
    applicable_declarations::ApplicableDeclarationBlock,
    properties::{
        Importance, PropertyDeclaration, PropertyDeclarationBlock,
        longhands::content_visibility::SpecifiedValue as ContentVisibility,
    },
    rule_tree::{CascadeLevel, CascadeOrigin},
    servo_arc::Arc,
    shared_lock::SharedRwLock,
    stylesheets::layer_rule::LayerOrder,
};

use crate::dom::{
    NodeId,
    native::{DomHost, Node},
};

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

pub(super) fn synthesize_hidden_until_found_presentational_hint<V>(
    host: &DomHost,
    handle: NodeId,
    shared_lock: &SharedRwLock,
    hints: &mut V,
) where
    V: Push<ApplicableDeclarationBlock>,
{
    let Some(element) = host.node(handle).and_then(Node::as_element) else {
        return;
    };
    if element.namespace() != HTML_NAMESPACE
        || !element
            .attribute("hidden")
            .is_some_and(|value| value.eq_ignore_ascii_case("until-found"))
    {
        return;
    }

    let declarations = PropertyDeclarationBlock::with_one(
        PropertyDeclaration::ContentVisibility(ContentVisibility::Hidden),
        Importance::Normal,
    );
    hints.push(ApplicableDeclarationBlock::from_declarations(
        Arc::new(shared_lock.wrap(declarations)),
        CascadeLevel::new(CascadeOrigin::PresHints),
        LayerOrder::root(),
    ));
}

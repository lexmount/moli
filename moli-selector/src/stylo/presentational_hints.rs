use selectors::sink::Push;
use style::{
    applicable_declarations::ApplicableDeclarationBlock,
    properties::{
        Importance, PropertyDeclaration, PropertyDeclarationBlock,
        longhands::content_visibility::SpecifiedValue as ContentVisibility,
        longhands::direction::SpecifiedValue as Direction,
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

use super::query::html_directionality;

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

pub(super) fn synthesize_directionality_presentational_hint<V>(
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
    if element.namespace() != HTML_NAMESPACE {
        return;
    }

    let dir = element.attribute("dir");
    let direction = match dir {
        Some(value) if value.eq_ignore_ascii_case("ltr") => Direction::Ltr,
        Some(value) if value.eq_ignore_ascii_case("rtl") => Direction::Rtl,
        Some(value) if value.eq_ignore_ascii_case("auto") => {
            direction_from_html_directionality(host, handle)
        }
        value
            if element.is_html_element("bdi")
                && !value.is_some_and(|value| {
                    matches!(value.to_ascii_lowercase().as_str(), "ltr" | "rtl" | "auto")
                }) =>
        {
            direction_from_html_directionality(host, handle)
        }
        Some(_) if element.is_html_element("body") => Direction::Ltr,
        _ => return,
    };

    let declarations = PropertyDeclarationBlock::with_one(
        PropertyDeclaration::Direction(direction),
        Importance::Normal,
    );
    hints.push(ApplicableDeclarationBlock::from_declarations(
        Arc::new(shared_lock.wrap(declarations)),
        CascadeLevel::new(CascadeOrigin::PresHints),
        LayerOrder::root(),
    ));
}

fn direction_from_html_directionality(host: &DomHost, handle: NodeId) -> Direction {
    match html_directionality(host, handle) {
        crate::CssDirection::Ltr => Direction::Ltr,
        crate::CssDirection::Rtl => Direction::Rtl,
    }
}

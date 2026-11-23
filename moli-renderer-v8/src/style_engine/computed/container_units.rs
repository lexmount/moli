use std::collections::HashMap;

use moli_selector::StyloElement;
use style::{
    Atom,
    applicable_declarations::{CascadePriority, RevertKind},
    computed_value_flags::ComputedValueFlags,
    context::SharedStyleContext,
    dom::{TElement, TNode},
    properties::{
        CSSWideKeyword, ComputedValues, CustomDeclarationValue, LonghandId,
        OwnedPropertyDeclarationId, PropertyDeclaration, PropertyDeclarationBlock,
        PropertyDeclarationId, PropertyId,
    },
    rule_tree::CascadeOrigin,
    selector_parser::PseudoElement,
    servo_arc::Arc as ServoArc,
    shared_lock::StylesheetGuards,
    values::computed::font::LineHeight,
};
use style_traits::ToCss;

use super::DomHandle;
use crate::css_style::value_dependencies::{
    ValueDependency, is_container_unit, value_has_dependency,
};

pub(super) fn property_uses_container_units(
    shared: &SharedStyleContext<'_>,
    element: StyloElement<'_>,
    style: ServoArc<ComputedValues>,
    pseudo: Option<&PseudoElement>,
    property: &str,
) -> bool {
    let Ok(property) = PropertyId::parse_enabled_for_all_content(property) else {
        return false;
    };
    let target = StyledElement {
        element,
        style,
        pseudo,
    };
    let mut dependencies = ContainerUnitDependencies {
        shared,
        properties: HashMap::new(),
    };
    match property.as_shorthand() {
        Ok(shorthand) => shorthand.longhands().any(|longhand| {
            dependencies.property(&target, PropertyDeclarationId::Longhand(longhand), 0)
        }),
        Err(property) => dependencies.property(&target, property, 0),
    }
}

#[derive(Clone)]
struct StyledElement<'dom, 'style> {
    element: StyloElement<'dom>,
    style: ServoArc<ComputedValues>,
    pseudo: Option<&'style PseudoElement>,
}

impl StyledElement<'_, '_> {
    fn inheritance_parent(&self) -> Option<Self> {
        let element = if self.pseudo.is_some() {
            self.element
        } else {
            self.element.traversal_parent()?
        };
        let style = element.borrow_data()?.styles.get_primary()?.clone();
        Some(Self {
            element,
            style,
            pseudo: None,
        })
    }

    fn has_size_container(&self) -> bool {
        self.style
            .flags
            .contains(ComputedValueFlags::SELF_OR_ANCESTOR_HAS_SIZE_CONTAINER_TYPE)
    }
}

struct ContainerUnitDependencies<'a, 'shared> {
    shared: &'a SharedStyleContext<'shared>,
    // None marks an edge under evaluation so cyclic references terminate.
    properties: HashMap<(DomHandle, bool, OwnedPropertyDeclarationId), Option<bool>>,
}

impl ContainerUnitDependencies<'_, '_> {
    fn property(
        &mut self,
        target: &StyledElement<'_, '_>,
        property: PropertyDeclarationId<'_>,
        depth: usize,
    ) -> bool {
        if !target.has_size_container() {
            return false;
        }
        if depth >= 64 {
            return true;
        }
        let key = (
            DomHandle::new(target.element.as_node().debug_id()),
            target.pseudo.is_some(),
            property.to_owned(),
        );
        if let Some(result) = self.properties.get(&key) {
            return result.unwrap_or(false);
        }
        self.properties.insert(key.clone(), None);
        let depends = self.declaration(target, property, depth);
        self.properties.insert(key, Some(depends));
        depends
    }

    fn declaration(
        &mut self,
        target: &StyledElement<'_, '_>,
        property: PropertyDeclarationId<'_>,
        depth: usize,
    ) -> bool {
        let (inherits, computes_units) = match property {
            PropertyDeclarationId::Longhand(id) => (id.inherited(), true),
            PropertyDeclarationId::Custom(name) => {
                let registration = self.shared.stylist.get_custom_property_registration(name);
                (registration.inherits(), !registration.is_universal())
            }
        };
        // Inheritance walks the flat tree without spending the dependency
        // recursion budget on ancestors that have no declaration of their own.
        let mut target = target.clone();
        while target.has_size_container() {
            let declaration = winning_declaration(&target, property, &self.shared.guards);
            let keyword = declaration.and_then(|(declaration, _)| declaration_keyword(declaration));
            if keyword == Some(CSSWideKeyword::Inherit)
                || inherits && (keyword == Some(CSSWideKeyword::Unset) || declaration.is_none())
            {
                let Some(parent) = target.inheritance_parent() else {
                    return false;
                };
                target = parent;
                continue;
            }
            let Some((declaration, block)) = declaration.filter(|_| keyword.is_none()) else {
                return false;
            };
            let Some(value) = serialized_declaration(declaration, block) else {
                return computes_units
                    && target
                        .style
                        .flags
                        .contains(ComputedValueFlags::USES_CONTAINER_UNITS);
            };
            return self.value(&target, property, &value, computes_units, depth);
        }
        false
    }

    fn parent_property(
        &mut self,
        target: &StyledElement<'_, '_>,
        property: PropertyDeclarationId<'_>,
        depth: usize,
    ) -> bool {
        target
            .inheritance_parent()
            .is_some_and(|parent| self.property(&parent, property, depth))
    }

    fn value(
        &mut self,
        target: &StyledElement<'_, '_>,
        property: PropertyDeclarationId<'_>,
        value: &str,
        computes_units: bool,
        depth: usize,
    ) -> bool {
        if depth >= 64 {
            return true;
        }
        let is_font_size = property == PropertyDeclarationId::Longhand(LonghandId::FontSize);
        let is_line_height = property == PropertyDeclarationId::Longhand(LonghandId::LineHeight);
        value_has_dependency(value, &mut |dependency| {
            if let ValueDependency::Variable(name) = dependency {
                let name = Atom::from(name.strip_prefix("--")?);
                let computed = target.style.custom_properties().get_for_cssom(&name)?;
                // Typed substitutions may already be pixels. Their producer
                // can depend on a container in a different inherited style.
                return Some(
                    self.property(target, PropertyDeclarationId::Custom(&name), depth + 1)
                        || computes_units
                            && self.value(
                                target,
                                property,
                                &computed.to_css_string(),
                                true,
                                depth + 1,
                            ),
                );
            }
            if !computes_units {
                return None;
            }
            let font_size = PropertyDeclarationId::Longhand(LonghandId::FontSize);
            match dependency {
                ValueDependency::Dimension(unit) if is_container_unit(unit) => Some(true),
                ValueDependency::Dimension(unit)
                    if ["em", "ex", "ch", "cap", "ic"]
                        .iter()
                        .any(|relative| unit.eq_ignore_ascii_case(relative)) =>
                {
                    Some(if is_font_size {
                        self.parent_property(target, font_size, depth + 1)
                    } else {
                        self.property(target, font_size, depth + 1)
                    })
                }
                ValueDependency::Dimension(unit) if unit.eq_ignore_ascii_case("lh") => {
                    Some(self.line_height(target, is_font_size || is_line_height, depth + 1))
                }
                ValueDependency::Percentage if is_font_size => {
                    Some(self.parent_property(target, font_size, depth + 1))
                }
                ValueDependency::Percentage if is_line_height => {
                    Some(self.property(target, font_size, depth + 1))
                }
                ValueDependency::Identifier(keyword)
                    if is_font_size
                        && (keyword.eq_ignore_ascii_case("larger")
                            || keyword.eq_ignore_ascii_case("smaller")) =>
                {
                    Some(self.parent_property(target, font_size, depth + 1))
                }
                ValueDependency::Function(name) if name.starts_with("--") => Some(
                    target
                        .style
                        .flags
                        .contains(ComputedValueFlags::USES_CONTAINER_UNITS),
                ),
                _ => None,
            }
        })
    }

    fn line_height(
        &mut self,
        target: &StyledElement<'_, '_>,
        inherited: bool,
        depth: usize,
    ) -> bool {
        if inherited {
            return target
                .inheritance_parent()
                .is_some_and(|parent| self.line_height(&parent, false, depth));
        }
        let property = match target.style.clone_line_height() {
            LineHeight::Length(_) => LonghandId::LineHeight,
            LineHeight::Normal | LineHeight::Number(_) => LonghandId::FontSize,
        };
        self.property(target, PropertyDeclarationId::Longhand(property), depth)
    }
}

fn declaration_keyword(declaration: &PropertyDeclaration) -> Option<CSSWideKeyword> {
    if let PropertyDeclaration::Custom(custom) = declaration {
        return match &custom.value {
            CustomDeclarationValue::CSSWideKeyword(keyword) => Some(*keyword),
            _ => None,
        };
    }
    declaration.get_css_wide_keyword()
}

fn serialized_declaration(
    declaration: &PropertyDeclaration,
    block: &PropertyDeclarationBlock,
) -> Option<String> {
    let mut value = String::new();
    declaration.to_css(&mut value).ok()?;
    if !value.is_empty() || !matches!(declaration, PropertyDeclaration::WithVariables(_)) {
        return Some(value);
    }
    // Pending substitutions from shorthands serialize on the shorthand.
    for shorthand in declaration.id().as_longhand()?.shorthands() {
        block
            .property_value_to_css(&PropertyId::NonCustom(shorthand.into()), &mut value)
            .ok()?;
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

fn winning_declaration<'a>(
    target: &'a StyledElement<'_, '_>,
    property: PropertyDeclarationId<'_>,
    guards: &'a StylesheetGuards<'a>,
) -> Option<(&'a PropertyDeclaration, &'a PropertyDeclarationBlock)> {
    let writing_mode = target.style.writing_mode;
    let restriction = target
        .pseudo
        .and_then(|pseudo| pseudo.property_restriction());
    let mut reverted: Option<(CascadePriority, RevertKind)> = None;
    for node in target.style.rules().self_and_ancestors() {
        let priority = node.cascade_priority();
        let level = priority.cascade_level();
        let origin = level.origin();
        if !priority.flags().is_empty()
            || reverted.is_some_and(|(from, kind)| !from.allows_when_reverted(&priority, kind))
        {
            continue;
        }
        let Some(source) = node.style_source() else {
            continue;
        };
        let block = source.read(origin.guard(guards));
        for (declaration, importance) in block.declaration_importance_iter().rev() {
            let matches = match (declaration.id(), property) {
                (
                    PropertyDeclarationId::Longhand(actual),
                    PropertyDeclarationId::Longhand(requested),
                ) => {
                    actual.to_physical(writing_mode) == requested.to_physical(writing_mode)
                        && (origin == CascadeOrigin::UA
                            || restriction.is_none_or(|flags| actual.flags().contains(flags)))
                }
                (actual, requested) => actual == requested,
            };
            if !matches || importance.important() != level.is_important() {
                continue;
            }
            if let Some(kind) =
                declaration_keyword(declaration).and_then(CSSWideKeyword::revert_kind)
            {
                reverted = Some((priority, kind));
                break;
            }
            return Some((declaration, block));
        }
    }
    None
}

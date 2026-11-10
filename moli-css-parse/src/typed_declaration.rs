//! Declaration-owned numeric values that CSSOM serialization cannot preserve.

use std::collections::HashMap;

use style::moli_declaration_block::CssDeclarationBlock as NativeDeclarationBlock;
use style::properties::{
    PropertyDeclaration, PropertyDeclarationBlock, PropertyDeclarationId, PropertyId,
};

use crate::{CssDeclarationEntry, CssMutationProjection, CssRemoveResult, CssSetResult};

#[derive(Clone, Debug, PartialEq)]
pub struct CssDeclaredUnitValue {
    pub value: f64,
    pub unit: String,
}

/// The native block continues to own CSSOM serialization and cascade values.
/// Typed unit writes additionally retain their double and unit: serializing an
/// opacity percentage to a number, for example, cannot reproduce its Typed OM
/// representation. All declaration mutations invalidate the affected values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CssDeclarationBlock {
    native: NativeDeclarationBlock,
    typed_units: HashMap<&'static str, CssDeclaredUnitValue>,
}

impl CssDeclarationBlock {
    pub fn new(block: PropertyDeclarationBlock) -> Self {
        Self {
            native: NativeDeclarationBlock::new(block),
            typed_units: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.native.is_empty()
    }

    pub fn len(&self) -> usize {
        self.native.len()
    }

    pub fn css_text(&self) -> String {
        let text = self.native.css_text();
        if self.native.property_is_declared("mask-position-x")
            || self.native.property_is_declared("mask-position-y")
        {
            project_cssom_declaration_names(text)
        } else {
            text
        }
    }

    pub fn item(&self, index: usize) -> Option<String> {
        self.native
            .item(index)
            .map(|name| crate::stylo_cssom_property_name(&name).to_owned())
    }

    /// Preserve the native grammar when CSSOM text is reused as style input.
    pub fn project_style_resolution_names(&self, text: String) -> String {
        self.native.project_style_resolution_names(text)
    }

    pub fn style_resolution_text(&self) -> String {
        self.project_style_resolution_names(self.css_text())
    }

    pub fn entries(&self) -> Vec<CssDeclarationEntry> {
        let mut entries = self.native.entries();
        for entry in &mut entries {
            project_cssom_name(&mut entry.name);
        }
        entries
    }

    /// Copy longhand and custom declarations in the requested order without
    /// reparsing their CSSOM values. Pending substitution serializes as an
    /// empty string, so that string cannot reconstruct its declaration.
    pub fn select_properties<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Option<Self> {
        let source = self.native.clone().into_inner();
        let mut selected = PropertyDeclarationBlock::new();
        for name in names {
            let property = PropertyId::parse_enabled_for_all_content(name).ok()?;
            let declaration_id = property.as_shorthand().err()?;
            let (declaration, importance) = source.get(declaration_id)?;
            selected.push(declaration.clone(), importance);
        }
        let native = NativeDeclarationBlock::new(selected);
        let mut typed_units = self.typed_units.clone();
        typed_units.retain(|name, _| native.property_is_declared(name));
        Some(Self {
            native,
            typed_units,
        })
    }

    pub fn property_value(&self, name: &str) -> Option<String> {
        self.native.property_value(name)
    }

    pub fn property_priority(&self, name: &str) -> bool {
        self.native.property_priority(name)
    }

    pub fn property_is_declared(&self, name: &str) -> bool {
        self.native.property_is_declared(name)
    }

    /// Whether a query includes declarations retaining unparsed substitution
    /// tokens, rather than ordinary values held by a compatibility adapter.
    pub fn property_has_unparsed_value(&self, name: &str) -> bool {
        let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
            return false;
        };
        let source = self.native.clone().into_inner();
        let unparsed = |id| {
            source.get(id).is_some_and(|(declaration, _)| {
                matches!(declaration, PropertyDeclaration::WithVariables(..))
            })
        };
        match property.as_shorthand() {
            Ok(shorthand) => shorthand
                .longhands()
                .any(|longhand| unparsed(PropertyDeclarationId::Longhand(longhand))),
            Err(id) => unparsed(id),
        }
    }

    pub fn affected_names_for_property(name: &str) -> Option<Vec<String>> {
        let mut names = NativeDeclarationBlock::affected_names_for_property(name)?;
        names.iter_mut().for_each(project_cssom_name);
        Some(names)
    }

    pub fn set_property(&mut self, name: &str, value: &str, priority: bool) -> CssSetResult {
        self.set_property_with_projection(name, value, priority)
            .set_result
    }

    pub fn set_property_with_projection(
        &mut self,
        name: &str,
        value: &str,
        priority: bool,
    ) -> CssMutationProjection {
        let mut result = self
            .native
            .set_property_with_projection(name, value, priority);
        if result.set_result != CssSetResult::ParseError {
            self.forget_typed_units(&result.affected_names);
        }
        for entry in &mut result.entries {
            project_cssom_name(&mut entry.name);
        }
        result
            .affected_names
            .iter_mut()
            .for_each(project_cssom_name);
        result.stored_names.iter_mut().for_each(project_cssom_name);
        result
    }

    pub fn remove_property(&mut self, name: &str) -> CssRemoveResult {
        let result = self.native.remove_property(name);
        if !self.typed_units.is_empty()
            && let Some(affected) = NativeDeclarationBlock::affected_names_for_property(name)
        {
            self.forget_typed_units(&affected);
        }
        result
    }

    pub fn into_inner(self) -> PropertyDeclarationBlock {
        self.native.into_inner()
    }

    pub fn typed_unit_value(&self, name: &str) -> Option<&CssDeclaredUnitValue> {
        if self.typed_units.is_empty() {
            return None;
        }
        let key = PropertyId::parse_enabled_for_all_content(name)
            .ok()?
            .longhand_id()?
            .name();
        self.typed_units.get(key)
    }

    /// Attach the already-validated Typed OM value to its native declaration.
    /// This never creates a declaration or bypasses its property grammar.
    pub fn retain_typed_unit_value(&mut self, name: &str, value: CssDeclaredUnitValue) -> bool {
        if !value.value.is_finite() || !self.native.property_is_declared(name) {
            return false;
        }
        let Some(id) = PropertyId::parse_enabled_for_all_content(name)
            .ok()
            .and_then(|id| id.longhand_id())
        else {
            return false;
        };
        self.typed_units.insert(id.name(), value);
        true
    }

    /// Compatibility declarations sometimes rebuild the native block. Carry
    /// over only values whose declaration survived, excluding the property the
    /// caller actually wrote (even if its serialization did not change).
    pub fn retain_unaffected_typed_units(&mut self, previous: &Self, changed_property: &str) {
        if previous.typed_units.is_empty() {
            return;
        }
        let affected = NativeDeclarationBlock::affected_names_for_property(changed_property)
            .unwrap_or_default();
        for (&name, value) in &previous.typed_units {
            if name != changed_property
                && !affected.iter().any(|affected| affected == name)
                && self.native.property_is_declared(name)
                && self.native.property_value(name) == previous.native.property_value(name)
            {
                self.typed_units.insert(name, value.clone());
            }
        }
    }

    /// A compatibility declaration stored alongside this block can override
    /// its longhands without replacing the block itself.
    pub fn invalidate_typed_units_for_property(&mut self, name: &str) -> bool {
        let before = self.typed_units.len();
        if before != 0
            && let Some(affected) = NativeDeclarationBlock::affected_names_for_property(name)
        {
            self.forget_typed_units(&affected);
        }
        self.typed_units.len() != before
    }

    fn forget_typed_units(&mut self, affected: &[String]) {
        if self.typed_units.is_empty() {
            return;
        }
        for name in affected {
            self.typed_units.remove(name.as_str());
        }
    }
}

fn project_cssom_name(name: &mut String) {
    let public = crate::stylo_cssom_property_name(name);
    if public != name.as_str() {
        *name = public.to_owned();
    }
}

/// Change only declaration-name token ranges in native serialization. Values
/// stay byte-for-byte intact, including custom tokens and pending substitution.
fn project_cssom_declaration_names(mut text: String) -> String {
    let mut parser = cssparser::Parser::new(&text);
    let mut declaration_start = true;
    let mut replacements = Vec::new();
    loop {
        let start = parser.position().byte_index();
        let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        match token {
            cssparser::Token::WhiteSpace(_) | cssparser::Token::Comment(_) => {}
            cssparser::Token::Semicolon => declaration_start = true,
            cssparser::Token::Ident(name) if declaration_start => {
                let public = crate::stylo_cssom_property_name(&name);
                if public != name.as_ref() {
                    replacements.push((start..parser.position().byte_index(), public.to_owned()));
                }
                declaration_start = false;
            }
            _ => declaration_start = false,
        }
    }
    drop(parser);
    for (range, name) in replacements.into_iter().rev() {
        text.replace_range(range, &name);
    }
    text
}

pub fn parse_declaration_block(css_text: &str) -> CssDeclarationBlock {
    CssDeclarationBlock {
        native: style::moli_declaration_block::parse_declaration_block(css_text),
        typed_units: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cssom_mask_axis_names_leave_value_tokens_intact() {
        let text = "mask-position-x: 10px !important; --tokens: \"mask-position-y: keep; mask-position-x: keep\"; mask-position-y: var(--y); --nested: fn(mask-position-x; mask-position-y);";
        assert_eq!(
            project_cssom_declaration_names(text.to_owned()),
            "-webkit-mask-position-x: 10px !important; --tokens: \"mask-position-y: keep; mask-position-x: keep\"; -webkit-mask-position-y: var(--y); --nested: fn(mask-position-x; mask-position-y);"
        );
    }

    #[test]
    fn selecting_declarations_preserves_pending_values_and_typed_units() {
        let mut block = parse_declaration_block(
            "border: var(--edge) !important; color: blue; opacity: 0.25; --empty: ;",
        );
        block.retain_typed_unit_value(
            "opacity",
            CssDeclaredUnitValue {
                value: 25.0,
                unit: "percent".to_owned(),
            },
        );
        let names = block
            .entries()
            .into_iter()
            .filter(|entry| entry.name != "color")
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        let selected = block
            .select_properties(names.iter().map(String::as_str))
            .expect("declared longhands and custom properties can be selected");
        assert_eq!(
            selected.property_value("border").as_deref(),
            Some("var(--edge)")
        );
        assert_eq!(selected.property_value("border-color").as_deref(), Some(""));
        assert!(selected.property_priority("border"));
        assert!(selected.property_has_unparsed_value("border"));
        assert!(selected.property_has_unparsed_value("border-top-color"));
        assert!(!selected.property_has_unparsed_value("opacity"));
        assert!(!selected.property_is_declared("color"));
        assert!(selected.property_is_declared("--empty"));
        assert_eq!(
            selected.typed_unit_value("opacity"),
            block.typed_unit_value("opacity")
        );
        assert!(
            selected
                .css_text()
                .contains("border: var(--edge) !important;")
        );
        assert!(block.property_is_declared("color"));
        assert!(block.select_properties(["border"]).is_none());
        assert!(block.select_properties(["outline-width"]).is_none());
    }

    #[test]
    fn font_shorthand_remains_usable_without_optional_size_adjust_support() {
        assert!(!crate::stylo_style_rule_property_name_is_enabled(
            "font-size-adjust"
        ));
        let mut block = parse_declaration_block("font: 16px serif");
        assert_eq!(block.property_value("font").as_deref(), Some("16px serif"));
        assert_ne!(
            block.set_property("font", "12px sans-serif", false),
            CssSetResult::ParseError
        );
        assert_eq!(
            block.property_value("font").as_deref(),
            Some("12px sans-serif")
        );
    }

    #[test]
    fn typed_units_follow_native_declaration_mutations() {
        let mut block = parse_declaration_block("opacity: 25%; margin-left: 1px");
        let percent = CssDeclaredUnitValue {
            value: 25.0,
            unit: "percent".into(),
        };
        assert!(block.retain_typed_unit_value("opacity", percent.clone()));
        assert!(block.retain_typed_unit_value(
            "margin-left",
            CssDeclaredUnitValue {
                value: 1.1234567891234567,
                unit: "px".into(),
            }
        ));
        let snapshot = block.clone();
        assert_eq!(block.property_value("opacity").as_deref(), Some("0.25"));
        block.set_property("color", "red", false);
        assert_eq!(block.typed_unit_value("opacity"), Some(&percent));
        assert_eq!(
            block.set_property("opacity", "invalid", false),
            CssSetResult::ParseError
        );
        assert_eq!(block.typed_unit_value("opacity"), Some(&percent));
        block.set_property("margin", "2px", false);
        assert!(block.typed_unit_value("margin-left").is_none());
        assert_eq!(block.typed_unit_value("opacity"), Some(&percent));
        block.set_property("opacity", "0.25", false);
        assert!(block.typed_unit_value("opacity").is_none());
        assert_eq!(snapshot.typed_unit_value("opacity"), Some(&percent));
        assert!(block.retain_typed_unit_value("opacity", percent));
        block.remove_property("all");
        assert!(block.typed_units.is_empty());
        assert!(!block.retain_typed_unit_value(
            "opacity",
            CssDeclaredUnitValue {
                value: 1.0,
                unit: "number".into(),
            }
        ));
    }
}

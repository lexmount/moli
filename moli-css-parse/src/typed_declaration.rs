//! Declaration-owned numeric values that CSSOM serialization cannot preserve.

use std::collections::HashMap;

use style::moli_declaration_block::CssDeclarationBlock as NativeDeclarationBlock;
use style::properties::{PropertyDeclarationBlock, PropertyId};

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
        self.native.css_text()
    }

    pub fn item(&self, index: usize) -> Option<String> {
        self.native.item(index)
    }

    pub fn entries(&self) -> Vec<CssDeclarationEntry> {
        self.native.entries()
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

    pub fn affected_names_for_property(name: &str) -> Option<Vec<String>> {
        NativeDeclarationBlock::affected_names_for_property(name)
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
        let result = self
            .native
            .set_property_with_projection(name, value, priority);
        if result.set_result != CssSetResult::ParseError {
            self.forget_typed_units(&result.affected_names);
        }
        result
    }

    pub fn remove_property(&mut self, name: &str) -> CssRemoveResult {
        let result = self.native.remove_property(name);
        if !self.typed_units.is_empty()
            && let Some(affected) = Self::affected_names_for_property(name)
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
        let affected = Self::affected_names_for_property(changed_property).unwrap_or_default();
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
            && let Some(affected) = Self::affected_names_for_property(name)
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

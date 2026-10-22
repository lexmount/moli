//! Declaration-owned numeric values that CSSOM serialization cannot preserve.

use std::{collections::HashMap, sync::Arc};

use style::moli_declaration_block::CssDeclarationBlock as NativeDeclarationBlock;
use style::properties::{PropertyDeclarationBlock, PropertyId};

use crate::{CssDeclarationEntry, CssMutationProjection, CssRemoveResult, CssSetResult};

#[derive(Clone, Debug, PartialEq)]
pub struct CssDeclaredUnitValue {
    pub value: f64,
    pub unit: String,
}

/// The operators exposed by CSSMathValue, in their native binding order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CssMathOperator {
    Sum,
    Product,
    Negate,
    Invert,
    Min,
    Max,
    Clamp,
}

impl CssMathOperator {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Product => "product",
            Self::Negate => "negate",
            Self::Invert => "invert",
            Self::Min => "min",
            Self::Max => "max",
            Self::Clamp => "clamp",
        }
    }

    pub fn arity(self) -> Option<usize> {
        match self {
            Self::Negate | Self::Invert => Some(1),
            Self::Clamp => Some(3),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CssDeclaredMathNode {
    Unit(CssDeclaredUnitValue),
    Math(CssMathOperator, Vec<usize>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum CssDeclaredNumericValue {
    Unit(CssDeclaredUnitValue),
    /// Index 0 is the root. A flat graph avoids recursive cloning and drops.
    Math(Arc<[CssDeclaredMathNode]>),
}

/// Values supplied as Typed OM objects, indexed by declaration iteration.
/// Missing entries use the native declaration's regular projection.
pub type CssDeclaredNumericValues = Vec<Option<CssDeclaredNumericValue>>;

/// The native block owns CSSOM serialization and cascade values. Typed OM
/// writes can additionally retain units, operand order and source precision.
/// Each declaration mutation invalidates the affected retained values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CssDeclarationBlock {
    native: NativeDeclarationBlock,
    typed_numeric: HashMap<&'static str, CssDeclaredNumericValues>,
}

impl CssDeclarationBlock {
    pub fn new(block: PropertyDeclarationBlock) -> Self {
        Self {
            native: NativeDeclarationBlock::new(block),
            typed_numeric: HashMap::new(),
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
            self.forget_typed_numeric_values(&result.affected_names);
        }
        result
    }

    pub fn remove_property(&mut self, name: &str) -> CssRemoveResult {
        let result = self.native.remove_property(name);
        if !self.typed_numeric.is_empty()
            && let Some(affected) = Self::affected_names_for_property(name)
        {
            self.forget_typed_numeric_values(&affected);
        }
        result
    }

    pub fn into_inner(self) -> PropertyDeclarationBlock {
        self.native.into_inner()
    }

    pub fn typed_numeric_values(&self, name: &str) -> Option<&CssDeclaredNumericValues> {
        if self.typed_numeric.is_empty() {
            return None;
        }
        let key = typed_numeric_property_key(name)?;
        self.typed_numeric.get(key)
    }

    /// Attach already-validated values without changing the native declaration.
    pub fn retain_typed_numeric_values(
        &mut self,
        name: &str,
        values: CssDeclaredNumericValues,
    ) -> bool {
        if values.iter().all(Option::is_none)
            || values.iter().flatten().any(|value| {
                matches!(value, CssDeclaredNumericValue::Unit(unit) if !unit.value.is_finite())
            })
            || !self.native.property_is_declared(name)
        {
            return false;
        }
        let Some(key) = typed_numeric_property_key(name) else {
            return false;
        };
        self.typed_numeric.insert(key, values);
        true
    }

    /// Compatibility declarations sometimes rebuild the native block. Carry
    /// over only values whose declaration survived, excluding the property the
    /// caller actually wrote (even if its serialization did not change).
    pub fn retain_unaffected_typed_numeric_values(
        &mut self,
        previous: &Self,
        changed_property: &str,
    ) {
        if previous.typed_numeric.is_empty() {
            return;
        }
        let affected = Self::affected_names_for_property(changed_property).unwrap_or_default();
        for (&name, value) in &previous.typed_numeric {
            if name != changed_property
                && !affected.iter().any(|affected| affected == name)
                && self.native.property_is_declared(name)
                && self.native.property_value(name) == previous.native.property_value(name)
            {
                self.typed_numeric.insert(name, value.clone());
            }
        }
    }

    /// A compatibility declaration stored alongside this block can override
    /// its longhands without replacing the block itself.
    pub fn invalidate_typed_numeric_for_property(&mut self, name: &str) -> bool {
        let before = self.typed_numeric.len();
        if before != 0
            && let Some(affected) = Self::affected_names_for_property(name)
        {
            self.forget_typed_numeric_values(&affected);
        }
        self.typed_numeric.len() != before
    }

    fn forget_typed_numeric_values(&mut self, affected: &[String]) {
        if self.typed_numeric.is_empty() {
            return;
        }
        for name in affected {
            self.typed_numeric.remove(name.as_str());
        }
    }
}

fn typed_numeric_property_key(name: &str) -> Option<&'static str> {
    Some(
        PropertyId::parse_enabled_for_all_content(name)
            .ok()?
            .longhand_id()?
            .name(),
    )
}

pub fn parse_declaration_block(css_text: &str) -> CssDeclarationBlock {
    CssDeclarationBlock {
        native: style::moli_declaration_block::parse_declaration_block(css_text),
        typed_numeric: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retain_unit(
        block: &mut CssDeclarationBlock,
        name: &str,
        unit: CssDeclaredUnitValue,
    ) -> bool {
        block.retain_typed_numeric_values(name, vec![Some(CssDeclaredNumericValue::Unit(unit))])
    }

    fn typed_unit<'a>(
        block: &'a CssDeclarationBlock,
        name: &str,
    ) -> Option<&'a CssDeclaredUnitValue> {
        let [Some(CssDeclaredNumericValue::Unit(unit))] =
            block.typed_numeric_values(name)?.as_slice()
        else {
            return None;
        };
        Some(unit)
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
    fn typed_numbers_follow_native_declaration_mutations() {
        let mut block = parse_declaration_block("opacity: 25%; margin-left: 1px");
        let percent = CssDeclaredUnitValue {
            value: 25.0,
            unit: "percent".into(),
        };
        assert!(retain_unit(&mut block, "opacity", percent.clone()));
        assert!(retain_unit(
            &mut block,
            "margin-left",
            CssDeclaredUnitValue {
                value: 1.1234567891234567,
                unit: "px".into(),
            }
        ));
        let snapshot = block.clone();
        assert_eq!(block.property_value("opacity").as_deref(), Some("0.25"));
        block.set_property("color", "red", false);
        assert_eq!(typed_unit(&block, "opacity"), Some(&percent));
        assert_eq!(
            block.set_property("opacity", "invalid", false),
            CssSetResult::ParseError
        );
        assert_eq!(typed_unit(&block, "opacity"), Some(&percent));
        block.set_property("margin", "2px", false);
        assert!(typed_unit(&block, "margin-left").is_none());
        assert_eq!(typed_unit(&block, "opacity"), Some(&percent));
        block.set_property("opacity", "0.25", false);
        assert!(typed_unit(&block, "opacity").is_none());
        assert_eq!(typed_unit(&snapshot, "opacity"), Some(&percent));
        assert!(retain_unit(&mut block, "opacity", percent));
        block.remove_property("all");
        assert!(block.typed_numeric.is_empty());
        assert!(!retain_unit(
            &mut block,
            "opacity",
            CssDeclaredUnitValue {
                value: 1.0,
                unit: "number".into(),
            }
        ));
    }
}

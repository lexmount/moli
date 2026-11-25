use crate::{ParsedCssomProperty, cssom_property::with_detached_style_context};
use cssparser::Parser;
use style::{
    moli_declaration_block::{
        CssDeclarationBlock as NativeBlock, CssDeclarationEntry, CssMutationProjection,
        CssRemoveResult, CssSetResult,
    },
    properties::{
        Importance, PropertyDeclaration, PropertyDeclarationBlock, PropertyDeclarationId,
        PropertyId, parse_property_declaration_list,
    },
    servo_arc::Arc,
    shared_lock::{Locked, SharedRwLock},
};
use style_traits::CssString;

/// A writable CSSOM block or the immutable block already shared with Stylo.
#[derive(Clone, Debug)]
pub struct CssDeclarationBlock(BlockStorage);

#[derive(Clone, Debug)]
enum BlockStorage {
    Owned(PropertyDeclarationBlock),
    Published(Arc<Locked<PropertyDeclarationBlock>>),
}

impl Default for CssDeclarationBlock {
    fn default() -> Self {
        Self::new(PropertyDeclarationBlock::new())
    }
}

impl PartialEq for CssDeclarationBlock {
    fn eq(&self, other: &Self) -> bool {
        self.with_declarations(|left| other.with_declarations(|right| left == right))
    }
}

impl CssDeclarationBlock {
    pub fn new(block: PropertyDeclarationBlock) -> Self {
        Self(BlockStorage::Owned(block))
    }

    pub fn into_inner(self) -> PropertyDeclarationBlock {
        match self.0 {
            BlockStorage::Owned(block) => block,
            BlockStorage::Published(block) => {
                block.read_with(&SharedRwLock::read_only().read()).clone()
            }
        }
    }

    pub fn publish(&mut self) -> Arc<Locked<PropertyDeclarationBlock>> {
        if let BlockStorage::Published(block) = &self.0 {
            return block.clone();
        }
        // A read-only Stylo lock permits use with the consuming style system's
        // read guard. No selector or stylesheet can mutate these declarations.
        let block = Arc::new(SharedRwLock::read_only().wrap(std::mem::take(self).into_inner()));
        self.0 = BlockStorage::Published(block.clone());
        block
    }

    pub(crate) fn with_declarations<T>(
        &self,
        read: impl FnOnce(&PropertyDeclarationBlock) -> T,
    ) -> T {
        match &self.0 {
            BlockStorage::Owned(block) => read(block),
            BlockStorage::Published(block) => {
                read(block.read_with(&SharedRwLock::read_only().read()))
            }
        }
    }

    fn mutate<T>(&mut self, operation: impl FnOnce(&mut PropertyDeclarationBlock) -> T) -> T {
        let mut block = std::mem::take(self).into_inner();
        let result = operation(&mut block);
        *self = Self::new(block);
        result
    }

    pub fn is_empty(&self) -> bool {
        self.with_declarations(PropertyDeclarationBlock::is_empty)
    }

    pub fn len(&self) -> usize {
        self.with_declarations(PropertyDeclarationBlock::len)
    }

    pub fn css_text(&self) -> String {
        self.with_declarations(serialize_block)
    }

    pub fn item(&self, index: usize) -> Option<String> {
        self.with_declarations(|block| {
            let (declaration, _) = block.declaration_importance_iter().nth(index)?;
            Some(declaration_name(declaration.id()))
        })
    }

    pub fn entries(&self) -> Vec<CssDeclarationEntry> {
        self.with_declarations(|block| {
            block
                .declaration_importance_iter()
                .filter_map(|(declaration, importance)| declaration_entry(declaration, importance))
                .collect()
        })
    }

    pub fn property_value(&self, name: &str) -> Option<String> {
        let property = PropertyId::parse_enabled_for_all_content(name).ok()?;
        Some(self.with_declarations(|block| property_value_by_id(block, &property)))
    }

    pub fn property_priority(&self, name: &str) -> bool {
        let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
            return false;
        };
        self.with_declarations(|block| block.property_priority(&property).important())
    }

    pub fn property_is_declared(&self, name: &str) -> bool {
        let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
            return false;
        };
        self.with_declarations(|block| block.first_declaration_to_remove(&property).is_some())
    }

    pub fn affected_names_for_property(name: &str) -> Option<Vec<String>> {
        NativeBlock::affected_names_for_property(name)
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
        let projection = with_detached_style_context(|context| {
            let property = PropertyId::parse(name, context).ok()?;
            let affected_names = Self::affected_names_for_property(name)?;
            if value.is_empty() {
                let result = self.remove_property_by_id(&property);
                return Some(CssMutationProjection {
                    set_result: if result.changed {
                        CssSetResult::ChangedPropertySet
                    } else {
                        CssSetResult::Unchanged
                    },
                    entries: Vec::new(),
                    affected_names,
                    stored_names: Vec::new(),
                    has_unresolved_value: false,
                });
            }
            let parsed = ParsedCssomProperty::parse_value(property, value, priority, context)?;
            let entries = parsed.entries()?;
            let has_unresolved_value = parsed.has_unresolved_value();
            let before = self.css_text();
            let modified_existing = self
                .with_declarations(|b| b.first_declaration_to_remove(parsed.property()).is_some());
            self.mutate(|block| parsed.replace_in_declaration_list(block));
            let set_result = if self.css_text() == before {
                CssSetResult::Unchanged
            } else if modified_existing {
                CssSetResult::ModifiedExisting
            } else {
                CssSetResult::ChangedPropertySet
            };
            Some(CssMutationProjection {
                set_result,
                affected_names,
                stored_names: entries.iter().map(|entry| entry.name.clone()).collect(),
                entries,
                has_unresolved_value,
            })
        });
        projection.unwrap_or_else(|| CssMutationProjection {
            set_result: CssSetResult::ParseError,
            affected_names: Vec::new(),
            entries: Vec::new(),
            stored_names: Vec::new(),
            has_unresolved_value: false,
        })
    }

    pub fn remove_property(&mut self, name: &str) -> CssRemoveResult {
        let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
            return CssRemoveResult::default();
        };
        self.remove_property_by_id(&property)
    }

    fn remove_property_by_id(&mut self, property: &PropertyId) -> CssRemoveResult {
        let Some(first) =
            self.with_declarations(|block| block.first_declaration_to_remove(property))
        else {
            return CssRemoveResult::default();
        };
        let old_value = self.with_declarations(|block| property_value_by_id(block, property));
        self.mutate(|block| block.remove_property(property, first));
        CssRemoveResult {
            changed: true,
            old_value: Some(old_value),
        }
    }
}

fn property_value_by_id(block: &PropertyDeclarationBlock, property: &PropertyId) -> String {
    let mut value = CssString::new();
    let _ = block.property_value_to_css(property, &mut value);
    value
}

fn serialize_block(block: &PropertyDeclarationBlock) -> String {
    let mut output = CssString::new();
    let _ = block.to_css(&mut output);
    output
}

fn declaration_name(property: PropertyDeclarationId<'_>) -> String {
    match property.name().as_ref() {
        "font-width" => "font-stretch".to_owned(),
        name => name.to_owned(),
    }
}

pub(crate) fn declaration_entry(
    declaration: &PropertyDeclaration,
    importance: Importance,
) -> Option<CssDeclarationEntry> {
    let mut value = CssString::new();
    declaration.to_css(&mut value).ok()?;
    Some(CssDeclarationEntry {
        name: declaration_name(declaration.id()),
        value,
        priority: importance.important(),
    })
}

pub fn parse_declaration_block(text: &str) -> CssDeclarationBlock {
    with_detached_style_context(|context| {
        let block = parse_property_declaration_list(context, &mut Parser::new(text), &[]);
        CssDeclarationBlock::new(block)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaration_projection_matches_the_stylo_mutation_api() {
        crate::ensure_stylo_browser_compat_prefs();
        for (source, name, value, important) in [
            ("width: 10px; color: red", "width", "10px", false),
            ("width: 10px !important", "width", "10px", false),
            ("width: 10px", "width", "10px\u{00a0}", false),
            ("width: 10px", "width", "", false),
            ("color: red", "width", "", false),
            (
                "margin-left: 3px; margin-top: 6px",
                "margin",
                "10px 2px",
                true,
            ),
            (
                "border-image: url(a.png) 3",
                "border",
                "1px solid red",
                false,
            ),
            (
                "background-image: url(a.png)",
                "background-image",
                "url(./a.png)",
                false,
            ),
            (
                "--Token: old !important",
                "--Token",
                "var(--data, red)",
                false,
            ),
            ("--Token: old", "--Token", " ", false),
            ("", "all", "initial", false),
            ("", "all", "var(--values)", true),
        ] {
            let mut block = parse_declaration_block(source);
            let mut oracle = style::moli_declaration_block::parse_declaration_block(source);
            block.publish();
            assert_eq!(
                block.set_property_with_projection(name, value, important),
                oracle.set_property_with_projection(name, value, important),
                "{source:?} with {name}: {value:?}",
            );
            assert_eq!(block.css_text(), oracle.css_text(), "{name}: {value:?}");
            assert_eq!(block.remove_property(name), oracle.remove_property(name));
            assert_eq!(block.css_text(), oracle.css_text());
        }
    }

    #[test]
    fn published_block_keeps_its_values_after_the_next_edit() {
        let mut block = parse_declaration_block("width: 10px; height: 20px;");
        let published = block.publish();
        assert!(Arc::ptr_eq(&published, &block.publish()));
        assert_eq!(block.property_value("width").as_deref(), Some("10px"));
        assert!(block.set_property("width", "30px", false) != CssSetResult::ParseError);
        let next = block.publish();
        assert!(!Arc::ptr_eq(&published, &next));
        let lock = SharedRwLock::new();
        let guard = lock.read();
        assert_eq!(
            serialize_block(published.read_with(&guard)),
            "width: 10px; height: 20px;"
        );
        assert_eq!(
            serialize_block(next.read_with(&guard)),
            "height: 20px; width: 30px;"
        );
    }
}

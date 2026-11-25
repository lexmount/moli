use style::{
    moli_declaration_block::{
        CssDeclarationBlock as NativeBlock, CssDeclarationEntry, CssMutationProjection,
        CssRemoveResult, CssSetResult,
    },
    properties::{PropertyDeclarationBlock, PropertyDeclarationId, PropertyId},
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

    fn mutate<T>(&mut self, operation: impl FnOnce(&mut NativeBlock) -> T) -> T {
        let mut block = NativeBlock::new(std::mem::take(self).into_inner());
        let result = operation(&mut block);
        *self = Self::new(block.into_inner());
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
                .filter_map(|(declaration, importance)| {
                    let mut value = CssString::new();
                    declaration.to_css(&mut value).ok()?;
                    Some(CssDeclarationEntry {
                        name: declaration_name(declaration.id()),
                        value,
                        priority: importance.important(),
                    })
                })
                .collect()
        })
    }

    pub fn property_value(&self, name: &str) -> Option<String> {
        let property = PropertyId::parse_enabled_for_all_content(name).ok()?;
        Some(self.with_declarations(|block| {
            let mut value = CssString::new();
            let _ = block.property_value_to_css(&property, &mut value);
            value
        }))
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
        self.mutate(|block| block.set_property(name, value, priority))
    }

    pub fn set_property_with_projection(
        &mut self,
        name: &str,
        value: &str,
        priority: bool,
    ) -> CssMutationProjection {
        self.mutate(|block| block.set_property_with_projection(name, value, priority))
    }

    pub fn remove_property(&mut self, name: &str) -> CssRemoveResult {
        self.mutate(|block| block.remove_property(name))
    }
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

pub fn parse_declaration_block(text: &str) -> CssDeclarationBlock {
    let block = style::moli_declaration_block::parse_declaration_block(text);
    CssDeclarationBlock::new(block.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

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

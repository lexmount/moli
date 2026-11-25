use crate::{
    CssDeclarationBlock, CssDeclarationEntry, cssom_block::declaration_entry,
    declaration::split_declaration_priority,
};
use cssparser::Parser;
use style::{
    context::QuirksMode,
    parser::ParserContext,
    properties::{
        AllShorthand, Importance, PropertyDeclaration, PropertyDeclarationBlock,
        PropertyDeclarationId, PropertyId, SourcePropertyDeclaration,
        SourcePropertyDeclarationUpdate,
    },
    stylesheets::{CssRuleType, Origin, UrlExtraData},
};
use style_traits::{CssString, ParsingMode};

/// One parsed CSSOM declaration, including the longhands of a shorthand.
pub struct ParsedCssomProperty {
    property: PropertyId,
    source: SourcePropertyDeclaration,
    importance: Importance,
}

impl ParsedCssomProperty {
    pub fn parse_without_document(name: &str, value: &str, important: bool) -> Option<Self> {
        let url_data = UrlExtraData::from(url::Url::parse("about:blank").unwrap());
        Self::parse(name, value, important, &url_data, QuirksMode::NoQuirks)
    }

    pub fn parse(
        name: &str,
        value: &str,
        important: bool,
        url_data: &UrlExtraData,
        quirks_mode: QuirksMode,
    ) -> Option<Self> {
        if value.is_empty()
            || (value.contains(';') && crate::escape_top_level_semicolons(value) != value)
            || split_declaration_priority(value).is_some_and(|(_, important)| important)
        {
            return None;
        }
        let context = ParserContext::new(
            Origin::Author,
            url_data,
            Some(CssRuleType::Style),
            ParsingMode::DEFAULT,
            quirks_mode,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        let property = PropertyId::parse(name, &context).ok()?;
        let mut source = SourcePropertyDeclaration::default();
        Parser::new(value)
            .parse_entirely(|input| {
                PropertyDeclaration::parse_into(&mut source, property.clone(), &context, input)
            })
            .ok()?;
        Some(Self {
            property,
            source,
            importance: if important {
                Importance::Important
            } else {
                Importance::Normal
            },
        })
    }

    pub fn is_ordinary_declaration_list(&self) -> bool {
        !self.source.declarations.is_empty() && !self.has_unresolved_value()
    }

    pub fn has_unresolved_value(&self) -> bool {
        matches!(self.source.all_shorthand, AllShorthand::WithVariables(_))
            || self
                .source
                .declarations
                .iter()
                .any(|declaration| matches!(declaration, PropertyDeclaration::WithVariables(_)))
    }

    pub fn entries(&self) -> Option<Vec<CssDeclarationEntry>> {
        if !matches!(self.source.all_shorthand, AllShorthand::NotSet) {
            let value = self.value()?;
            return (!value.is_empty()).then(|| {
                vec![CssDeclarationEntry {
                    name: "all".to_owned(),
                    value,
                    priority: self.importance.important(),
                }]
            });
        }
        self.source
            .declarations
            .iter()
            .map(|declaration| declaration_entry(declaration, self.importance))
            .collect()
    }

    pub fn value(&self) -> Option<String> {
        let mut block = PropertyDeclarationBlock::new();
        for declaration in self.source.declarations.iter().cloned() {
            block.push(declaration, self.importance);
        }
        for declaration in self.source.all_shorthand.declarations() {
            block.push(declaration, self.importance);
        }
        block.first_declaration_to_remove(&self.property)?;
        let mut value = CssString::new();
        block
            .property_value_to_css(&self.property, &mut value)
            .ok()?;
        Some(value)
    }

    pub fn is_single_declaration(&self) -> bool {
        self.source.declarations.len() == 1
    }

    pub fn append_to_declaration_list(mut self, target: &mut PropertyDeclarationBlock) {
        target.extend(self.source.drain(), self.importance);
    }

    pub fn apply(
        mut self,
        target: &mut CssDeclarationBlock,
        compare_single_longhand: bool,
    ) -> bool {
        let mut updates = SourcePropertyDeclarationUpdate::default();
        if compare_single_longhand
            && self.is_single_declaration()
            && self.is_ordinary_declaration_list()
        {
            let in_place = target.with_declarations(|block| {
                block.prepare_for_update(&self.source, self.importance, &mut updates)
            });
            if in_place {
                // Stylo keeps independent longhands in place and moves declarations
                // across conflicting logical/physical declarations when needed.
                let mut block = std::mem::take(target).into_inner();
                block.update(self.source.drain(), self.importance, &mut updates);
                *target = CssDeclarationBlock::new(block);
                return true;
            }
            let incoming = self.source.declarations.iter().next().unwrap();
            let same_serialization = target.with_declarations(|block| {
                block.declaration_importance_iter().any(|(old, _)| {
                    old.id() == incoming.id() && specified_text(old) == specified_text(incoming)
                })
            });
            if same_serialization {
                return false;
            }
            // Typed equality can hide a difference in specified CSS text, for
            // example two URL spellings that resolve to the same address.
        }
        let mut block = std::mem::take(target).into_inner();
        // Preserve the CSSOM order and priority of the existing append path.
        if !matches!(self.source.all_shorthand, AllShorthand::NotSet) {
            remove(&mut block, &self.property);
        } else {
            for declaration in &self.source.declarations {
                let property = match declaration.id() {
                    PropertyDeclarationId::Longhand(id) => PropertyId::NonCustom(id.into()),
                    PropertyDeclarationId::Custom(name) => PropertyId::Custom(name.clone()),
                };
                remove(&mut block, &property);
            }
        }
        block.extend(self.source.drain(), self.importance);
        *target = CssDeclarationBlock::new(block);
        true
    }
}

fn remove(block: &mut PropertyDeclarationBlock, property: &PropertyId) {
    if let Some(index) = block.first_declaration_to_remove(property) {
        block.remove_property(property, index);
    }
}

fn specified_text(declaration: &PropertyDeclaration) -> String {
    let mut text = CssString::new();
    let _ = declaration.to_css(&mut text);
    text
}

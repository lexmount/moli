use std::sync::LazyLock;

use style::{
    context::QuirksMode,
    moli_declaration_block::CssDeclarationBlock,
    properties::{
        Importance, PropertyDeclaration, PropertyId, SourcePropertyDeclaration,
        SourcePropertyDeclarationUpdate, parse_one_declaration_into,
    },
    stylesheets::{CssRuleType, Origin, UrlExtraData},
};
use style_traits::{CssString, ParsingMode};

static CSSOM_URL_DATA: LazyLock<UrlExtraData> = LazyLock::new(|| {
    UrlExtraData::from(url::Url::parse("about:blank").expect("CSSOM parser base URL"))
});

/// A native property write whose declarations can be applied without projecting
/// them to CSS text. Callers handle CSSOM compatibility values separately.
pub struct ParsedCssomProperty {
    source: SourcePropertyDeclaration,
    importance: Importance,
}

impl ParsedCssomProperty {
    pub fn parse(name: &str, value: &str, important: bool) -> Option<Self> {
        if (value.contains(';') && crate::escape_top_level_semicolons(value) != value)
            || crate::split_important_priority(value).1
        {
            return None;
        }
        let property = PropertyId::parse_enabled_for_all_content(name).ok()?;
        let mut source = SourcePropertyDeclaration::default();
        parse_one_declaration_into(
            &mut source,
            property,
            value,
            Origin::Author,
            &CSSOM_URL_DATA,
            None,
            ParsingMode::DEFAULT,
            QuirksMode::NoQuirks,
            CssRuleType::Style,
        )
        .ok()?;
        if source.declarations.is_empty()
            || source.declarations.iter().any(|declaration| {
                declaration.id().as_longhand().is_none()
                    || matches!(declaration, PropertyDeclaration::WithVariables(_))
            })
        {
            return None;
        }
        Some(Self {
            source,
            importance: if important {
                Importance::Important
            } else {
                Importance::Normal
            },
        })
    }

    pub fn apply(
        mut self,
        target: &mut CssDeclarationBlock,
        compare_single_longhand: bool,
    ) -> bool {
        let mut block = std::mem::take(target).into_inner();
        if compare_single_longhand && self.source.declarations.len() == 1 {
            let mut updates = SourcePropertyDeclarationUpdate::default();
            if block.prepare_for_update(&self.source, self.importance, &mut updates) {
                // Stylo keeps independent longhands in place and moves declarations
                // across conflicting logical/physical declarations when needed.
                block.update(self.source.drain(), self.importance, &mut updates);
                *target = CssDeclarationBlock::new(block);
                return true;
            }
            let incoming = self.source.declarations.iter().next().unwrap();
            let same_serialization = block.declaration_importance_iter().any(|(old, _)| {
                old.id() == incoming.id() && specified_text(old) == specified_text(incoming)
            });
            if same_serialization {
                *target = CssDeclarationBlock::new(block);
                return false;
            }
            // Typed equality can hide a difference in specified CSS text, for
            // example two URL spellings that resolve to the same address.
        }
        // Preserve the CSSOM order and priority of the existing append path.
        for declaration in &self.source.declarations {
            let property = PropertyId::NonCustom(declaration.id().as_longhand().unwrap().into());
            if let Some(index) = block.first_declaration_to_remove(&property) {
                block.remove_property(&property, index);
            }
        }
        block.extend(self.source.drain(), self.importance);
        *target = CssDeclarationBlock::new(block);
        true
    }
}

fn specified_text(declaration: &PropertyDeclaration) -> String {
    let mut text = CssString::new();
    let _ = declaration.to_css(&mut text);
    text
}

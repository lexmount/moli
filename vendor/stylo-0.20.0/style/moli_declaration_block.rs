//! Moli-facing CSS declaration block hooks.

use std::borrow::Cow;

use cssparser::{Parser, ParserInput};
use style_traits::{CssString, ParsingMode};

use crate::{
    context::QuirksMode,
    custom_properties::AttrTaint,
    parser::ParserContext,
    properties::{
        parse_one_declaration_into, parse_property_declaration_list, AllShorthand, Importance,
        PropertyDeclaration, PropertyDeclarationBlock, PropertyDeclarationId, PropertyId,
        SourcePropertyDeclaration,
    },
    stylesheets::{CssRuleType, Namespaces, Origin, UrlExtraData},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssDeclarationEntry {
    pub name: String,
    pub value: String,
    pub priority: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CssSetResult {
    ParseError,
    Unchanged,
    ModifiedExisting,
    ChangedPropertySet,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CssRemoveResult {
    pub changed: bool,
    pub old_value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssMutationProjection {
    pub set_result: CssSetResult,
    pub entries: Vec<CssDeclarationEntry>,
    pub affected_names: Vec<String>,
    pub stored_names: Vec<String>,
    pub has_unresolved_value: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CssDeclarationBlock {
    block: PropertyDeclarationBlock,
}

impl CssDeclarationBlock {
    pub fn new(block: PropertyDeclarationBlock) -> Self {
        Self { block }
    }

    pub fn is_empty(&self) -> bool {
        self.block.is_empty()
    }

    pub fn len(&self) -> usize {
        self.block.len()
    }

    pub fn css_text(&self) -> String {
        let mut output = CssString::new();
        self.block.to_css(&mut output).ok();
        output
    }

    pub fn item(&self, index: usize) -> Option<String> {
        let (declaration, _) = self.block.declaration_importance_iter().nth(index)?;
        Some(property_declaration_name(declaration.id()))
    }

    pub fn entries(&self) -> Vec<CssDeclarationEntry> {
        self.block
            .declaration_importance_iter()
            .filter_map(|(declaration, importance)| declaration_entry(declaration, importance))
            .collect()
    }

    pub fn property_value(&self, name: &str) -> Option<String> {
        let property = PropertyId::parse_enabled_for_all_content(name).ok()?;
        Some(self.property_value_by_id(&property))
    }

    pub fn property_priority(&self, name: &str) -> bool {
        let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
            return false;
        };
        self.block.property_priority(&property) == Importance::Important
    }

    pub fn property_is_declared(&self, name: &str) -> bool {
        let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
            return false;
        };
        self.block.first_declaration_to_remove(&property).is_some()
    }

    pub fn affected_names_for_property(name: &str) -> Option<Vec<String>> {
        let property = PropertyId::parse_enabled_for_all_content(name).ok()?;
        Some(affected_names_for_property_id(&property))
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
        let Some(property) = parse_style_context_property_id(name) else {
            return CssMutationProjection::parse_error();
        };
        let affected_names = affected_names_for_property_id(&property);

        if value.is_empty() {
            let result = self.remove_property_by_id(&property);
            return CssMutationProjection {
                set_result: if result.changed {
                    CssSetResult::ChangedPropertySet
                } else {
                    CssSetResult::Unchanged
                },
                entries: Vec::new(),
                affected_names,
                stored_names: Vec::new(),
                has_unresolved_value: false,
            };
        }

        let Some(mut parsed) = ParsedCssPropertyMutation::parse_property(property, value, priority)
        else {
            return CssMutationProjection::parse_error();
        };
        let before_css_text = self.css_text();
        let modified_existing = self
            .block
            .first_declaration_to_remove(&parsed.property)
            .is_some();
        self.remove_property_by_id(&parsed.property);
        self.block
            .extend(parsed.declarations.drain(), parsed.importance);
        let after_css_text = self.css_text();
        let set_result = if after_css_text == before_css_text {
            CssSetResult::Unchanged
        } else if modified_existing {
            CssSetResult::ModifiedExisting
        } else {
            CssSetResult::ChangedPropertySet
        };

        CssMutationProjection {
            set_result,
            affected_names,
            stored_names: parsed
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect(),
            has_unresolved_value: parsed.has_unresolved_value,
            entries: parsed.entries,
        }
    }

    pub fn remove_property(&mut self, name: &str) -> CssRemoveResult {
        let Ok(property) = PropertyId::parse_enabled_for_all_content(name) else {
            return CssRemoveResult::default();
        };
        self.remove_property_by_id(&property)
    }

    pub fn into_inner(self) -> PropertyDeclarationBlock {
        self.block
    }

    fn property_value_by_id(&self, property: &PropertyId) -> String {
        let mut output = CssString::new();
        self.block.property_value_to_css(property, &mut output).ok();
        output
    }

    fn remove_property_by_id(&mut self, property: &PropertyId) -> CssRemoveResult {
        let Some(first_declaration) = self.block.first_declaration_to_remove(property) else {
            return CssRemoveResult::default();
        };
        let old_value = self.property_value_by_id(property);
        self.block.remove_property(property, first_declaration);
        CssRemoveResult {
            changed: true,
            old_value: Some(old_value),
        }
    }
}

impl CssMutationProjection {
    fn parse_error() -> Self {
        Self {
            set_result: CssSetResult::ParseError,
            entries: Vec::new(),
            affected_names: Vec::new(),
            stored_names: Vec::new(),
            has_unresolved_value: false,
        }
    }
}

struct ParsedCssPropertyMutation {
    property: PropertyId,
    importance: Importance,
    declarations: SourcePropertyDeclaration,
    entries: Vec<CssDeclarationEntry>,
    has_unresolved_value: bool,
}

impl ParsedCssPropertyMutation {
    fn parse_property(property: PropertyId, value: &str, priority: bool) -> Option<Self> {
        let importance = if priority {
            Importance::Important
        } else {
            Importance::Normal
        };
        let mut declarations = SourcePropertyDeclaration::default();
        with_declaration_context(|context| {
            parse_one_declaration_into(
                &mut declarations,
                property.clone(),
                value,
                Origin::Author,
                context.url_data,
                None,
                ParsingMode::DEFAULT,
                QuirksMode::NoQuirks,
                CssRuleType::Style,
            )
            .ok()
        })??;
        let entries = source_declaration_entries(&declarations, importance)?;
        let has_unresolved_value = source_declaration_has_unresolved_value(&declarations);
        Some(Self {
            property,
            importance,
            declarations,
            entries,
            has_unresolved_value,
        })
    }
}

fn affected_names_for_property_id(property: &PropertyId) -> Vec<String> {
    let name = property_name(property);
    let mut names = match property.as_shorthand() {
        Ok(shorthand) => {
            let mut names = vec![name.clone()];
            names.extend(
                shorthand
                    .longhands()
                    .map(|longhand| longhand.name().to_owned()),
            );
            names
        },
        Err(id) => vec![id.name().into_owned()],
    };
    append_moli_cssom_affected_names(&name, &mut names);
    names
}

fn append_moli_cssom_affected_names(name: &str, names: &mut Vec<String>) {
    match name {
        "border" => append_unique_name(names, "border-image"),
        "font" => append_unique_name(names, "font-variant"),
        "overscroll-behavior" => {
            append_unique_name(names, "overscroll-behavior-x");
            append_unique_name(names, "overscroll-behavior-y");
        },
        _ => {},
    }
}

fn append_unique_name(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|existing| existing == name) {
        names.push(name.to_owned());
    }
}

fn parse_style_context_property_id(name: &str) -> Option<PropertyId> {
    with_declaration_context(|context| PropertyId::parse(name, context).ok()).flatten()
}

fn property_name(property: &PropertyId) -> String {
    match property.as_shorthand() {
        Ok(shorthand) => shorthand.name().to_owned(),
        Err(id) => id.name().into_owned(),
    }
}

fn property_declaration_name(property: PropertyDeclarationId<'_>) -> String {
    property.name().into_owned()
}

fn source_declaration_has_unresolved_value(declarations: &SourcePropertyDeclaration) -> bool {
    matches!(declarations.all_shorthand, AllShorthand::WithVariables(_))
        || declarations
            .declarations
            .iter()
            .any(|declaration| matches!(declaration, PropertyDeclaration::WithVariables(_)))
}

fn declaration_entry(
    declaration: &PropertyDeclaration,
    importance: Importance,
) -> Option<CssDeclarationEntry> {
    let name = property_declaration_name(declaration.id());
    let mut value = CssString::new();
    declaration.to_css(&mut value).ok()?;
    Some(CssDeclarationEntry {
        name,
        value,
        priority: importance.important(),
    })
}

fn source_declaration_entries(
    declarations: &SourcePropertyDeclaration,
    importance: Importance,
) -> Option<Vec<CssDeclarationEntry>> {
    if !matches!(declarations.all_shorthand, AllShorthand::NotSet) {
        let mut block = PropertyDeclarationBlock::new();
        for declaration in declarations.all_shorthand.declarations() {
            block.push(declaration, importance);
        }
        let mut value = CssString::new();
        block
            .property_value_to_css(
                &PropertyId::parse_enabled_for_all_content("all").ok()?,
                &mut value,
            )
            .ok()?;
        return (!value.is_empty()).then(|| {
            vec![CssDeclarationEntry {
                name: "all".to_owned(),
                value,
                priority: importance.important(),
            }]
        });
    }
    declarations
        .declarations
        .iter()
        .map(|declaration| declaration_entry(declaration, importance))
        .collect()
}

pub fn parse_declaration_block(css_text: &str) -> CssDeclarationBlock {
    with_declaration_context(|context| {
        let mut input = ParserInput::new(css_text);
        let mut parser = Parser::new(&mut input);
        CssDeclarationBlock::new(parse_property_declaration_list(context, &mut parser, &[]))
    })
    .unwrap_or_default()
}

fn with_declaration_context<R>(f: impl FnOnce(&ParserContext) -> R) -> Option<R> {
    let url_data = UrlExtraData::from(url::Url::parse("about:blank").ok()?);
    let context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(CssRuleType::Style),
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        Cow::Owned(Namespaces::default()),
        None,
        None,
        AttrTaint::default(),
    );
    Some(f(&context))
}

#[cfg(test)]
mod tests {
    use super::{parse_declaration_block, CssDeclarationBlock, CssDeclarationEntry, CssSetResult};

    fn set_projection_entries(
        block: &mut CssDeclarationBlock,
        name: &str,
        value: &str,
        priority: bool,
    ) -> Vec<CssDeclarationEntry> {
        let projection = block.set_property_with_projection(name, value, priority);
        assert_ne!(projection.set_result, CssSetResult::ParseError);
        projection.entries
    }

    #[test]
    fn declaration_block_uses_stylo_parser_and_cssom_serialization() {
        let block = parse_declaration_block(
            "width: 0; color: invalid; margin: 1px 2px; --token: a b; color: red !important;",
        );

        assert_eq!(block.item(0).as_deref(), Some("width"));
        assert_eq!(block.item(1).as_deref(), Some("margin-top"));
        assert_eq!(block.item(99), None);
        assert_eq!(block.property_value("width").as_deref(), Some("0px"));
        assert_eq!(block.property_value("margin").as_deref(), Some("1px 2px"));
        assert_eq!(block.property_value("--token").as_deref(), Some("a b"));
        assert_eq!(block.property_value("color").as_deref(), Some("red"));
        assert!(block.property_priority("color"));
        assert_eq!(
            block.css_text(),
            "width: 0px; margin: 1px 2px; --token: a b; color: red !important;"
        );
    }

    #[test]
    fn opacity_cssom_serialization_preserves_calc_percentage_type() {
        let mut block = CssDeclarationBlock::default();
        for (value, expected) in [
            ("50%", "0.5"),
            ("calc(-50% - 50%)", "calc(-100%)"),
            ("calc(25% * 2)", "calc(50%)"),
            ("clamp(50%, 80%, 70%)", "clamp(50%, 80%, 70%)"),
            ("calc(-0.5 - 0.5)", "calc(-1)"),
        ] {
            let entries = set_projection_entries(&mut block, "opacity", value, false);
            assert_eq!(entries.len(), 1, "{value}");
            assert_eq!(entries[0].value, expected, "{value}");
            assert_eq!(
                block.property_value("opacity").as_deref(),
                Some(expected),
                "{value}"
            );
        }
    }

    #[test]
    fn declaration_block_item_and_entries_expose_custom_property_cssom_names() {
        let block = parse_declaration_block(r"--a\;b: value; --\\: other;");
        let entries = block.entries();

        assert_eq!(block.item(0).as_deref(), Some("--a;b"));
        assert_eq!(block.item(1).as_deref(), Some(r"--\"));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "--a;b");
        assert_eq!(entries[0].value, "value");
        assert_eq!(entries[1].name, r"--\");
        assert_eq!(entries[1].value, "other");
        assert_eq!(block.property_value("--a;b").as_deref(), Some("value"));
        assert_eq!(block.property_value(r"--\").as_deref(), Some("other"));
        assert_eq!(block.css_text(), r"--a\;b: value; --\\: other;");
    }

    #[test]
    fn declaration_block_entries_are_expanded_pdb_declarations() {
        let block = parse_declaration_block("padding: 1px 2px; background-color: transparent;");
        let entries = block.entries();

        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].name, "padding-top");
        assert_eq!(entries[0].value, "1px");
        assert_eq!(entries[1].name, "padding-right");
        assert_eq!(entries[1].value, "2px");
        assert_eq!(entries[4].name, "background-color");
        assert_eq!(entries[4].value, "transparent");
    }

    #[test]
    fn declaration_block_exposes_moli_cssom_compat_properties() {
        static_prefs::set_pref!("layout.columns.enabled", true);
        static_prefs::set_pref!("layout.css.tree-counting-functions.enabled", true);
        let block = parse_declaration_block(
            "column-rule-width: 0; column-width: 0; scroll-margin-top: 0; \
             scroll-padding-bottom: 0; scroll-snap-align: start start; \
             scrollbar-color: auto; scrollbar-width: thin; shape-margin: 0; \
             appearance: auto; user-select: none; print-color-adjust: economy; \
             color-adjust: exact; forced-color-adjust: preserve-parent-color; \
             color-scheme: dark only; orphans: 2; widows: 3; \
             page-break-after: always; page-break-before: avoid; \
             page-break-inside: avoid; alignment-baseline: alphabetic; \
             background-attachment: local; background-clip: text; baseline-source: first; \
             bookmark-level: 1; bookmark-state: closed; border-collapse: collapse; \
             caption-side: bottom; clear: both; clip: rect(0px, 1px, 2px, 3px); \
             empty-cells: hide; link-parameters: param(--a, orange), param(--b); \
             list-style-position: inside; list-style-type: upper-alpha; \
             outline-style: auto; table-layout: fixed; \
             text-size-adjust: calc(10% * sibling-index()); text-transform: uppercase;",
        );

        assert_eq!(
            block.property_value("column-rule-width").as_deref(),
            Some("0px")
        );
        assert_eq!(block.property_value("column-width").as_deref(), Some("0px"));
        assert_eq!(
            block.property_value("scroll-margin-top").as_deref(),
            Some("0px")
        );
        assert_eq!(
            block.property_value("scroll-padding-bottom").as_deref(),
            Some("0px")
        );
        assert_eq!(
            block.property_value("scroll-snap-align").as_deref(),
            Some("start")
        );
        assert_eq!(
            block.property_value("scrollbar-color").as_deref(),
            Some("auto")
        );
        assert_eq!(
            block.property_value("scrollbar-width").as_deref(),
            Some("thin")
        );
        assert_eq!(block.property_value("shape-margin").as_deref(), Some("0px"));
        assert_eq!(block.property_value("appearance").as_deref(), Some("auto"));
        assert_eq!(block.property_value("user-select").as_deref(), Some("none"));
        assert_eq!(
            block.property_value("color-adjust").as_deref(),
            Some("exact")
        );
        assert_eq!(
            block.property_value("print-color-adjust").as_deref(),
            Some("exact")
        );
        assert_eq!(
            block.property_value("forced-color-adjust").as_deref(),
            Some("preserve-parent-color")
        );
        assert_eq!(
            block.property_value("color-scheme").as_deref(),
            Some("dark only")
        );
        assert_eq!(block.property_value("orphans").as_deref(), Some("2"));
        assert_eq!(block.property_value("widows").as_deref(), Some("3"));
        assert_eq!(
            block.property_value("page-break-after").as_deref(),
            Some("always")
        );
        assert_eq!(
            block.property_value("page-break-before").as_deref(),
            Some("avoid")
        );
        assert_eq!(
            block.property_value("page-break-inside").as_deref(),
            Some("avoid")
        );
        assert_eq!(
            block.property_value("alignment-baseline").as_deref(),
            Some("alphabetic")
        );
        assert_eq!(
            block.property_value("background-attachment").as_deref(),
            Some("local")
        );
        assert_eq!(
            block.property_value("background-clip").as_deref(),
            Some("text")
        );
        assert_eq!(
            block.property_value("baseline-source").as_deref(),
            Some("first")
        );
        assert_eq!(block.property_value("bookmark-level").as_deref(), Some("1"));
        assert_eq!(
            block.property_value("bookmark-state").as_deref(),
            Some("closed")
        );
        assert_eq!(
            block.property_value("border-collapse").as_deref(),
            Some("collapse")
        );
        assert_eq!(
            block.property_value("caption-side").as_deref(),
            Some("bottom")
        );
        assert_eq!(block.property_value("clear").as_deref(), Some("both"));
        assert_eq!(
            block.property_value("clip").as_deref(),
            Some("rect(0px, 1px, 2px, 3px)")
        );
        assert_eq!(block.property_value("empty-cells").as_deref(), Some("hide"));
        assert_eq!(
            block.property_value("link-parameters").as_deref(),
            Some("param(--a, orange), param(--b)")
        );
        assert_eq!(
            block.property_value("list-style-position").as_deref(),
            Some("inside")
        );
        assert_eq!(
            block.property_value("list-style-type").as_deref(),
            Some("upper-alpha")
        );
        assert_eq!(
            block.property_value("outline-style").as_deref(),
            Some("auto")
        );
        assert_eq!(
            block.property_value("table-layout").as_deref(),
            Some("fixed")
        );
        assert_eq!(
            block.property_value("text-size-adjust").as_deref(),
            Some("calc(10% * sibling-index())")
        );
        assert_eq!(
            block.property_value("text-transform").as_deref(),
            Some("uppercase")
        );
    }

    #[test]
    fn declaration_block_owns_computed_compat_longhands_in_stylo() {
        static_prefs::set_pref!("layout.columns.enabled", true);
        static_prefs::set_pref!("layout.css.scroll-driven-animations.enabled", true);
        static_prefs::set_pref!("layout.css.zoom.enabled", true);

        let block = parse_declaration_block(
            "animation-timeline: auto; animation-range-start: entry 10%; \
             animation-range-end: exit 20%; column-span: all; column-width: 12px; \
             font-variant-alternates: historical-forms; font-variant-emoji: emoji; \
             font-variant-position: super; zoom: 125%;",
        );

        for (property, expected) in [
            ("animation-timeline", "auto"),
            ("animation-range-start", "entry 10%"),
            ("animation-range-end", "exit 20%"),
            ("column-span", "all"),
            ("column-width", "12px"),
            ("font-variant-alternates", "historical-forms"),
            ("font-variant-emoji", "emoji"),
            ("font-variant-position", "super"),
            ("zoom", "125%"),
        ] {
            assert_eq!(
                block.property_value(property).as_deref(),
                Some(expected),
                "{property} should be parsed and serialized by Stylo",
            );
            assert!(block.property_is_declared(property));
        }

        let dynamic_zoom = parse_declaration_block("zoom: calc(sign(1em - 1px) * 2%);");
        assert_eq!(
            dynamic_zoom.property_value("zoom").as_deref(),
            Some("calc(2% * sign(1em - 1px))")
        );

        let variant = parse_declaration_block("font-variant: historical-forms emoji super;");
        assert_eq!(
            variant.property_value("font-variant-alternates").as_deref(),
            Some("historical-forms")
        );
        assert_eq!(
            variant.property_value("font-variant-emoji").as_deref(),
            Some("emoji")
        );
        assert_eq!(
            variant.property_value("font-variant-position").as_deref(),
            Some("super")
        );

        let reset = parse_declaration_block(
            "font-variant: historical-forms emoji super; font: italic 16px serif;",
        );
        for property in [
            "font-variant-alternates",
            "font-variant-emoji",
            "font-variant-position",
        ] {
            assert_eq!(reset.property_value(property).as_deref(), Some("normal"));
        }
    }

    #[test]
    fn declaration_block_handles_moli_cssom_compat_edge_values() {
        static_prefs::set_pref!("layout.css.tree-counting-functions.enabled", true);

        let block = parse_declaration_block(
            "text-size-adjust: calc(10% + 5%); \
             link-parameters: param(--a, ); \
             bookmark-level: none;",
        );

        assert_eq!(
            block.property_value("text-size-adjust").as_deref(),
            Some("calc(15%)")
        );
        assert_eq!(
            block.property_value("link-parameters").as_deref(),
            Some("param(--a, )")
        );
        assert_eq!(
            block.property_value("bookmark-level").as_deref(),
            Some("none")
        );

        let block = parse_declaration_block(
            "text-size-adjust: calc(10% * sibling-index()); \
             link-parameters: param(--a);",
        );
        assert_eq!(
            block.property_value("text-size-adjust").as_deref(),
            Some("calc(10% * sibling-index())")
        );
        assert_eq!(
            block.property_value("link-parameters").as_deref(),
            Some("param(--a)")
        );

        let block = parse_declaration_block("link-parameters: param(--a");
        assert_eq!(
            block.property_value("link-parameters").as_deref(),
            Some("param(--a)")
        );
    }

    #[test]
    fn declaration_block_rejects_invalid_moli_cssom_compat_values() {
        let block = parse_declaration_block(
            "bookmark-level: 0; bookmark-state: none; \
             text-size-adjust: -100%; text-size-adjust: 10px; \
             link-parameters: param(-a); link-parameters: param(--a red); \
             link-parameters: param(--a, red) param(--b, blue);",
        );

        assert_eq!(block.property_value("bookmark-level").as_deref(), Some(""));
        assert_eq!(block.property_value("bookmark-state").as_deref(), Some(""));
        assert_eq!(
            block.property_value("text-size-adjust").as_deref(),
            Some("")
        );
        assert_eq!(block.property_value("link-parameters").as_deref(), Some(""));
        assert!(!block.property_is_declared("bookmark-level"));
        assert!(!block.property_is_declared("bookmark-state"));
        assert!(!block.property_is_declared("text-size-adjust"));
        assert!(!block.property_is_declared("link-parameters"));
    }

    #[test]
    fn declaration_block_set_property_updates_through_pdb() {
        let mut block = parse_declaration_block("color: red !important; padding: 1px 2px;");

        let entries = set_projection_entries(&mut block, "color", "blue", false);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "color");
        assert_eq!(entries[0].value, "blue");
        assert_eq!(entries[0].priority, false);
        assert_eq!(block.property_value("color").as_deref(), Some("blue"));
        assert!(block.property_is_declared("color"));
        assert!(!block.property_priority("color"));

        let entries = set_projection_entries(&mut block, "margin", "0 2px", true);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["margin-top", "margin-right", "margin-bottom", "margin-left"]
        );
        assert_eq!(block.property_value("margin").as_deref(), Some("0px 2px"));
        assert!(block.property_is_declared("margin"));
        assert!(block.property_is_declared("margin-left"));
        assert!(block.property_priority("margin"));
        assert_eq!(
            block.css_text(),
            "padding: 1px 2px; color: blue; margin: 0px 2px !important;"
        );

        let entries = set_projection_entries(&mut block, "link-parameters", "param(--a", false);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "link-parameters");
        assert_eq!(entries[0].value, "param(--a)");
        assert_eq!(entries[0].priority, false);
        assert_eq!(
            block.property_value("link-parameters").as_deref(),
            Some("param(--a)")
        );
        assert!(!block.property_priority("link-parameters"));

        let mut longhand_block = parse_declaration_block("");
        for name in ["margin-top", "margin-right", "margin-bottom", "margin-left"] {
            set_projection_entries(&mut longhand_block, name, "0", true);
        }
        assert_eq!(
            longhand_block.property_value("margin").as_deref(),
            Some("0px")
        );
        assert!(longhand_block.property_priority("margin"));
        assert_eq!(longhand_block.css_text(), "margin: 0px !important;");

        let mut single_longhand_block = parse_declaration_block("");
        set_projection_entries(&mut single_longhand_block, "opacity", "0.5", true);
        assert_eq!(
            single_longhand_block.property_value("opacity").as_deref(),
            Some("0.5")
        );
        assert!(single_longhand_block.property_priority("opacity"));
        assert_eq!(single_longhand_block.css_text(), "opacity: 0.5 !important;");

        let mut all_block = parse_declaration_block("display: block; color: red;");
        let entries = set_projection_entries(&mut all_block, "all", "inherit", false);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "all");
        assert_eq!(entries[0].value, "inherit");
        assert_eq!(entries[0].priority, false);
        assert_eq!(all_block.css_text(), "all: inherit;");
    }

    #[test]
    fn declaration_block_animation_shorthand_uses_cssom_longhand_order() {
        static_prefs::set_pref!("layout.css.scroll-driven-animations.enabled", true);

        let mut block = parse_declaration_block("");
        let entries = set_projection_entries(
            &mut block,
            "animation",
            "fade 1s linear 2s 3 reverse both paused",
            false,
        );
        let expected = [
            "animation-duration",
            "animation-timing-function",
            "animation-delay",
            "animation-iteration-count",
            "animation-direction",
            "animation-fill-mode",
            "animation-play-state",
            "animation-name",
            "animation-timeline",
            "animation-range-start",
            "animation-range-end",
        ];
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(block.item(0).as_deref(), Some("animation-duration"));
        assert_eq!(block.item(7).as_deref(), Some("animation-name"));
        assert_eq!(block.item(10).as_deref(), Some("animation-range-end"));
        assert_eq!(
            block.property_value("animation").as_deref(),
            Some("1s linear 2s 3 reverse both paused fade")
        );
    }

    #[test]
    fn declaration_block_uses_stylo_easing_range_semantics() {
        static_prefs::set_pref!("layout.css.tree-counting-functions.enabled", true);

        for value in [
            "steps(calc(1), jump-none)",
            "steps(calc(-10), start)",
            "steps(calc(2 * sibling-index()), jump-none)",
            "cubic-bezier(calc(-2), calc(0.7 / 2), calc(1.5), calc(0))",
            "cubic-bezier(0, sibling-index(), 1, sign(2em - 20px))",
        ] {
            let mut block = parse_declaration_block("");
            let projection =
                block.set_property_with_projection("animation-timing-function", value, false);
            assert_ne!(
                projection.set_result,
                CssSetResult::ParseError,
                "valid easing value was rejected: {value}"
            );
            assert!(block.property_is_declared("animation-timing-function"));
        }

        let mut block = parse_declaration_block("");
        let projection = block.set_property_with_projection(
            "animation-timing-function",
            "steps(calc(0/0), jump-none)",
            false,
        );
        assert_eq!(projection.set_result, CssSetResult::ParseError);
        assert!(block.is_empty());

        let projection = block.set_property_with_projection(
            "animation-timing-function",
            "cubic-bezier(-0.1, 0.1, 0.5, 0.9)",
            false,
        );
        assert_eq!(projection.set_result, CssSetResult::ParseError);
        assert!(block.is_empty());
    }

    #[test]
    fn declaration_block_animation_longhands_keep_write_order() {
        let mut block = parse_declaration_block("");
        set_projection_entries(&mut block, "animation-name", "fade", false);
        set_projection_entries(&mut block, "animation-duration", "1s", false);
        assert_eq!(block.item(0).as_deref(), Some("animation-name"));
        assert_eq!(block.item(1).as_deref(), Some("animation-duration"));

        let mut block = parse_declaration_block("");
        set_projection_entries(&mut block, "animation-duration", "1s", false);
        set_projection_entries(&mut block, "animation-name", "fade", false);
        assert_eq!(block.item(0).as_deref(), Some("animation-duration"));
        assert_eq!(block.item(1).as_deref(), Some("animation-name"));
    }

    #[test]
    fn declaration_block_set_property_rejects_non_style_context_properties() {
        let mut block = CssDeclarationBlock::default();

        let projection = block.set_property_with_projection("size", "initial", false);
        assert_eq!(projection.set_result, CssSetResult::ParseError);
        assert!(projection.entries.is_empty());
        assert!(block.css_text().is_empty());

        let projection =
            block.set_property_with_projection("border-bottom-color", "inherit", false);
        assert_eq!(projection.set_result, CssSetResult::ChangedPropertySet);
        assert_eq!(projection.entries.len(), 1);
        assert_eq!(projection.entries[0].name, "border-bottom-color");
        assert_eq!(projection.entries[0].value, "inherit");
    }

    #[test]
    fn declaration_block_set_property_accepts_background_clip_text() {
        let mut block = CssDeclarationBlock::default();
        let projection = block.set_property_with_projection("background-clip", "text", false);

        assert_eq!(projection.set_result, CssSetResult::ChangedPropertySet);
        assert_eq!(projection.entries.len(), 1);
        assert_eq!(projection.entries[0].name, "background-clip");
        assert_eq!(projection.entries[0].value, "text");
        assert_eq!(projection.stored_names, ["background-clip"]);
        assert_eq!(
            block.property_value("background-clip").as_deref(),
            Some("text")
        );
        assert_eq!(block.css_text(), "background-clip: text;");
    }

    #[test]
    fn moli_property_surface_matches_chromium_for_former_gecko_gates() {
        let chromium_supported = [
            "-webkit-line-clamp",
            "anchor-name",
            "anchor-scope",
            "clip-rule",
            "column-rule-color",
            "column-rule-style",
            "contain-intrinsic-block-size",
            "contain-intrinsic-height",
            "contain-intrinsic-inline-size",
            "contain-intrinsic-width",
            "content-visibility",
            "counter-set",
            "flood-color",
            "flood-opacity",
            "font-palette",
            "font-synthesis-small-caps",
            "font-synthesis-style",
            "lighting-color",
            "marker-end",
            "marker-mid",
            "marker-start",
            "offset-anchor",
            "offset-distance",
            "offset-position",
            "offset-rotate",
            "overflow-anchor",
            "page",
            "paint-order",
            "position-anchor",
            "position-try-order",
            "position-visibility",
            "resize",
            "scroll-snap-stop",
            "scroll-snap-type",
            "scrollbar-gutter",
            "shape-image-threshold",
            "shape-outside",
            "stop-color",
            "stop-opacity",
            "transform-box",
            "scroll-timeline-axis",
            "scroll-timeline-name",
            "view-timeline-axis",
            "view-timeline-inset",
            "view-timeline-name",
            "vector-effect",
            "hyphenate-character",
            "hyphenate-limit-chars",
            "ruby-position",
            "text-autospace",
            "text-box-edge",
            "text-box-trim",
            "initial-letter",
            "column-fill",
            "math-shift",
            "image-orientation",
            "text-anchor",
            "color-interpolation",
            "color-interpolation-filters",
            "shape-rendering",
            "hyphens",
            "ruby-align",
            "text-combine-upright",
            "text-wrap-style",
            "field-sizing",
            "dominant-baseline",
            "timeline-scope",
            "scroll-margin",
            "scroll-margin-block",
            "scroll-margin-inline",
            "scroll-padding",
            "scroll-padding-block",
            "scroll-padding-inline",
            "offset",
            "contain-intrinsic-size",
            "position-try",
            "marker",
            "scroll-timeline",
            "view-timeline",
            "column-rule",
            "font-synthesis",
            "text-box",
            "text-wrap",
        ];
        for name in chromium_supported {
            let mut block = CssDeclarationBlock::default();
            let projection = block.set_property_with_projection(name, "initial", false);
            assert_ne!(
                projection.set_result,
                CssSetResult::ParseError,
                "Chromium-supported property should be parsed for Moli: {name}"
            );
            assert!(
                !block.css_text().is_empty(),
                "Chromium-supported property should be retained: {name}"
            );
        }

        let chromium_unsupported = [
            "-moz-box-flex",
            "-moz-box-ordinal-group",
            "font-synthesis-position",
            "-moz-image-decoding",
            "masonry-auto-flow",
            "-moz-context-properties",
            "-moz-inert",
            "-moz-user-focus",
            "-moz-theme",
            "-moz-force-broken-image-icon",
            "-moz-subtree-hidden-only-visually",
            "-moz-window-input-region-margin",
            "-moz-window-opacity",
            "-moz-window-transform",
            "-moz-control-character-visibility",
            "-x-span",
            "-moz-float-edge",
            "-moz-top-layer",
            "-moz-orient",
            "-moz-osx-font-smoothing",
            "-moz-box-collapse",
            "-moz-text-size-adjust",
            "ime-mode",
            "-moz-window-dragging",
            "-moz-window-shadow",
            "-moz-box-align",
            "-moz-box-direction",
            "-moz-box-orient",
            "-moz-box-pack",
            "-moz-math-variant",
        ];
        for name in chromium_unsupported {
            let mut block = CssDeclarationBlock::default();
            let projection = block.set_property_with_projection(name, "initial", false);
            assert_eq!(
                projection.set_result,
                CssSetResult::ParseError,
                "Gecko-only property must not leak into Moli: {name}"
            );
            assert!(
                block.is_empty(),
                "rejected property must not be retained: {name}"
            );
        }
    }

    #[test]
    fn moli_chromium_shorthands_parse_non_wide_values() {
        for (name, value) in [
            ("scroll-margin", "1px 2px"),
            ("scroll-margin-block", "1px 2px"),
            ("scroll-margin-inline", "1px 2px"),
            ("scroll-padding", "1px 2px"),
            ("scroll-padding-block", "1px 2px"),
            ("scroll-padding-inline", "1px 2px"),
            ("offset", "none"),
            ("contain-intrinsic-size", "100px 200px"),
            ("position-try", "--fallback"),
            ("marker", "none"),
            ("scroll-timeline", "--scroll block"),
            ("view-timeline", "--view inline"),
            ("column-rule", "1px solid red"),
            ("font-synthesis", "weight style small-caps"),
            ("text-box", "normal"),
            ("text-wrap", "wrap balance"),
        ] {
            let mut block = CssDeclarationBlock::default();
            let projection = block.set_property_with_projection(name, value, false);
            assert_ne!(
                projection.set_result,
                CssSetResult::ParseError,
                "Chromium shorthand value should parse: {name}: {value}"
            );
            assert!(
                block
                    .property_value(name)
                    .is_some_and(|serialized| !serialized.is_empty()),
                "Chromium shorthand should serialize after parsing: {name}: {value}"
            );
        }
    }

    #[test]
    fn declaration_block_set_property_reports_cssom_mutation_results() {
        let mut block = parse_declaration_block("color: red;");

        assert_eq!(
            block.set_property("color", "red", false),
            CssSetResult::Unchanged
        );
        assert_eq!(
            block.set_property("color", "blue", false),
            CssSetResult::ModifiedExisting
        );
        assert_eq!(
            block.set_property("margin", "0", true),
            CssSetResult::ChangedPropertySet
        );
        assert_eq!(block.property_value("margin").as_deref(), Some("0px"));
        assert!(block.property_priority("margin"));

        assert_eq!(
            block.set_property("margin", "", false),
            CssSetResult::ChangedPropertySet
        );
        assert_eq!(block.property_value("margin").as_deref(), Some(""));
        assert!(!block.property_is_declared("margin"));
        assert_eq!(
            block.set_property("margin", "", false),
            CssSetResult::Unchanged
        );
    }

    #[test]
    fn declaration_block_exposes_cssom_affected_names_metadata() {
        static_prefs::set_pref!("layout.css.scroll-driven-animations.enabled", true);

        fn assert_contains(property: &str, names: &[&str]) {
            let affected = CssDeclarationBlock::affected_names_for_property(property)
                .unwrap_or_else(|| panic!("{property} should expose affected names"));
            for name in names {
                assert!(
                    affected.iter().any(|affected| affected == name),
                    "{property} should affect {name}; got {affected:?}"
                );
            }
        }

        assert_eq!(
            CssDeclarationBlock::affected_names_for_property("color"),
            Some(vec!["color".to_owned()])
        );
        assert_eq!(
            CssDeclarationBlock::affected_names_for_property("--token"),
            Some(vec!["--token".to_owned()])
        );
        assert_eq!(
            CssDeclarationBlock::affected_names_for_property("margin"),
            Some(vec![
                "margin".to_owned(),
                "margin-top".to_owned(),
                "margin-right".to_owned(),
                "margin-bottom".to_owned(),
                "margin-left".to_owned(),
            ])
        );
        assert_eq!(
            CssDeclarationBlock::affected_names_for_property("margin-top"),
            Some(vec!["margin-top".to_owned()])
        );
        assert_contains(
            "animation",
            &[
                "animation",
                "animation-timeline",
                "animation-range-start",
                "animation-range-end",
            ],
        );
        assert_contains("border", &["border", "border-image"]);
        assert_contains(
            "font",
            &[
                "font",
                "font-variant",
                "font-variant-alternates",
                "font-variant-position",
                "font-variant-emoji",
            ],
        );
        assert_contains(
            "font-variant",
            &[
                "font-variant",
                "font-variant-alternates",
                "font-variant-position",
                "font-variant-emoji",
            ],
        );
        assert_eq!(
            CssDeclarationBlock::affected_names_for_property("font-variant-alternates"),
            Some(vec!["font-variant-alternates".to_owned()])
        );
        assert_contains(
            "overscroll-behavior",
            &[
                "overscroll-behavior",
                "overscroll-behavior-x",
                "overscroll-behavior-y",
            ],
        );
        assert_eq!(CssDeclarationBlock::affected_names_for_property("--"), None);
    }

    #[test]
    fn declaration_block_preserves_unresolved_cssom_values_in_pdb() {
        let mut block = parse_declaration_block("");

        let projection = block.set_property_with_projection("margin", "var(--gap)", true);
        assert_eq!(projection.set_result, CssSetResult::ChangedPropertySet);
        assert!(projection.has_unresolved_value);
        assert_eq!(
            block.property_value("margin").as_deref(),
            Some("var(--gap)")
        );
        assert!(block.property_priority("margin"));
        assert_eq!(block.css_text(), "margin: var(--gap) !important;");

        let projection =
            block.set_property_with_projection("padding-top", "env(safe-area-inset-top)", false);
        assert_eq!(projection.set_result, CssSetResult::ChangedPropertySet);
        assert!(projection.has_unresolved_value);
        assert_eq!(
            block.property_value("padding-top").as_deref(),
            Some("env(safe-area-inset-top)")
        );

        let projection = block.set_property_with_projection("top", "env(test 0 1, green)", false);
        assert_eq!(projection.set_result, CssSetResult::ChangedPropertySet);
        assert!(projection.has_unresolved_value);
        assert_eq!(
            block.property_value("top").as_deref(),
            Some("env(test 0 1, green)")
        );

        assert_eq!(
            block.set_property("--token", "var(--gap, 1px)", true),
            CssSetResult::ChangedPropertySet
        );
        assert_eq!(
            block.property_value("--token").as_deref(),
            Some("var(--gap, 1px)")
        );
        assert!(block.property_priority("--token"));
    }

    #[test]
    fn declaration_block_mutates_custom_properties_through_pdb() {
        let mut block = parse_declaration_block("--token: old;");

        let projection = block.set_property_with_projection("--token", "var(--gap, 1px)", true);
        assert_eq!(projection.set_result, CssSetResult::ModifiedExisting);
        assert_eq!(projection.affected_names, ["--token"]);
        assert_eq!(projection.stored_names, ["--token"]);
        assert!(!projection.has_unresolved_value);
        assert_eq!(
            block.property_value("--token").as_deref(),
            Some("var(--gap, 1px)")
        );
        assert!(block.property_priority("--token"));
        assert_eq!(block.css_text(), "--token: var(--gap, 1px) !important;");

        let projection = block.set_property_with_projection("--token", "  ", false);
        assert_eq!(projection.set_result, CssSetResult::ModifiedExisting);
        assert_eq!(block.property_value("--token").as_deref(), Some(""));
        assert!(!block.property_priority("--token"));
        assert_eq!(block.css_text(), "--token: ;");

        let projection = block.set_property_with_projection("--token", "", false);
        assert_eq!(projection.set_result, CssSetResult::ChangedPropertySet);
        assert!(projection.entries.is_empty());
        assert!(!block.property_is_declared("--token"));
        assert_eq!(block.property_value("--token").as_deref(), Some(""));
        assert_eq!(block.css_text(), "");
    }

    #[test]
    fn declaration_block_set_property_rejects_declaration_source_fragments() {
        let mut block = parse_declaration_block("color: red;");

        assert_eq!(
            block.set_property("color", "blue; width: 1px", false),
            CssSetResult::ParseError
        );
        assert_eq!(block.css_text(), "color: red;");
    }

    #[test]
    fn declaration_block_remove_property_uses_cssom_shorthand_semantics() {
        let mut block = parse_declaration_block("padding: 1px 2px; color: red;");

        let result = block.remove_property("padding-left");
        assert!(result.changed);
        assert_eq!(result.old_value.as_deref(), Some("2px"));
        assert_eq!(block.property_value("padding").as_deref(), Some(""));
        assert_eq!(block.property_value("padding-left").as_deref(), Some(""));
        assert_eq!(block.property_value("padding-top").as_deref(), Some("1px"));

        let result = block.remove_property("padding");
        assert!(result.changed);
        assert_eq!(result.old_value.as_deref(), Some(""));
        assert_eq!(block.property_value("padding-top").as_deref(), Some(""));
        assert_eq!(block.property_value("padding-right").as_deref(), Some(""));
        assert_eq!(block.remove_property("does-not-exist"), Default::default());
    }
}

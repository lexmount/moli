use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    rc::{Rc, Weak as RcWeak},
    sync::{Arc as StdArc, Weak as StdWeak, atomic::AtomicBool},
};

use cssparser::{Parser, ParserInput, SourceLocation, Token};
use parking_lot::Mutex;
use selectors::SelectorList;
use style::stylesheets::keyframes_rule::{Keyframe, KeyframeSelectors, KeyframesRule};
use style::{
    context::QuirksMode,
    custom_properties::AttrTaint,
    font_face::FontFaceRule,
    invalidation::stylesheets::RuleChangeKind,
    media_queries::MediaList,
    parser::{NestingContext, Parse, ParserContext},
    properties::{PropertyDeclarationBlock, parse_property_declaration_list},
    selector_parser::{SelectorImpl, SelectorParser},
    servo_arc::Arc as ServoArc,
    shared_lock::{Locked, SharedRwLock, SharedRwLockReadGuard, ToCssWithGuard},
    stylesheets::{
        AllowImportRules, CssRule, CssRuleType, CssRuleTypes, CssRules, Origin, PageSelectors,
        RulesMutateError, Stylesheet, StylesheetContents, StylesheetInDocument, StylesheetLoader,
        UrlExtraData,
        font_feature_values_rule::{
            FFVDeclaration, FontFeatureValuesRule, PairValues, SingleValue, VectorValues,
        },
        import_rule::{ImportLayer, ImportRule, ImportSheet, ImportSupportsCondition},
    },
    values::{CssUrl, KeyframesName, computed::font::FamilyName},
};
use style_traits::{CssStringWriter, ParsingMode, ToCss};

use moli_crypto::Sha256Context;
use moli_css_parse::{
    CssRuleInsertError, CssRuleSnapshot, css_rule_snapshot_from_native_with_stylo,
    keyframe_rule_snapshot_from_native_with_stylo, parse_font_face_cssom_rule_with_stylo_context,
    refresh_native_stylesheet_namespaces_after_cssom_mutation,
};

/// Exact mutation history is an optimization, not a correctness cache. A
/// consumer older than this window detects the generation gap and performs a
/// full retained-stylesheet update instead of keeping old Stylo rule trees
/// alive indefinitely.
pub(crate) const LIVE_STYLESHEET_CASCADE_MUTATION_JOURNAL_CAPACITY: usize = 256;

pub(crate) type LiveStylesheetCascadeMutationJournal = VecDeque<LiveStylesheetCascadeMutationBatch>;

#[cfg(test)]
thread_local! {
    static LIVE_STYLESHEET_CSS_TEXT_PROJECTION_COUNT: Cell<usize> = const { Cell::new(0) };
    static LIVE_STYLESHEET_PARSE_COUNT: Cell<usize> = const { Cell::new(0) };
    static LIVE_STYLESHEET_MUTATION_METRICS: Cell<LiveStylesheetMutationMetrics> =
        const { Cell::new(LiveStylesheetMutationMetrics::new()) };
    static LIVE_STYLESHEET_DEPENDENCY_SUMMARY_PROJECTION_COUNT: Cell<usize> = const { Cell::new(0) };
    static LIVE_STYLESHEET_FONT_FACE_PROJECTION_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LiveStylesheetMutationMetrics {
    pub(crate) native_top_level_mutations: usize,
    pub(crate) native_nested_mutations: usize,
    pub(crate) native_keyframe_mutations: usize,
    pub(crate) native_rule_value_mutations: usize,
    pub(crate) recursive_rule_snapshots: usize,
}

#[cfg(test)]
impl LiveStylesheetMutationMetrics {
    const fn new() -> Self {
        Self {
            native_top_level_mutations: 0,
            native_nested_mutations: 0,
            native_keyframe_mutations: 0,
            native_rule_value_mutations: 0,
            recursive_rule_snapshots: 0,
        }
    }
}

#[cfg(test)]
pub(crate) fn reset_live_stylesheet_mutation_metrics_for_test() {
    LIVE_STYLESHEET_MUTATION_METRICS
        .with(|metrics| metrics.set(LiveStylesheetMutationMetrics::new()));
}

#[cfg(test)]
pub(crate) fn live_stylesheet_mutation_metrics_for_test() -> LiveStylesheetMutationMetrics {
    LIVE_STYLESHEET_MUTATION_METRICS.with(Cell::get)
}

#[cfg(test)]
fn note_native_top_level_mutation_for_test() {
    LIVE_STYLESHEET_MUTATION_METRICS.with(|metrics| {
        let mut value = metrics.get();
        value.native_top_level_mutations += 1;
        metrics.set(value);
    });
}

#[cfg(test)]
fn note_native_nested_mutation_for_test() {
    LIVE_STYLESHEET_MUTATION_METRICS.with(|metrics| {
        let mut value = metrics.get();
        value.native_nested_mutations += 1;
        metrics.set(value);
    });
}

#[cfg(test)]
fn note_native_keyframe_mutation_for_test() {
    LIVE_STYLESHEET_MUTATION_METRICS.with(|metrics| {
        let mut value = metrics.get();
        value.native_keyframe_mutations += 1;
        metrics.set(value);
    });
}

#[cfg(test)]
fn note_native_rule_value_mutation_for_test() {
    LIVE_STYLESHEET_MUTATION_METRICS.with(|metrics| {
        let mut value = metrics.get();
        value.native_rule_value_mutations += 1;
        metrics.set(value);
    });
}

#[cfg(test)]
fn note_recursive_rule_snapshot_for_test() {
    LIVE_STYLESHEET_MUTATION_METRICS.with(|metrics| {
        let mut value = metrics.get();
        value.recursive_rule_snapshots += 1;
        metrics.set(value);
    });
}

#[cfg(test)]
pub(crate) fn reset_live_stylesheet_css_text_projection_count_for_test() {
    LIVE_STYLESHEET_CSS_TEXT_PROJECTION_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn live_stylesheet_css_text_projection_count_for_test() -> usize {
    LIVE_STYLESHEET_CSS_TEXT_PROJECTION_COUNT.with(Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_live_stylesheet_parse_count_for_test() {
    LIVE_STYLESHEET_PARSE_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn live_stylesheet_parse_count_for_test() -> usize {
    LIVE_STYLESHEET_PARSE_COUNT.with(Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_live_stylesheet_dependency_summary_projection_count_for_test() {
    LIVE_STYLESHEET_DEPENDENCY_SUMMARY_PROJECTION_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn live_stylesheet_dependency_summary_projection_count_for_test() -> usize {
    LIVE_STYLESHEET_DEPENDENCY_SUMMARY_PROJECTION_COUNT.with(Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_live_stylesheet_font_face_projection_count_for_test() {
    LIVE_STYLESHEET_FONT_FACE_PROJECTION_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn live_stylesheet_font_face_projection_count_for_test() -> usize {
    LIVE_STYLESHEET_FONT_FACE_PROJECTION_COUNT.with(Cell::get)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StylesheetId(u64);

impl StylesheetId {
    pub(crate) const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_raw(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }
}

pub(crate) type LiveStylesheetRef = Rc<LiveStylesheet>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StylesheetWrapperLeaseId(u64);

impl StylesheetWrapperLeaseId {
    pub(crate) const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_raw(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }
}

pub(crate) type StylesheetWrapperLease = Rc<RefCell<Option<LiveStylesheetRef>>>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StylesheetRuleWrapperLeaseId(u64);

impl StylesheetRuleWrapperLeaseId {
    pub(crate) const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_raw(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum NativeStylesheetRule {
    Css(CssRule),
    Keyframe(ServoArc<Locked<Keyframe>>),
}

impl NativeStylesheetRule {
    fn snapshot(&self, shared_lock: &SharedRwLock) -> CssRuleSnapshot {
        #[cfg(test)]
        note_recursive_rule_snapshot_for_test();
        let guard = shared_lock.read();
        match self {
            Self::Css(rule) => css_rule_snapshot_from_native_with_stylo(rule, &guard),
            Self::Keyframe(rule) => keyframe_rule_snapshot_from_native_with_stylo(rule, &guard),
        }
    }

    fn rule_type(&self) -> CssRuleType {
        match self {
            Self::Css(rule) => rule.rule_type(),
            Self::Keyframe(_) => CssRuleType::Keyframe,
        }
    }

    fn css_text(&self, shared_lock: &SharedRwLock) -> String {
        let guard = shared_lock.read();
        match self {
            // Blink exposes CSSPageRule.cssText on one line. Stylo's generic
            // stylesheet serializer is multiline, so preserve the CSSOM form
            // while reading declarations and margin rules under one lock.
            Self::Css(CssRule::Page(rule)) => {
                let rule = rule.read_with(&guard);
                let selector = rule.selectors.to_css_string();
                let mut block = serialize_declaration_block(&rule.block, &guard);
                for child in &rule.rules.read_with(&guard).0 {
                    if !matches!(child, CssRule::Margin(_)) {
                        continue;
                    }
                    if !block.is_empty() {
                        block.push(' ');
                    }
                    block.push_str(&child.to_css_string(&guard));
                }
                serialize_page_rule_css_text(&selector, &block)
            }
            Self::Css(rule) => rule.to_css_string(&guard),
            Self::Keyframe(rule) => rule.read_with(&guard).to_css_string(&guard),
        }
    }

    fn style_selector_text(&self, shared_lock: &SharedRwLock) -> Option<String> {
        let Self::Css(CssRule::Style(rule)) = self else {
            return None;
        };
        let guard = shared_lock.read();
        Some(cssparser::ToCss::to_css_string(
            &rule.read_with(&guard).selectors,
        ))
    }

    fn keyframe_selector_text(&self, shared_lock: &SharedRwLock) -> Option<String> {
        let Self::Keyframe(rule) = self else {
            return None;
        };
        let guard = shared_lock.read();
        Some(rule.read_with(&guard).selector.to_css_string())
    }

    fn style_has_child_rules(&self, shared_lock: &SharedRwLock) -> Option<bool> {
        let Self::Css(rule @ CssRule::Style(_)) = self else {
            return None;
        };
        let guard = shared_lock.read();
        Some(!rule.children(&guard).is_empty())
    }

    fn grouping_prelude(&self, shared_lock: &SharedRwLock) -> Option<(CssRuleType, String)> {
        let guard = shared_lock.read();
        let (rule_type, prelude) = match self {
            Self::Css(CssRule::Media(rule)) => (
                CssRuleType::Media,
                rule.media_queries.read_with(&guard).to_css_string(),
            ),
            Self::Css(CssRule::Supports(rule)) => {
                (CssRuleType::Supports, rule.condition.to_css_string())
            }
            Self::Css(CssRule::Container(rule)) => {
                (CssRuleType::Container, rule.conditions.to_css_string())
            }
            Self::Css(CssRule::Scope(rule)) => {
                let mut components = Vec::new();
                if let Some(start) = rule.bounds.start.as_ref() {
                    components.push(format!("({})", cssparser::ToCss::to_css_string(start)));
                }
                if let Some(end) = rule.bounds.end.as_ref() {
                    components.push(format!("to ({})", cssparser::ToCss::to_css_string(end)));
                }
                (CssRuleType::Scope, components.join(" "))
            }
            Self::Css(CssRule::LayerBlock(rule)) => (
                CssRuleType::LayerBlock,
                rule.name
                    .as_ref()
                    .map(ToCss::to_css_string)
                    .unwrap_or_default(),
            ),
            Self::Css(CssRule::Page(rule)) => (
                CssRuleType::Page,
                rule.read_with(&guard).selectors.to_css_string(),
            ),
            Self::Css(CssRule::StartingStyle(_)) => (CssRuleType::StartingStyle, String::new()),
            _ => return None,
        };
        Some((rule_type, prelude))
    }

    fn at_rule_declaration_text(&self, shared_lock: &SharedRwLock) -> Option<String> {
        let guard = shared_lock.read();
        match self {
            Self::Css(CssRule::FontFace(rule)) => Some(rule.read_with(&guard).style_css_text()),
            Self::Css(CssRule::Page(rule)) => Some(serialize_declaration_block(
                &rule.read_with(&guard).block,
                &guard,
            )),
            _ => None,
        }
    }

    fn condition_rule_read(
        &self,
        shared_lock: &SharedRwLock,
    ) -> Option<LiveStylesheetConditionRuleRead> {
        let guard = shared_lock.read();
        let read = match self {
            Self::Css(CssRule::Media(rule)) => LiveStylesheetConditionRuleRead {
                rule_type: CssRuleType::Media,
                condition_text: rule.media_queries.read_with(&guard).to_css_string(),
                container_name: None,
                container_query: None,
                scope_start: None,
                scope_end: None,
            },
            Self::Css(CssRule::Supports(rule)) => LiveStylesheetConditionRuleRead {
                rule_type: CssRuleType::Supports,
                condition_text: rule.condition.to_css_string(),
                container_name: None,
                container_query: None,
                scope_start: None,
                scope_end: None,
            },
            Self::Css(CssRule::Container(rule)) => {
                let (container_name, container_query) =
                    container_rule_name_and_query(&rule.conditions);
                LiveStylesheetConditionRuleRead {
                    rule_type: CssRuleType::Container,
                    condition_text: rule.conditions.to_css_string(),
                    container_name,
                    container_query,
                    scope_start: None,
                    scope_end: None,
                }
            }
            Self::Css(CssRule::Scope(rule)) => LiveStylesheetConditionRuleRead {
                rule_type: CssRuleType::Scope,
                condition_text: scope_rule_condition_text(rule),
                container_name: None,
                container_query: None,
                scope_start: rule
                    .bounds
                    .start
                    .as_ref()
                    .map(cssparser::ToCss::to_css_string),
                scope_end: rule
                    .bounds
                    .end
                    .as_ref()
                    .map(cssparser::ToCss::to_css_string),
            },
            _ => return None,
        };
        Some(read)
    }

    fn layer_rule_read(&self) -> Option<LiveStylesheetLayerRuleRead> {
        let read = match self {
            Self::Css(CssRule::LayerBlock(rule)) => LiveStylesheetLayerRuleRead {
                rule_type: CssRuleType::LayerBlock,
                name: rule.name.as_ref().map(ToCss::to_css_string),
                names: Vec::new(),
            },
            Self::Css(CssRule::LayerStatement(rule)) => LiveStylesheetLayerRuleRead {
                rule_type: CssRuleType::LayerStatement,
                name: None,
                names: rule.names.iter().map(ToCss::to_css_string).collect(),
            },
            _ => return None,
        };
        Some(read)
    }

    fn keyframes_name(&self, shared_lock: &SharedRwLock) -> Option<String> {
        let Self::Css(CssRule::Keyframes(rule)) = self else {
            return None;
        };
        let guard = shared_lock.read();
        Some(rule.read_with(&guard).name.as_atom().to_string())
    }

    fn page_rule_read(&self, shared_lock: &SharedRwLock) -> Option<LiveStylesheetPageRuleRead> {
        let Self::Css(CssRule::Page(rule)) = self else {
            return None;
        };
        let guard = shared_lock.read();
        let rule = rule.read_with(&guard);
        Some(LiveStylesheetPageRuleRead {
            selector_text: rule.selectors.to_css_string(),
            declaration_text: serialize_declaration_block(&rule.block, &guard),
        })
    }

    #[cfg(test)]
    fn ptr_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Css(left), Self::Css(right)) => css_rules_ptr_eq(left, right),
            (Self::Keyframe(left), Self::Keyframe(right)) => ServoArc::ptr_eq(left, right),
            _ => false,
        }
    }
}

#[cfg(test)]
fn css_rules_ptr_eq(left: &CssRule, right: &CssRule) -> bool {
    macro_rules! same_variant {
        ($variant:ident) => {
            match (left, right) {
                (CssRule::$variant(left), CssRule::$variant(right)) => {
                    ServoArc::ptr_eq(left, right)
                }
                _ => false,
            }
        };
    }

    same_variant!(Style)
        || same_variant!(Namespace)
        || same_variant!(Import)
        || same_variant!(Media)
        || same_variant!(CustomMedia)
        || same_variant!(Container)
        || same_variant!(FontFace)
        || same_variant!(FontFeatureValues)
        || same_variant!(FontPaletteValues)
        || same_variant!(CounterStyle)
        || same_variant!(Keyframes)
        || same_variant!(Margin)
        || same_variant!(Supports)
        || same_variant!(Page)
        || same_variant!(Property)
        || same_variant!(Document)
        || same_variant!(LayerBlock)
        || same_variant!(LayerStatement)
        || same_variant!(Scope)
        || same_variant!(StartingStyle)
        || same_variant!(AppearanceBase)
        || same_variant!(PositionTry)
        || same_variant!(NestedDeclarations)
        || same_variant!(ViewTransition)
}

#[derive(Clone, Debug)]
pub(crate) struct StylesheetRuleWrapperBinding {
    stylesheet_id: Option<StylesheetId>,
    path: Vec<usize>,
    rule: NativeStylesheetRule,
    shared_lock: SharedRwLock,
    source_declaration_override: LiveStylesheetSourceDeclarationOverrideState,
    detached_source_declaration_override: Option<String>,
}

impl StylesheetRuleWrapperBinding {
    fn snapshot(&self) -> CssRuleSnapshot {
        let mut snapshot = self.rule.snapshot(&self.shared_lock);
        if let Some(declaration_text) = self.source_declaration_override() {
            snapshot.declaration_text = Some(declaration_text.clone());
            if let Some(selector_text) = snapshot.selector_text.as_deref() {
                snapshot.css_text = format!("{selector_text} {{ {declaration_text} }}");
            }
        }
        snapshot
    }

    pub(crate) fn rule_type(&self) -> CssRuleType {
        self.rule.rule_type()
    }

    pub(crate) fn css_text(&self) -> String {
        if let Some(declaration_text) = self.source_declaration_override()
            && let Some(selector_text) = self.rule.style_selector_text(&self.shared_lock)
        {
            return format!("{selector_text} {{ {declaration_text} }}");
        }
        self.rule.css_text(&self.shared_lock)
    }

    fn source_declaration_override(&self) -> Option<String> {
        if self.stylesheet_id.is_none() {
            return self.detached_source_declaration_override.clone();
        }
        let source_override = self.source_declaration_override.borrow();
        source_override
            .as_ref()
            .filter(|source_override| source_override.rule_path == self.path)
            .map(|source_override| source_override.declaration_text.clone())
    }

    pub(crate) fn style_selector_text(&self) -> Option<String> {
        self.rule.style_selector_text(&self.shared_lock)
    }

    pub(crate) fn keyframe_selector_text(&self) -> Option<String> {
        self.rule.keyframe_selector_text(&self.shared_lock)
    }

    pub(crate) fn style_has_child_rules(&self) -> Option<bool> {
        self.rule.style_has_child_rules(&self.shared_lock)
    }

    pub(crate) fn grouping_prelude(&self) -> Option<(CssRuleType, String)> {
        self.rule.grouping_prelude(&self.shared_lock)
    }

    pub(crate) fn at_rule_declaration_text(&self) -> Option<String> {
        self.rule.at_rule_declaration_text(&self.shared_lock)
    }

    pub(crate) fn condition_rule_read(&self) -> Option<LiveStylesheetConditionRuleRead> {
        self.rule.condition_rule_read(&self.shared_lock)
    }

    pub(crate) fn layer_rule_read(&self) -> Option<LiveStylesheetLayerRuleRead> {
        self.rule.layer_rule_read()
    }

    pub(crate) fn keyframes_name(&self) -> Option<String> {
        self.rule.keyframes_name(&self.shared_lock)
    }

    pub(crate) fn page_rule_read(&self) -> Option<LiveStylesheetPageRuleRead> {
        self.rule.page_rule_read(&self.shared_lock)
    }

    #[cfg(test)]
    pub(crate) const fn stylesheet_id(&self) -> Option<StylesheetId> {
        self.stylesheet_id
    }

    #[cfg(test)]
    pub(crate) fn rule(&self) -> &NativeStylesheetRule {
        &self.rule
    }

    fn mark_detached(&mut self) {
        self.detached_source_declaration_override = self.source_declaration_override();
        self.stylesheet_id = None;
        self.path.clear();
    }
}

fn serialize_declaration_block(
    block: &ServoArc<Locked<PropertyDeclarationBlock>>,
    guard: &SharedRwLockReadGuard<'_>,
) -> String {
    let mut text = CssStringWriter::new();
    block
        .read_with(guard)
        .to_css(&mut text)
        .expect("serializing declarations into a String must not fail");
    text.trim_end().to_owned()
}

fn serialize_page_rule_css_text(selector: &str, block: &str) -> String {
    match (selector.is_empty(), block.is_empty()) {
        (true, true) => "@page { }".to_owned(),
        (false, true) => format!("@page {selector} {{ }}"),
        (true, false) => format!("@page {{ {block} }}"),
        (false, false) => format!("@page {selector} {{ {block} }}"),
    }
}

fn container_rule_name_and_query(
    conditions: &style::stylesheets::container_rule::ContainerConditions,
) -> (Option<String>, Option<String>) {
    let Some(first) = conditions.0.iter().next() else {
        return (None, None);
    };
    let name = (!first.name().is_none()).then(|| first.name().to_css_string());
    let mut query_parts = Vec::new();
    if name.is_some() {
        if let Some(condition) = first.query_condition() {
            query_parts.push(condition.to_css_string());
        }
    } else {
        query_parts.push(first.to_css_string());
    }
    query_parts.extend(conditions.0.iter().skip(1).map(ToCss::to_css_string));
    let query = (!query_parts.is_empty()).then(|| query_parts.join(", "));
    (name, query)
}

fn scope_rule_condition_text(rule: &style::stylesheets::ScopeRule) -> String {
    let mut components = Vec::new();
    if let Some(start) = rule.bounds.start.as_ref() {
        components.push(format!("({})", cssparser::ToCss::to_css_string(start)));
    }
    if let Some(end) = rule.bounds.end.as_ref() {
        components.push(format!("to ({})", cssparser::ToCss::to_css_string(end)));
    }
    components.join(" ")
}

pub(crate) type StylesheetRuleWrapperLease = Rc<RefCell<Option<StylesheetRuleWrapperBinding>>>;

#[derive(Debug, Default)]
pub(crate) struct LiveStylesheetSelectorNamespaces {
    pub(crate) default_namespace_uri: Option<String>,
    pub(crate) namespace_prefixes: Vec<(String, String)>,
}

#[derive(Debug)]
pub(crate) struct LiveStylesheetConditionRuleRead {
    pub(crate) rule_type: CssRuleType,
    pub(crate) condition_text: String,
    pub(crate) container_name: Option<String>,
    pub(crate) container_query: Option<String>,
    pub(crate) scope_start: Option<String>,
    pub(crate) scope_end: Option<String>,
}

#[derive(Debug)]
pub(crate) struct LiveStylesheetLayerRuleRead {
    pub(crate) rule_type: CssRuleType,
    pub(crate) name: Option<String>,
    pub(crate) names: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct LiveStylesheetPageRuleRead {
    pub(crate) selector_text: String,
    pub(crate) declaration_text: String,
}

#[derive(Debug)]
pub(crate) enum LiveStylesheetRuleWrapperSeed {
    Style {
        selector_text: String,
        declaration_text: String,
    },
    Keyframe {
        key_text: String,
        declaration_text: String,
    },
    NestedDeclarations {
        declaration_text: String,
    },
    Media {
        media_text: String,
    },
    Page {
        declaration_text: String,
    },
    TypedAtRule(CssRuleType),
    GenericAtRule(CssRuleType),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FontFeatureValuesMapGroup {
    Annotation,
    Ornaments,
    Stylistic,
    Styleset,
    CharacterVariant,
    Swash,
}

fn css_rule_insert_error(error: RulesMutateError) -> CssRuleInsertError {
    match error {
        RulesMutateError::Syntax => CssRuleInsertError::Syntax,
        RulesMutateError::IndexSize => CssRuleInsertError::IndexSize,
        RulesMutateError::HierarchyRequest => CssRuleInsertError::HierarchyRequest,
        RulesMutateError::InvalidState => CssRuleInsertError::InvalidState,
    }
}

fn css_text_starts_with_at_keyword(css_text: &str, expected: &str) -> bool {
    let mut input = ParserInput::new(css_text);
    let mut parser = Parser::new(&mut input);
    loop {
        match parser.next_including_whitespace_and_comments() {
            Ok(Token::WhiteSpace(_) | Token::Comment(_)) => {}
            Ok(Token::AtKeyword(keyword)) => return keyword.eq_ignore_ascii_case(expected),
            _ => return false,
        }
    }
}

fn child_rule_path(parent_path: &[usize], index: usize) -> Vec<usize> {
    let mut path = Vec::with_capacity(parent_path.len() + 1);
    path.extend_from_slice(parent_path);
    path.push(index);
    path
}

fn top_level_rule_index(rule_path: &[usize]) -> Result<usize, CssRuleInsertError> {
    match rule_path {
        [index] => Ok(*index),
        _ => Err(CssRuleInsertError::HierarchyRequest),
    }
}

fn update_font_feature_values_entry<T>(
    entries: &mut Vec<FFVDeclaration<T>>,
    name: style::Atom,
    value: T,
) {
    if let Some(entry) = entries.iter_mut().find(|entry| entry.name == name) {
        entry.value = value;
    } else {
        entries.push(FFVDeclaration { name, value });
    }
}

fn delete_font_feature_values_entry<T>(
    entries: &mut Vec<FFVDeclaration<T>>,
    name: &style::Atom,
) -> bool {
    let Some(index) = entries.iter().position(|entry| &entry.name == name) else {
        return false;
    };
    entries.remove(index);
    true
}

fn font_feature_values_rule_has_entry(
    rule: &FontFeatureValuesRule,
    group: FontFeatureValuesMapGroup,
    name: &style::Atom,
) -> bool {
    match group {
        FontFeatureValuesMapGroup::Annotation => {
            rule.annotation.iter().any(|entry| &entry.name == name)
        }
        FontFeatureValuesMapGroup::Ornaments => {
            rule.ornaments.iter().any(|entry| &entry.name == name)
        }
        FontFeatureValuesMapGroup::Stylistic => {
            rule.stylistic.iter().any(|entry| &entry.name == name)
        }
        FontFeatureValuesMapGroup::Styleset => {
            rule.styleset.iter().any(|entry| &entry.name == name)
        }
        FontFeatureValuesMapGroup::CharacterVariant => rule
            .character_variant
            .iter()
            .any(|entry| &entry.name == name),
        FontFeatureValuesMapGroup::Swash => rule.swash.iter().any(|entry| &entry.name == name),
    }
}

const MAX_DATA_STYLESHEET_IMPORT_EXPANSIONS: usize = 16;
const MAX_DATA_STYLESHEET_IMPORT_URL_BYTES: usize = 16 * 1024;

pub(crate) fn import_url_identity(url: &url::Url) -> url::Url {
    let mut identity = url.clone();
    identity.set_fragment(None);
    identity
}

fn decode_data_stylesheet(url: &url::Url) -> Option<String> {
    if url.as_str().len() > MAX_DATA_STYLESHEET_IMPORT_URL_BYTES {
        return None;
    }
    let (body, mime_type) = moli_web_mime::data_url_body_and_mime_type(url.as_str())?;
    // A data: URL selected by a stylesheet request is parsed as CSS by
    // Chromium regardless of its declared media type. Network stylesheet MIME
    // validation is intentionally kept out of this local-scheme decoder.
    Some(moli_encoding::decode_text_for_legacy_web(
        &body,
        moli_web_mime::mime_charset(&mime_type).as_deref(),
    ))
}

fn parse_keyframe_selectors(selector_text: &str) -> Result<KeyframeSelectors, CssRuleInsertError> {
    let mut input = ParserInput::new(selector_text);
    let mut input = Parser::new(&mut input);
    input
        .parse_entirely(KeyframeSelectors::parse)
        .map_err(|_| CssRuleInsertError::Syntax)
}

struct LiveStylesheetImportPlaceholderLoader;

impl StylesheetLoader for LiveStylesheetImportPlaceholderLoader {
    fn request_stylesheet(
        &self,
        url: CssUrl,
        location: SourceLocation,
        lock: &SharedRwLock,
        media: ServoArc<Locked<MediaList>>,
        supports: Option<ImportSupportsCondition>,
        layer: ImportLayer,
    ) -> ServoArc<Locked<ImportRule>> {
        // Parsing owns rule identity only. DocumentRuntime remains the sole
        // owner of import admission, network scheduling, cancellation and
        // terminal events until it installs the real child stylesheet.
        if supports
            .as_ref()
            .is_some_and(|condition| !condition.enabled)
            || url.url().is_none()
        {
            return ServoArc::new(lock.wrap(ImportRule {
                url,
                stylesheet: ImportSheet::new_refused(),
                supports,
                layer,
                source_location: location,
            }));
        }

        let base_url = url
            .url()
            .expect("resolved import URL was checked above")
            .as_ref()
            .clone();
        let contents = StylesheetContents::from_str(
            "",
            UrlExtraData::from(base_url),
            Origin::Author,
            lock,
            None,
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::No,
            None,
        );
        let stylesheet = ServoArc::new(Stylesheet {
            contents: lock.wrap(contents),
            shared_lock: lock.clone(),
            media,
            disabled: AtomicBool::new(false),
        });
        ServoArc::new(lock.wrap(ImportRule {
            url,
            stylesheet: ImportSheet::new(stylesheet),
            supports,
            layer,
            source_location: location,
        }))
    }
}

fn parse_live_stylesheet(
    css_text: &str,
    base_url: &url::Url,
    media: ServoArc<Locked<MediaList>>,
    shared_lock: SharedRwLock,
    quirks_mode: QuirksMode,
    allow_import_rules: AllowImportRules,
) -> Stylesheet {
    let import_loader = LiveStylesheetImportPlaceholderLoader;
    let stylesheet_loader = match allow_import_rules {
        AllowImportRules::Yes => Some(&import_loader as &dyn StylesheetLoader),
        AllowImportRules::No => None,
    };
    Stylesheet::from_str(
        css_text,
        UrlExtraData::from(base_url.clone()),
        Origin::Author,
        media,
        shared_lock,
        stylesheet_loader,
        None,
        quirks_mode,
        allow_import_rules,
    )
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StylesheetImportEdgeId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveStylesheetImportState {
    Pending,
    Refused,
    Loaded { successful: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveStylesheetRuntimeStateChange {
    Unchanged,
    CssomOnly,
    Cascade,
}

impl LiveStylesheetRuntimeStateChange {
    pub(crate) const fn affects_cascade(self) -> bool {
        matches!(self, Self::Cascade)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveStylesheetRuntimeStateKind {
    Cascade,
    IndependentCssom,
}

#[derive(Debug)]
enum LiveStylesheetCssomRuntimeState {
    Cascade,
    Independent {
        media: ServoArc<Locked<MediaList>>,
        disabled: Cell<bool>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct LiveStylesheetImportRequest {
    pub(crate) edge_id: StylesheetImportEdgeId,
    pub(crate) url: url::Url,
}

#[derive(Clone, Debug)]
pub(crate) struct LiveStylesheetImportResponse {
    pub(crate) request_url: url::Url,
    pub(crate) response_url: url::Url,
    pub(crate) css_text: String,
    pub(crate) successful: bool,
    pub(crate) origin_clean: bool,
}

#[derive(Debug)]
struct LiveStylesheetImportEdge {
    id: StylesheetImportEdgeId,
    rule: ServoArc<Locked<ImportRule>>,
    state: LiveStylesheetImportState,
    child: Option<LiveStylesheetRef>,
}

#[derive(Debug)]
struct LiveStylesheetParent {
    stylesheet: RcWeak<LiveStylesheet>,
    edge_id: StylesheetImportEdgeId,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct StylesheetContentsCacheKey {
    css_fingerprint: [u8; 32],
    css_len: usize,
    base_url: url::Url,
    quirks_mode: QuirksMode,
    allow_import_rules: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LiveStylesheetSourceDeclarationOverride {
    rule_path: Vec<usize>,
    declaration_text: String,
}

type LiveStylesheetSourceDeclarationOverrideState =
    Rc<RefCell<Option<LiveStylesheetSourceDeclarationOverride>>>;

impl StylesheetContentsCacheKey {
    fn new(
        css_text: &str,
        base_url: &url::Url,
        quirks_mode: QuirksMode,
        allow_import_rules: AllowImportRules,
    ) -> Self {
        let mut hasher = Sha256Context::new();
        hasher.update(css_text.len().to_le_bytes());
        hasher.update(css_text.as_bytes());
        Self {
            css_fingerprint: hasher.finish(),
            css_len: css_text.len(),
            base_url: base_url.clone(),
            quirks_mode,
            allow_import_rules: matches!(allow_import_rules, AllowImportRules::Yes),
        }
    }
}

#[derive(Debug)]
pub(crate) struct SharedStylesheetContents {
    contents: ServoArc<StylesheetContents>,
    base_url: url::Url,
    quirks_mode: QuirksMode,
    allow_import_rules: AllowImportRules,
    source_declaration_override: Option<LiveStylesheetSourceDeclarationOverride>,
}

/// Renderer-owned identity for one authoritative parsed stylesheet.
///
/// CSSOM wrappers and installed style clients retain this handle. The Stylo
/// stylesheet is the only parsed truth; serialized text is a revision-scoped
/// read cache, never an independently mutable source.
#[derive(Debug)]
pub(crate) struct LiveStylesheet {
    id: StylesheetId,
    stylesheet: ServoArc<Stylesheet>,
    base_url: url::Url,
    quirks_mode: QuirksMode,
    allow_import_rules: AllowImportRules,
    origin_clean: Cell<bool>,
    cssom_runtime_state: LiveStylesheetCssomRuntimeState,
    contents_revision: Cell<u64>,
    cascade_generation: Cell<u64>,
    cascade_mutations: StdArc<Mutex<LiveStylesheetCascadeMutationJournal>>,
    import_generation: Cell<u64>,
    parent: RefCell<Option<LiveStylesheetParent>>,
    next_import_edge_id: Cell<u64>,
    import_edges: RefCell<Vec<LiveStylesheetImportEdge>>,
    derived_state: StdArc<LiveStylesheetDerivedState>,
    font_face_cache: RefCell<
        Option<(
            FontFaceProjectionKey,
            Rc<crate::style_engine::StylesheetFontFaceProjection>,
        )>,
    >,
    next_font_face_rule_identity: Cell<u64>,
    font_face_rule_identities: RefCell<Vec<FontFaceRuleIdentity>>,
    shared_initial_contents: RefCell<Option<StdArc<SharedStylesheetContents>>>,
    source_declaration_override: LiveStylesheetSourceDeclarationOverrideState,
    rule_wrapper_leases: RefCell<
        HashMap<
            StylesheetRuleWrapperLeaseId,
            RcWeak<RefCell<Option<StylesheetRuleWrapperBinding>>>,
        >,
    >,
}

/// One exact Stylo rule mutation retained within the bounded journal window.
#[derive(Clone, Debug)]
pub(crate) struct LiveStylesheetRuleMutation {
    rule: CssRule,
    ancestors: Vec<CssRule>,
    change_kind: RuleChangeKind,
}

impl LiveStylesheetRuleMutation {
    pub(crate) fn new(rule: CssRule, ancestors: Vec<CssRule>, change_kind: RuleChangeKind) -> Self {
        Self {
            rule,
            ancestors,
            change_kind,
        }
    }

    pub(crate) fn rule(&self) -> &CssRule {
        &self.rule
    }

    pub(crate) fn ancestors(&self) -> &[CssRule] {
        &self.ancestors
    }

    pub(crate) fn change_kind(&self) -> RuleChangeKind {
        self.change_kind
    }
}

#[derive(Clone, Debug)]
pub(crate) enum LiveStylesheetCascadeMutation {
    Full,
    Rules(Vec<LiveStylesheetRuleMutation>),
}

#[derive(Clone, Debug)]
pub(crate) struct LiveStylesheetCascadeMutationBatch {
    generation: u64,
    mutation: LiveStylesheetCascadeMutation,
}

impl LiveStylesheetCascadeMutationBatch {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn mutation(&self) -> &LiveStylesheetCascadeMutation {
        &self.mutation
    }
}

#[derive(Debug)]
struct FontFaceRuleIdentity {
    id: u64,
    rule: ServoArc<Locked<FontFaceRule>>,
    fingerprint: String,
}

#[derive(Debug, Default)]
pub(crate) struct LiveStylesheetDerivedState {
    serialized_css_text_cache: Mutex<Option<(u64, StdArc<str>)>>,
    dependency_summary_cache: Mutex<
        Option<(
            (u64, u64),
            StdArc<moli_selector::StyloSourceDependencySummary>,
        )>,
    >,
}

impl LiveStylesheetDerivedState {
    pub(crate) fn serialized_css_text(
        &self,
        contents_revision: u64,
        build: impl FnOnce() -> String,
    ) -> StdArc<str> {
        let mut cache = self.serialized_css_text_cache.lock();
        if let Some((cached_revision, css_text)) = cache.as_ref()
            && *cached_revision == contents_revision
        {
            return StdArc::clone(css_text);
        }

        #[cfg(test)]
        LIVE_STYLESHEET_CSS_TEXT_PROJECTION_COUNT.with(|count| count.set(count.get() + 1));
        let css_text = StdArc::<str>::from(build());
        *cache = Some((contents_revision, StdArc::clone(&css_text)));
        css_text
    }

    fn clear_serialized_css_text(&self) {
        self.serialized_css_text_cache.lock().take();
    }

    pub(crate) fn source_dependency_summary(
        &self,
        contents_revision: u64,
        cascade_generation: u64,
        build: impl FnOnce() -> moli_selector::StyloSourceDependencySummary,
    ) -> StdArc<moli_selector::StyloSourceDependencySummary> {
        let revision = (contents_revision, cascade_generation);
        let mut cache = self.dependency_summary_cache.lock();
        if let Some((cached_revision, summary)) = cache.as_ref()
            && *cached_revision == revision
        {
            return StdArc::clone(summary);
        }

        #[cfg(test)]
        LIVE_STYLESHEET_DEPENDENCY_SUMMARY_PROJECTION_COUNT
            .with(|count| count.set(count.get() + 1));
        let summary = StdArc::new(build());
        *cache = Some((revision, StdArc::clone(&summary)));
        summary
    }

    fn clear_dependency_summary(&self) {
        self.dependency_summary_cache.lock().take();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FontFaceProjectionKey {
    contents_revision: u64,
    cascade_generation: u64,
    environment: crate::style_engine::StyloStyleEnvironment,
    viewport_width: Option<u64>,
    viewport_height: Option<u64>,
    screen_width: Option<u64>,
    screen_height: Option<u64>,
}

impl FontFaceProjectionKey {
    fn new(
        stylesheet: &LiveStylesheet,
        environment: crate::style_engine::StyloStyleEnvironment,
        viewport: crate::style_engine::StyleViewport,
    ) -> Self {
        Self {
            contents_revision: stylesheet.contents_revision(),
            cascade_generation: stylesheet.cascade_generation(),
            environment,
            viewport_width: viewport.width.map(f64::to_bits),
            viewport_height: viewport.height.map(f64::to_bits),
            screen_width: viewport.screen_width.map(f64::to_bits),
            screen_height: viewport.screen_height.map(f64::to_bits),
        }
    }
}

impl LiveStylesheet {
    fn parse(
        id: StylesheetId,
        css_text: &str,
        base_url: url::Url,
        quirks_mode: QuirksMode,
        allow_import_rules: AllowImportRules,
        shared_lock: SharedRwLock,
    ) -> Self {
        #[cfg(test)]
        LIVE_STYLESHEET_PARSE_COUNT.with(|count| count.set(count.get() + 1));
        let media = ServoArc::new(shared_lock.wrap(MediaList::empty()));
        Self::parse_with_media(
            id,
            css_text,
            base_url,
            quirks_mode,
            allow_import_rules,
            shared_lock,
            media,
            LiveStylesheetRuntimeStateKind::Cascade,
        )
    }

    fn parse_with_media(
        id: StylesheetId,
        css_text: &str,
        base_url: url::Url,
        quirks_mode: QuirksMode,
        allow_import_rules: AllowImportRules,
        shared_lock: SharedRwLock,
        media: ServoArc<Locked<MediaList>>,
        runtime_state_kind: LiveStylesheetRuntimeStateKind,
    ) -> Self {
        let stylesheet = ServoArc::new(parse_live_stylesheet(
            css_text,
            &base_url,
            media,
            shared_lock,
            quirks_mode,
            allow_import_rules,
        ));
        let source_declaration_override =
            stylesheet_eof_open_var_declaration_override(&stylesheet, css_text);
        Self::from_stylesheet(
            id,
            stylesheet,
            base_url,
            quirks_mode,
            allow_import_rules,
            runtime_state_kind,
            None,
            source_declaration_override,
        )
    }

    fn from_shared_initial_contents(
        id: StylesheetId,
        shared_lock: SharedRwLock,
        shared_contents: StdArc<SharedStylesheetContents>,
    ) -> Self {
        let source_declaration_override = shared_contents.source_declaration_override.clone();
        let media = ServoArc::new(shared_lock.wrap(MediaList::empty()));
        let stylesheet = ServoArc::new(Stylesheet {
            contents: shared_lock.wrap(shared_contents.contents.clone()),
            shared_lock,
            media,
            disabled: AtomicBool::new(false),
        });
        Self::from_stylesheet(
            id,
            stylesheet,
            shared_contents.base_url.clone(),
            shared_contents.quirks_mode,
            shared_contents.allow_import_rules,
            LiveStylesheetRuntimeStateKind::Cascade,
            Some(shared_contents),
            source_declaration_override,
        )
    }

    fn from_stylesheet(
        id: StylesheetId,
        stylesheet: ServoArc<Stylesheet>,
        base_url: url::Url,
        quirks_mode: QuirksMode,
        allow_import_rules: AllowImportRules,
        runtime_state_kind: LiveStylesheetRuntimeStateKind,
        shared_initial_contents: Option<StdArc<SharedStylesheetContents>>,
        source_declaration_override: Option<LiveStylesheetSourceDeclarationOverride>,
    ) -> Self {
        let cssom_runtime_state = match runtime_state_kind {
            LiveStylesheetRuntimeStateKind::Cascade => LiveStylesheetCssomRuntimeState::Cascade,
            LiveStylesheetRuntimeStateKind::IndependentCssom => {
                LiveStylesheetCssomRuntimeState::Independent {
                    media: ServoArc::new(stylesheet.shared_lock.wrap(MediaList::empty())),
                    disabled: Cell::new(false),
                }
            }
        };
        let stylesheet = Self {
            id,
            stylesheet,
            base_url,
            quirks_mode,
            allow_import_rules,
            origin_clean: Cell::new(true),
            cssom_runtime_state,
            contents_revision: Cell::new(1),
            cascade_generation: Cell::new(1),
            cascade_mutations: StdArc::new(Mutex::new(VecDeque::new())),
            import_generation: Cell::new(1),
            parent: RefCell::new(None),
            next_import_edge_id: Cell::new(0),
            import_edges: RefCell::new(Vec::new()),
            derived_state: StdArc::new(LiveStylesheetDerivedState::default()),
            font_face_cache: RefCell::new(None),
            next_font_face_rule_identity: Cell::new(0),
            font_face_rule_identities: RefCell::new(Vec::new()),
            shared_initial_contents: RefCell::new(shared_initial_contents),
            source_declaration_override: Rc::new(RefCell::new(source_declaration_override)),
            rule_wrapper_leases: RefCell::new(HashMap::new()),
        };
        stylesheet.reconcile_import_edges();
        stylesheet
    }

    fn initial_contents_can_be_shared(&self) -> bool {
        let guard = self.stylesheet.shared_lock.read();
        let rules = self.stylesheet.contents(&guard).rules(&guard);
        !(rules.is_empty() || rules.iter().any(|rule| matches!(rule, CssRule::Import(_))))
    }

    pub(crate) fn share_initial_contents(&self) -> StdArc<SharedStylesheetContents> {
        if let Some(shared_contents) = self.shared_initial_contents.borrow().as_ref() {
            return StdArc::clone(shared_contents);
        }
        let contents = {
            let guard = self.stylesheet.shared_lock.read();
            self.stylesheet.contents.read_with(&guard).clone()
        };
        let shared_contents = StdArc::new(SharedStylesheetContents {
            contents,
            base_url: self.base_url.clone(),
            quirks_mode: self.quirks_mode,
            allow_import_rules: self.allow_import_rules,
            source_declaration_override: self.source_declaration_override.borrow().clone(),
        });
        *self.shared_initial_contents.borrow_mut() = Some(StdArc::clone(&shared_contents));
        shared_contents
    }

    pub(crate) fn shared_initial_contents(&self) -> Option<StdArc<SharedStylesheetContents>> {
        self.shared_initial_contents
            .borrow()
            .as_ref()
            .map(StdArc::clone)
    }

    pub(crate) fn stylesheet(&self) -> ServoArc<Stylesheet> {
        self.stylesheet.clone()
    }

    pub(crate) fn base_url(&self) -> &url::Url {
        &self.base_url
    }

    pub(crate) fn media_text(&self) -> String {
        let guard = self.stylesheet.shared_lock.read();
        self.cssom_media().read_with(&guard).to_css_string()
    }

    pub(crate) fn contents_revision(&self) -> u64 {
        self.contents_revision.get()
    }

    pub(crate) fn cascade_generation(&self) -> u64 {
        self.cascade_generation.get()
    }

    pub(crate) fn cascade_mutation_journal(
        &self,
    ) -> StdArc<Mutex<LiveStylesheetCascadeMutationJournal>> {
        StdArc::clone(&self.cascade_mutations)
    }

    fn publish_cascade_mutation(&self, generation: u64, mutation: LiveStylesheetCascadeMutation) {
        let mut journal = self.cascade_mutations.lock();
        if journal.len() == LIVE_STYLESHEET_CASCADE_MUTATION_JOURNAL_CAPACITY {
            journal.pop_front();
        }
        journal.push_back(LiveStylesheetCascadeMutationBatch {
            generation,
            mutation,
        });
    }

    pub(crate) fn import_generation(&self) -> u64 {
        self.import_generation.get()
    }

    pub(crate) fn set_origin_clean(&self, origin_clean: bool) {
        self.origin_clean.set(origin_clean);
    }

    pub(crate) fn origin_clean(&self) -> bool {
        self.origin_clean.get()
    }

    pub(crate) fn derived_state(&self) -> StdArc<LiveStylesheetDerivedState> {
        StdArc::clone(&self.derived_state)
    }

    pub(crate) fn set_media_text(&self, media_text: &str) -> LiveStylesheetRuntimeStateChange {
        let media = crate::style_engine::media_list::parse_media_query_list_with_context(
            media_text,
            &self.base_url,
            self.quirks_mode,
        );
        let shared_lock = &self.stylesheet.shared_lock;
        let mut guard = shared_lock.write();
        let current = self.cssom_media().write_with(&mut guard);
        if current.to_css_string() == media.to_css_string() {
            return LiveStylesheetRuntimeStateChange::Unchanged;
        }
        *current = media;
        drop(guard);
        match self.cssom_runtime_state {
            LiveStylesheetCssomRuntimeState::Cascade => {
                self.note_cascade_mutation();
                LiveStylesheetRuntimeStateChange::Cascade
            }
            LiveStylesheetCssomRuntimeState::Independent { .. } => {
                LiveStylesheetRuntimeStateChange::CssomOnly
            }
        }
    }

    pub(crate) fn set_disabled(&self, disabled: bool) -> LiveStylesheetRuntimeStateChange {
        match &self.cssom_runtime_state {
            LiveStylesheetCssomRuntimeState::Cascade => {
                if !self.stylesheet.set_disabled(disabled) {
                    return LiveStylesheetRuntimeStateChange::Unchanged;
                }
                self.note_cascade_mutation();
                LiveStylesheetRuntimeStateChange::Cascade
            }
            LiveStylesheetCssomRuntimeState::Independent {
                disabled: current, ..
            } => {
                if current.replace(disabled) == disabled {
                    LiveStylesheetRuntimeStateChange::Unchanged
                } else {
                    LiveStylesheetRuntimeStateChange::CssomOnly
                }
            }
        }
    }

    pub(crate) fn disabled(&self) -> bool {
        match &self.cssom_runtime_state {
            LiveStylesheetCssomRuntimeState::Cascade => self.stylesheet.disabled(),
            LiveStylesheetCssomRuntimeState::Independent { disabled, .. } => disabled.get(),
        }
    }

    fn cssom_media(&self) -> &ServoArc<Locked<MediaList>> {
        match &self.cssom_runtime_state {
            LiveStylesheetCssomRuntimeState::Cascade => &self.stylesheet.media,
            LiveStylesheetCssomRuntimeState::Independent { media, .. } => media,
        }
    }

    pub(crate) fn selector_namespaces(&self) -> LiveStylesheetSelectorNamespaces {
        let contents = self.current_contents();
        let default_namespace_uri = contents
            .namespaces
            .default
            .as_ref()
            .map(|namespace| namespace.0.to_string());
        let mut namespace_prefixes = contents
            .namespaces
            .prefixes
            .iter()
            .map(|(prefix, namespace)| (prefix.0.to_string(), namespace.0.to_string()))
            .collect::<Vec<_>>();
        namespace_prefixes.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        LiveStylesheetSelectorNamespaces {
            default_namespace_uri,
            namespace_prefixes,
        }
    }

    pub(crate) fn native_rule_at_path(&self, path: &[usize]) -> Option<NativeStylesheetRule> {
        let (first, rest) = path.split_first()?;
        let guard = self.stylesheet.shared_lock.read();
        let contents = self.stylesheet.contents.read_with(&guard);
        let rules = contents.rules.read_with(&guard);
        let mut current = NativeStylesheetRule::Css(rules.0.get(*first)?.clone());

        for index in rest {
            current = match &current {
                NativeStylesheetRule::Css(CssRule::Keyframes(rule)) => {
                    let rule = rule.read_with(&guard);
                    NativeStylesheetRule::Keyframe(rule.keyframes.get(*index)?.clone())
                }
                NativeStylesheetRule::Css(rule) => {
                    NativeStylesheetRule::Css(rule.children(&guard).get(*index)?.clone())
                }
                NativeStylesheetRule::Keyframe(_) => return None,
            };
        }
        Some(current)
    }

    pub(crate) fn rule_wrapper_seed_at_path(
        &self,
        path: &[usize],
    ) -> Option<LiveStylesheetRuleWrapperSeed> {
        let native_rule = self.native_rule_at_path(path)?;
        let guard = self.stylesheet.shared_lock.read();
        Some(match native_rule {
            NativeStylesheetRule::Css(CssRule::Style(rule)) => {
                let rule = rule.read_with(&guard);
                let declaration_text = self
                    .source_declaration_override
                    .borrow()
                    .as_ref()
                    .filter(|source_override| source_override.rule_path == path)
                    .map(|source_override| source_override.declaration_text.clone())
                    .unwrap_or_else(|| serialize_declaration_block(&rule.block, &guard));
                LiveStylesheetRuleWrapperSeed::Style {
                    selector_text: cssparser::ToCss::to_css_string(&rule.selectors),
                    declaration_text,
                }
            }
            NativeStylesheetRule::Keyframe(rule) => {
                let rule = rule.read_with(&guard);
                LiveStylesheetRuleWrapperSeed::Keyframe {
                    key_text: rule.selector.to_css_string(),
                    declaration_text: serialize_declaration_block(&rule.block, &guard),
                }
            }
            NativeStylesheetRule::Css(CssRule::NestedDeclarations(rule)) => {
                LiveStylesheetRuleWrapperSeed::NestedDeclarations {
                    declaration_text: serialize_declaration_block(
                        &rule.read_with(&guard).block,
                        &guard,
                    ),
                }
            }
            NativeStylesheetRule::Css(CssRule::Media(rule)) => {
                LiveStylesheetRuleWrapperSeed::Media {
                    media_text: rule.media_queries.read_with(&guard).to_css_string(),
                }
            }
            NativeStylesheetRule::Css(CssRule::Page(rule)) => LiveStylesheetRuleWrapperSeed::Page {
                declaration_text: serialize_declaration_block(
                    &rule.read_with(&guard).block,
                    &guard,
                ),
            },
            NativeStylesheetRule::Css(
                rule @ (CssRule::FontFace(_)
                | CssRule::FontFeatureValues(_)
                | CssRule::Margin(_)
                | CssRule::Property(_)),
            ) => LiveStylesheetRuleWrapperSeed::TypedAtRule(rule.rule_type()),
            NativeStylesheetRule::Css(
                rule @ (CssRule::Namespace(_)
                | CssRule::Import(_)
                | CssRule::CustomMedia(_)
                | CssRule::Container(_)
                | CssRule::FontPaletteValues(_)
                | CssRule::CounterStyle(_)
                | CssRule::Keyframes(_)
                | CssRule::Supports(_)
                | CssRule::Document(_)
                | CssRule::LayerBlock(_)
                | CssRule::LayerStatement(_)
                | CssRule::Scope(_)
                | CssRule::StartingStyle(_)
                | CssRule::AppearanceBase(_)
                | CssRule::PositionTry(_)
                | CssRule::ViewTransition(_)),
            ) => LiveStylesheetRuleWrapperSeed::GenericAtRule(rule.rule_type()),
        })
    }

    pub(crate) fn child_rule_count_at_path(&self, path: &[usize]) -> Option<usize> {
        match self.native_rule_at_path(path)? {
            NativeStylesheetRule::Css(CssRule::Keyframes(rule)) => {
                let guard = self.stylesheet.shared_lock.read();
                Some(rule.read_with(&guard).keyframes.len())
            }
            NativeStylesheetRule::Css(rule) => {
                let Some(rules) = self.existing_child_rules_for_rule(&rule) else {
                    return matches!(rule, CssRule::Style(_)).then_some(0);
                };
                let guard = self.stylesheet.shared_lock.read();
                Some(rules.read_with(&guard).0.len())
            }
            NativeStylesheetRule::Keyframe(_) => None,
        }
    }

    pub(crate) fn find_keyframe_rule_index(
        &self,
        parent_path: &[usize],
        selector_text: &str,
    ) -> Option<usize> {
        let selector = parse_keyframe_selectors(selector_text).ok()?;
        let keyframes_rule = self.keyframes_rule_at_path(parent_path)?;
        let guard = self.stylesheet.shared_lock.read();
        keyframes_rule
            .read_with(&guard)
            .keyframes
            .iter()
            .rposition(|rule| rule.read_with(&guard).selector == selector)
    }

    fn existing_child_rules_for_rule(&self, rule: &CssRule) -> Option<ServoArc<Locked<CssRules>>> {
        let guard = self.stylesheet.shared_lock.read();
        match rule {
            CssRule::Style(rule) => rule.read_with(&guard).rules.clone(),
            CssRule::Media(rule) => Some(rule.rules.clone()),
            CssRule::Container(rule) => Some(rule.rules.clone()),
            CssRule::Supports(rule) => Some(rule.rules.clone()),
            CssRule::Page(rule) => Some(rule.read_with(&guard).rules.clone()),
            CssRule::Document(rule) => Some(rule.rules.clone()),
            CssRule::LayerBlock(rule) => Some(rule.rules.clone()),
            CssRule::Scope(rule) => Some(rule.rules.clone()),
            CssRule::StartingStyle(rule) => Some(rule.rules.clone()),
            CssRule::AppearanceBase(rule) => Some(rule.rules.clone()),
            _ => None,
        }
    }

    fn mutable_child_rules_at_path(
        &self,
        parent_path: &[usize],
    ) -> Option<ServoArc<Locked<CssRules>>> {
        let NativeStylesheetRule::Css(rule) = self.native_rule_at_path(parent_path)? else {
            return None;
        };
        if let Some(rules) = self.existing_child_rules_for_rule(&rule) {
            return Some(rules);
        }
        let CssRule::Style(style_rule) = rule else {
            return None;
        };
        let mut guard = self.stylesheet.shared_lock.write();
        let style_rule = style_rule.write_with(&mut guard);
        if style_rule.rules.is_none() {
            style_rule.rules = Some(CssRules::new(Vec::new(), &self.stylesheet.shared_lock));
        }
        style_rule.rules.clone()
    }

    fn keyframes_rule_at_path(
        &self,
        parent_path: &[usize],
    ) -> Option<ServoArc<Locked<KeyframesRule>>> {
        match self.native_rule_at_path(parent_path)? {
            NativeStylesheetRule::Css(CssRule::Keyframes(rule)) => Some(rule),
            _ => None,
        }
    }

    fn nested_rule_count(&self, parent_path: &[usize]) -> Option<usize> {
        let NativeStylesheetRule::Css(rule) = self.native_rule_at_path(parent_path)? else {
            return None;
        };
        if let Some(rules) = self.existing_child_rules_for_rule(&rule) {
            let guard = self.stylesheet.shared_lock.read();
            return Some(rules.read_with(&guard).0.len());
        }
        matches!(rule, CssRule::Style(_)).then_some(0)
    }

    fn keyframe_rule_count(&self, parent_path: &[usize]) -> Option<usize> {
        let rule = self.keyframes_rule_at_path(parent_path)?;
        let guard = self.stylesheet.shared_lock.read();
        Some(rule.read_with(&guard).keyframes.len())
    }

    fn track_rule_wrapper_lease(
        &self,
        id: StylesheetRuleWrapperLeaseId,
        lease: &StylesheetRuleWrapperLease,
    ) {
        let mut leases = self.rule_wrapper_leases.borrow_mut();
        if leases.len().is_multiple_of(256) {
            leases.retain(|_, lease| lease.strong_count() != 0);
        }
        leases.insert(id, Rc::downgrade(lease));
    }

    pub(crate) fn font_faces(
        &self,
        environment: crate::style_engine::StyloStyleEnvironment,
        viewport: crate::style_engine::StyleViewport,
    ) -> Rc<crate::style_engine::StylesheetFontFaceProjection> {
        let key = FontFaceProjectionKey::new(self, environment, viewport);
        if let Some((cached_key, font_faces)) = self.font_face_cache.borrow().as_ref()
            && *cached_key == key
        {
            return Rc::clone(font_faces);
        }
        #[cfg(test)]
        LIVE_STYLESHEET_FONT_FACE_PROJECTION_COUNT.with(|count| count.set(count.get() + 1));
        let native_projection = crate::style_engine::native_font_face_projection_for_stylesheet(
            &self.stylesheet,
            environment,
            viewport,
        );
        let font_faces = Rc::new(self.assign_font_face_rule_identities(native_projection));
        *self.font_face_cache.borrow_mut() = Some((key, Rc::clone(&font_faces)));
        font_faces
    }

    fn assign_font_face_rule_identities(
        &self,
        native_projection: crate::style_engine::NativeStylesheetFontFaceProjection,
    ) -> crate::style_engine::StylesheetFontFaceProjection {
        let mut identities = self.font_face_rule_identities.borrow_mut();
        let mut projection = crate::style_engine::StylesheetFontFaceProjection::default();

        for native_rule in &native_projection.all_rules {
            let rule_address = native_rule.rule.raw_ptr().as_ptr() as usize;
            let identity = match identities
                .iter_mut()
                .find(|identity| ServoArc::ptr_eq(&identity.rule, &native_rule.rule))
            {
                Some(identity) if identity.fingerprint == native_rule.rule_fingerprint => {
                    identity.id
                }
                Some(identity) => {
                    let id = self.allocate_font_face_rule_identity();
                    identity.id = id;
                    identity.fingerprint = native_rule.rule_fingerprint.clone();
                    id
                }
                None => {
                    let id = self.allocate_font_face_rule_identity();
                    identities.push(FontFaceRuleIdentity {
                        id,
                        rule: native_rule.rule.clone(),
                        fingerprint: native_rule.rule_fingerprint.clone(),
                    });
                    id
                }
            };
            projection
                .all_rules
                .push(crate::style_engine::StylesheetFontFaceRuleProjection {
                    rule_identity: identity,
                    rule_fingerprint: native_rule.rule_fingerprint.clone(),
                    descriptor: native_rule.descriptor.clone(),
                });
            if native_projection
                .effective_rule_addresses
                .contains(&rule_address)
            {
                projection.effective_rule_identities.insert(identity);
            }
        }

        identities.retain(|identity| {
            native_projection
                .all_rules
                .iter()
                .any(|native_rule| ServoArc::ptr_eq(&identity.rule, &native_rule.rule))
        });
        projection
    }

    fn allocate_font_face_rule_identity(&self) -> u64 {
        let next = self
            .next_font_face_rule_identity
            .get()
            .checked_add(1)
            .expect("font-face rule identity space exhausted");
        self.next_font_face_rule_identity.set(next);
        next
    }

    pub(crate) fn ensure_owned_contents_for_mutation(&self) {
        let Some(shared_contents) = self.shared_initial_contents.borrow_mut().take() else {
            return;
        };
        if StdArc::strong_count(&shared_contents) == 1 {
            return;
        }

        let preserve_font_face_identities = !self.font_face_rule_identities.borrow().is_empty();
        let previous_font_face_rules = preserve_font_face_identities
            .then(|| crate::style_engine::native_font_face_rules_for_stylesheet(&self.stylesheet));
        let shared_lock = self.stylesheet.shared_lock.clone();
        let owned_contents = {
            let guard = shared_lock.read();
            self.stylesheet
                .contents(&guard)
                .deep_clone(&shared_lock, None, &guard)
        };
        {
            let mut guard = shared_lock.write();
            *self.stylesheet.contents.write_with(&mut guard) = owned_contents;
        }

        self.remap_rule_wrapper_leases_after_contents_clone();

        if let Some(previous_font_face_rules) = previous_font_face_rules {
            let current_font_face_rules =
                crate::style_engine::native_font_face_rules_for_stylesheet(&self.stylesheet);
            self.remap_font_face_rule_identities_after_contents_clone(
                &previous_font_face_rules,
                &current_font_face_rules,
            );
        }
    }

    fn remap_rule_wrapper_leases_after_contents_clone(&self) {
        self.remap_rule_wrapper_leases_for_current_paths();
    }

    fn remap_rule_wrapper_leases_for_current_paths(&self) {
        let mut leases = self.rule_wrapper_leases.borrow_mut();
        leases.retain(|_, lease| lease.strong_count() != 0);
        for lease in leases.values().filter_map(RcWeak::upgrade) {
            let path = {
                let binding = lease.borrow();
                let Some(binding) = binding.as_ref() else {
                    continue;
                };
                if binding.stylesheet_id != Some(self.id) {
                    continue;
                }
                binding.path.clone()
            };
            let rule = self.native_rule_at_path(&path);
            let mut binding = lease.borrow_mut();
            let Some(binding) = binding.as_mut() else {
                continue;
            };
            match rule {
                Some(rule) => {
                    binding.rule = rule;
                    binding.shared_lock = self.stylesheet.shared_lock.clone();
                }
                None => binding.mark_detached(),
            }
        }
    }

    fn replace_rule_wrapper_bindings_at_path(&self, replaced_path: &[usize]) {
        let replacement = self.native_rule_at_path(replaced_path);
        self.for_each_attached_rule_wrapper_binding(|binding| {
            if binding.path == replaced_path {
                if let Some(replacement) = replacement.as_ref() {
                    binding.rule = replacement.clone();
                    binding.shared_lock = self.stylesheet.shared_lock.clone();
                } else {
                    binding.mark_detached();
                }
                return;
            }
            if binding.path.starts_with(replaced_path) {
                binding.mark_detached();
            }
        });
    }

    fn refresh_rule_wrapper_bindings_at_path(&self, rule_path: &[usize]) {
        let replacement = self.native_rule_at_path(rule_path);
        self.for_each_attached_rule_wrapper_binding(|binding| {
            if binding.path != rule_path {
                return;
            }
            if let Some(replacement) = replacement.as_ref() {
                binding.rule = replacement.clone();
                binding.shared_lock = self.stylesheet.shared_lock.clone();
            } else {
                binding.mark_detached();
            }
        });
    }

    fn retire_all_rule_wrapper_bindings(&self) {
        self.for_each_attached_rule_wrapper_binding(StylesheetRuleWrapperBinding::mark_detached);
    }

    fn shift_rule_wrapper_paths_for_top_level_insert(&self, index: usize) {
        self.shift_rule_wrapper_paths_for_insert(&[], index);
    }

    fn shift_rule_wrapper_paths_for_insert(&self, parent_path: &[usize], index: usize) {
        self.for_each_attached_rule_wrapper_binding(|binding| {
            if binding.path.starts_with(parent_path)
                && binding.path.len() > parent_path.len()
                && binding.path[parent_path.len()] >= index
            {
                binding.path[parent_path.len()] = binding.path[parent_path.len()].saturating_add(1);
            }
        });
    }

    fn shift_rule_wrapper_paths_for_top_level_delete(&self, index: usize) {
        self.shift_rule_wrapper_paths_for_delete(&[], index);
    }

    fn shift_rule_wrapper_paths_for_delete(&self, parent_path: &[usize], index: usize) {
        self.for_each_attached_rule_wrapper_binding(|binding| {
            if !binding.path.starts_with(parent_path) || binding.path.len() <= parent_path.len() {
                return;
            }
            match binding.path.get(parent_path.len()).copied() {
                Some(current) if current == index => binding.mark_detached(),
                Some(current) if current > index => {
                    binding.path[parent_path.len()] = current - 1;
                }
                _ => {}
            }
        });
    }

    fn for_each_attached_rule_wrapper_binding(
        &self,
        mut f: impl FnMut(&mut StylesheetRuleWrapperBinding),
    ) {
        let mut leases = self.rule_wrapper_leases.borrow_mut();
        leases.retain(|_, lease| lease.strong_count() != 0);
        for lease in leases.values().filter_map(RcWeak::upgrade) {
            let mut binding = lease.borrow_mut();
            let Some(binding) = binding.as_mut() else {
                continue;
            };
            if binding.stylesheet_id == Some(self.id) {
                f(binding);
            }
        }
    }

    fn remap_font_face_rule_identities_after_contents_clone(
        &self,
        previous_rules: &[crate::style_engine::NativeStylesheetFontFaceRuleProjection],
        current_rules: &[crate::style_engine::NativeStylesheetFontFaceRuleProjection],
    ) {
        for identity in self.font_face_rule_identities.borrow_mut().iter_mut() {
            let Some(index) = previous_rules
                .iter()
                .position(|rule| ServoArc::ptr_eq(&identity.rule, &rule.rule))
            else {
                continue;
            };
            let Some(current_rule) = current_rules.get(index) else {
                continue;
            };
            if current_rule.rule_fingerprint == identity.fingerprint {
                identity.rule = current_rule.rule.clone();
            }
        }
    }
}

fn stylesheet_eof_open_var_declaration_override(
    stylesheet: &ServoArc<Stylesheet>,
    css_text: &str,
) -> Option<LiveStylesheetSourceDeclarationOverride> {
    let guard = stylesheet.shared_lock.read();
    let contents = stylesheet.contents.read_with(&guard);
    let rules = contents.rules.read_with(&guard);
    let mut rule_path = vec![rules.0.len().checked_sub(1)?];
    let mut rule = rules.0.last()?.clone();

    loop {
        let children = rule.children(&guard);
        let Some(child_index) = children.len().checked_sub(1) else {
            break;
        };
        rule_path.push(child_index);
        rule = children[child_index].clone();
    }

    let CssRule::Style(rule) = rule else {
        return None;
    };
    let serialized_declaration_text = {
        let rule = rule.read_with(&guard);
        serialize_declaration_block(&rule.block, &guard)
    };
    let declaration_text = moli_css_parse::recover_stylesheet_eof_open_var_declaration_text(
        css_text,
        &serialized_declaration_text,
    )?;
    Some(LiveStylesheetSourceDeclarationOverride {
        rule_path,
        declaration_text,
    })
}

mod import_graph;
mod mutation;
mod registry;
pub(crate) use registry::LiveStylesheetRegistry;
#[cfg(test)]
mod tests;

use crate::webidl;

use cssparser::{
    ParseError, Parser, ParserInput, SourcePosition, ToCss, Token, TokenSerializationType,
};
use moli_css_parse::{
    CssDeclaration, CssFontFace, DeclarationParseOptions, parse_declaration_list, parse_font_faces,
};

pub(crate) use moli_css_parse::{
    camel_case_style_property_name, canonical_style_property_identifier,
    canonical_style_property_name, escape_top_level_semicolons, serialize_style_property_name,
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleDeclaration.item")]
pub(crate) struct CssStyleDeclarationItemArgs {
    #[webidl(
        required,
        missing_message = "Failed to execute 'item' on 'CSSStyleDeclaration'"
    )]
    pub(crate) index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleDeclaration.setProperty")]
pub(crate) struct CssStyleDeclarationSetPropertyArgs {
    #[webidl(
        required,
        missing_message = "Failed to execute 'setProperty' on 'CSSStyleDeclaration'"
    )]
    pub(crate) property: String,
    #[webidl(
        required,
        missing_message = "Failed to execute 'setProperty' on 'CSSStyleDeclaration'",
        treat_null_as_empty_string
    )]
    pub(crate) value: String,
    #[webidl(default = "", treat_null_as_empty_string)]
    pub(crate) priority: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleDeclaration property")]
pub(crate) struct CssStyleDeclarationPropertyArgs {
    #[webidl(
        required,
        missing_message = "Failed to execute CSS property method on 'CSSStyleDeclaration'"
    )]
    pub(crate) property: String,
}

#[derive(Clone)]
pub(crate) struct CssStyleEntry {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) priority: bool,
}

#[derive(Clone, Default)]
pub(crate) struct CssInlineStyleDeclarationState {
    pub(crate) block: moli_css_parse::CssDeclarationBlock,
    pub(crate) entries: Vec<CssStyleEntry>,
    pub(crate) side_entries: Vec<CssStyleEntry>,
}

impl CssInlineStyleDeclarationState {
    pub(crate) fn entries(&self) -> Vec<CssStyleEntry> {
        if self.entries.iter().any(|entry| entry.name == "all") {
            return self.entries.clone();
        }
        if self.side_entries.is_empty() && self.entries.is_empty() && !self.block.is_empty() {
            return self.pdb_entries();
        }
        if self.entries.is_empty() && !self.block.is_empty() {
            let mut entries = self.pdb_entries();
            entries.extend(self.side_entries.clone());
            return entries;
        }
        if let Some(entries) =
            css_style_entries_with_pdb_block(&self.entries, &self.side_entries, &self.block)
        {
            return entries;
        }
        self.entries.clone()
    }

    pub(crate) fn property_names(&self) -> Vec<String> {
        if self.side_entries.is_empty() && self.entries.is_empty() && !self.block.is_empty() {
            return stylo_declaration_block_property_names(&self.block);
        }
        self.entries().into_iter().map(|entry| entry.name).collect()
    }

    pub(crate) fn css_text(&self) -> String {
        if self.entries.iter().any(|entry| entry.name == "all") {
            return serialize_css_style_entries(&self.entries());
        }
        if css_style_entries_have_renderer_projection(&self.entries) {
            return serialize_css_style_entries(&self.entries);
        }
        if self.side_entries.is_empty() {
            return self.block.css_text();
        }
        self.mixed_css_text()
    }

    pub(crate) fn style_resolution_text(&self) -> String {
        if self.entries.iter().any(|entry| entry.name == "all") {
            return serialize_css_style_entries(&self.entries());
        }
        if css_style_entries_have_renderer_projection(&self.entries) {
            let entries = self
                .entries
                .iter()
                .cloned()
                .map(|mut entry| {
                    if entry.value.is_empty()
                        && let Some(value) =
                            self.style_resolution_value_for_placeholder_entry(&entry.name)
                    {
                        entry.value = value;
                    }
                    entry
                })
                .collect::<Vec<_>>();
            return serialize_css_style_entries(&entries);
        }
        if self.side_entries.is_empty() {
            return self.block.css_text();
        }
        serialize_css_style_entries_with_pdb_block(&self.entries(), &self.side_entries, &self.block)
            .or_else(|| {
                serialize_css_style_entries_with_contiguous_pdb_block(
                    &self.entries(),
                    &self.side_entries,
                    &self.block,
                )
            })
            .unwrap_or_else(|| serialize_css_style_entries(&self.entries()))
    }

    pub(crate) fn refresh_pdb_entries(&mut self) {
        if self.side_entries.is_empty()
            && !self.entries.iter().any(|entry| entry.name == "all")
            && !css_style_entries_have_renderer_projection(&self.entries)
        {
            self.entries.clear();
        }
    }

    fn pdb_entries(&self) -> Vec<CssStyleEntry> {
        self.block
            .entries()
            .into_iter()
            .map(CssStyleEntry::from)
            .collect()
    }

    fn style_resolution_value_for_placeholder_entry(&self, property: &str) -> Option<String> {
        self.block
            .property_value(property)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                let (shorthand, _) = box_shorthand_from_first_longhand(property)?;
                self.block
                    .property_value(shorthand)
                    .filter(|value| !value.is_empty())
            })
    }

    fn mixed_css_text(&self) -> String {
        if css_style_entries_have_renderer_projection(&self.entries) {
            return serialize_css_style_entries(&self.entries);
        }
        serialize_css_style_entries_with_pdb_block(&self.entries(), &self.side_entries, &self.block)
            .unwrap_or_else(|| serialize_css_style_entries(&self.entries()))
    }
}

pub(crate) fn stylo_declaration_block_property_names(
    block: &moli_css_parse::CssDeclarationBlock,
) -> Vec<String> {
    (0..block.len())
        .filter_map(|index| block.item(index))
        .map(|name| canonical_style_property_name(&name))
        .collect()
}

pub(crate) fn mask_compat_property_name(property: &str) -> bool {
    matches!(
        canonical_style_property_name(property).as_str(),
        "mask"
            | "-webkit-mask"
            | "-webkit-mask-box-image"
            | "-webkit-mask-box-image-source"
            | "-webkit-mask-image"
            | "-webkit-mask-box-image-outset"
            | "-webkit-mask-box-image-slice"
            | "-webkit-mask-box-image-repeat"
            | "-webkit-mask-box-image-width"
            | "-webkit-mask-size"
            | "-webkit-mask-clip"
            | "-webkit-mask-origin"
            | "-webkit-mask-composite"
            | "-webkit-mask-position"
            | "-webkit-mask-repeat"
    )
}

/// Standard mask properties parsed and cascaded by the pinned Stylo world
/// once the same `layout.unimplemented` gate used by Blitz is enabled.
///
/// Keep the legacy `-webkit-mask-*` CSSOM surface on its existing narrow
/// compatibility path. Stylo accepts some of those names as stylesheet
/// aliases, but our CSSOM alias contract is not backed by the same declaration
/// metadata and must not be opened implicitly.
pub(crate) fn stylo_mask_property_name(property: &str) -> bool {
    matches!(
        canonical_style_property_name(property).as_str(),
        "mask"
            | "mask-image"
            | "mask-mode"
            | "mask-repeat"
            | "mask-clip"
            | "mask-origin"
            | "mask-composite"
            | "mask-position"
            | "mask-position-x"
            | "mask-position-y"
            | "mask-size"
    )
}

pub(crate) fn mask_compat_value_is_supported(property: &str, value: &str) -> bool {
    let property = canonical_style_property_name(property);
    if !mask_compat_property_name(&property) {
        return false;
    }
    let value = value.trim().to_ascii_lowercase();
    if matches!(
        value.as_str(),
        "initial" | "inherit" | "unset" | "revert" | "revert-layer" | "revert-rule"
    ) {
        return true;
    }
    match property.as_str() {
        "mask"
        | "-webkit-mask"
        | "-webkit-mask-box-image"
        | "-webkit-mask-box-image-source"
        | "-webkit-mask-image" => value == "none",
        "-webkit-mask-box-image-outset" | "-webkit-mask-box-image-slice" => value == "0",
        "-webkit-mask-box-image-repeat" => value == "stretch",
        "-webkit-mask-box-image-width" | "-webkit-mask-size" => value == "auto",
        "-webkit-mask-clip" | "-webkit-mask-origin" => value == "border-box",
        "-webkit-mask-composite" => value == "source-over",
        "-webkit-mask-position" => value == "0% 0%",
        "-webkit-mask-repeat" => value == "repeat",
        _ => false,
    }
}

pub(crate) fn webkit_transform_origin_compat_property_name(property: &str) -> bool {
    canonical_style_property_name(property) == "-webkit-transform-origin"
}

pub(crate) fn webkit_transform_origin_compat_value_is_supported(
    property: &str,
    value: &str,
) -> bool {
    if !webkit_transform_origin_compat_property_name(property) {
        return false;
    }
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if is_css_wide_keyword_value(value) {
        return true;
    }
    pdb_value_fragment_is_supported("transform-origin", value)
}

fn is_css_wide_keyword_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "initial" | "inherit" | "unset" | "revert" | "revert-layer" | "revert-rule"
    )
}

fn pdb_value_fragment_is_supported(property: &str, value: &str) -> bool {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let mut block = moli_css_parse::CssDeclarationBlock::default();
    block
        .set_property_with_projection(property, value.trim(), false)
        .set_result
        != moli_css_parse::CssSetResult::ParseError
}

fn css_style_entries_have_renderer_projection(entries: &[CssStyleEntry]) -> bool {
    entries.iter().any(|entry| {
        let name = canonical_style_property_name(&entry.name);
        !moli_css_parse::is_cssom_custom_property_name(&name)
            && (entry.value.is_empty()
                || moli_css_parse::css_value_may_contain_var_function(&entry.value)
                || moli_css_parse::css_value_may_contain_env_function(&entry.value))
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineStyleEntryKind {
    Pdb,
    Side,
}

fn style_entries_equal(left: &CssStyleEntry, right: &CssStyleEntry) -> bool {
    left.name == right.name && left.value == right.value && left.priority == right.priority
}

#[derive(Clone)]
enum OrderedCssStyleEntry {
    Pdb(CssStyleEntry),
    Side(CssStyleEntry),
}

impl OrderedCssStyleEntry {
    fn entry(&self) -> &CssStyleEntry {
        match self {
            OrderedCssStyleEntry::Pdb(entry) | OrderedCssStyleEntry::Side(entry) => entry,
        }
    }

    fn into_entry(self) -> CssStyleEntry {
        match self {
            OrderedCssStyleEntry::Pdb(entry) | OrderedCssStyleEntry::Side(entry) => entry,
        }
    }
}

struct PdbStyleEntryCandidate {
    entry_index: usize,
    normalized_entries: Vec<CssStyleEntry>,
    used: Vec<bool>,
}

pub(crate) fn serialize_css_style_entries_with_pdb_block(
    entries: &[CssStyleEntry],
    side_entries: &[CssStyleEntry],
    block: &moli_css_parse::CssDeclarationBlock,
) -> Option<String> {
    if entries.iter().any(|entry| entry.name == "all") {
        return None;
    }
    serialize_css_style_entries_with_pdb_runs(entries, side_entries, block).or_else(|| {
        serialize_css_style_entries_with_contiguous_pdb_block(entries, side_entries, block)
    })
}

fn serialize_css_style_entries_with_pdb_runs(
    entries: &[CssStyleEntry],
    side_entries: &[CssStyleEntry],
    block: &moli_css_parse::CssDeclarationBlock,
) -> Option<String> {
    let ordered_entries = ordered_css_style_entries_with_pdb_block(entries, side_entries, block)?;
    if ordered_entries_need_renderer_projection(&ordered_entries) {
        let entries = ordered_entries
            .into_iter()
            .map(OrderedCssStyleEntry::into_entry)
            .collect::<Vec<_>>();
        let css_text = serialize_css_style_entries(&entries);
        return (!css_text.is_empty()).then_some(css_text);
    }
    serialize_ordered_css_style_entries_with_pdb_runs(&ordered_entries)
}

fn ordered_entries_need_renderer_projection(entries: &[OrderedCssStyleEntry]) -> bool {
    ordered_entries_need_renderer_overflow_projection(entries)
        || ordered_entries_need_renderer_text_decoration_projection(entries)
}

fn ordered_entries_need_renderer_overflow_projection(entries: &[OrderedCssStyleEntry]) -> bool {
    let entries = entries
        .iter()
        .map(|entry| entry.entry().clone())
        .collect::<Vec<_>>();
    indexed_entry(&entries, "overflow-x")
        .zip(indexed_entry(&entries, "overflow-y"))
        .is_some_and(|((_, x), (_, y))| x.priority == y.priority)
}

fn ordered_entries_need_renderer_text_decoration_projection(
    entries: &[OrderedCssStyleEntry],
) -> bool {
    let entries = entries
        .iter()
        .map(|entry| entry.entry().clone())
        .collect::<Vec<_>>();
    [
        "text-decoration-line",
        "text-decoration-thickness",
        "text-decoration-style",
        "text-decoration-color",
    ]
    .into_iter()
    .map(|name| indexed_entry(&entries, name).map(|(_, entry)| entry.priority))
    .collect::<Option<Vec<_>>>()
    .is_some_and(|priorities| priorities.iter().all(|priority| priority == &priorities[0]))
}

fn css_style_entries_with_pdb_block(
    entries: &[CssStyleEntry],
    side_entries: &[CssStyleEntry],
    block: &moli_css_parse::CssDeclarationBlock,
) -> Option<Vec<CssStyleEntry>> {
    Some(
        ordered_css_style_entries_with_pdb_block(entries, side_entries, block)?
            .into_iter()
            .map(OrderedCssStyleEntry::into_entry)
            .collect(),
    )
}

fn ordered_css_style_entries_with_pdb_block(
    entries: &[CssStyleEntry],
    side_entries: &[CssStyleEntry],
    block: &moli_css_parse::CssDeclarationBlock,
) -> Option<Vec<OrderedCssStyleEntry>> {
    if side_entries.iter().any(|entry| entry.name == "all") {
        return None;
    }
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let block_entries = block
        .entries()
        .into_iter()
        .map(CssStyleEntry::from)
        .collect::<Vec<_>>();
    if block_entries.is_empty() {
        return None;
    }
    if entries.is_empty() {
        return Some(
            block_entries
                .into_iter()
                .map(OrderedCssStyleEntry::Pdb)
                .chain(side_entries.iter().cloned().map(OrderedCssStyleEntry::Side))
                .collect(),
        );
    }

    let mut remaining_side_entries = side_entries.to_vec();
    let mut entry_is_side = vec![false; entries.len()];
    let mut pdb_candidates = Vec::new();
    for (entry_index, entry) in entries.iter().enumerate() {
        if let Some(position) = remaining_side_entries
            .iter()
            .position(|side| style_entries_equal(side, entry))
        {
            remaining_side_entries.remove(position);
            entry_is_side[entry_index] = true;
            continue;
        }

        let normalized_entries = normalized_pdb_entries_for_style_entry(entry)?;
        if normalized_entries.is_empty() {
            return None;
        }
        let used = vec![false; normalized_entries.len()];
        pdb_candidates.push(PdbStyleEntryCandidate {
            entry_index,
            normalized_entries,
            used,
        });
    }
    if !remaining_side_entries.is_empty() {
        return None;
    }

    // Use the full PDB block to decide which ordinary declarations survive,
    // then place those active declarations back into the CSSOM side-table order.
    let mut active_pdb_entries_by_index = vec![Vec::new(); entries.len()];
    let mut pending_unanchored_pdb_entries = Vec::new();
    for block_entry in block_entries {
        let mut matched = None;
        for candidate in pdb_candidates.iter_mut().rev() {
            if let Some(normalized_index) =
                candidate.normalized_entries.iter().enumerate().rposition(
                    |(normalized_index, normalized_entry)| {
                        !candidate.used[normalized_index]
                            && style_entries_equal(normalized_entry, &block_entry)
                    },
                )
            {
                candidate.used[normalized_index] = true;
                matched = Some(candidate.entry_index);
                break;
            }
        }
        if let Some(entry_index) = matched {
            active_pdb_entries_by_index[entry_index].append(&mut pending_unanchored_pdb_entries);
            active_pdb_entries_by_index[entry_index].push(block_entry);
        } else {
            pending_unanchored_pdb_entries.push(block_entry);
        }
    }

    let mut ordered_entries = Vec::new();
    if !pending_unanchored_pdb_entries.is_empty() && pdb_candidates.is_empty() {
        ordered_entries.extend(
            pending_unanchored_pdb_entries
                .drain(..)
                .map(OrderedCssStyleEntry::Pdb),
        );
    }
    for (entry_index, entry) in entries.iter().enumerate() {
        if entry_is_side[entry_index] {
            ordered_entries.push(OrderedCssStyleEntry::Side(entry.clone()));
            continue;
        }
        ordered_entries.extend(
            active_pdb_entries_by_index[entry_index]
                .iter()
                .cloned()
                .map(OrderedCssStyleEntry::Pdb),
        );
    }
    ordered_entries.extend(
        pending_unanchored_pdb_entries
            .into_iter()
            .map(OrderedCssStyleEntry::Pdb),
    );
    Some(ordered_entries)
}

fn serialize_css_style_entries_with_contiguous_pdb_block(
    entries: &[CssStyleEntry],
    side_entries: &[CssStyleEntry],
    block: &moli_css_parse::CssDeclarationBlock,
) -> Option<String> {
    let pdb_text = block.css_text();
    if pdb_text.is_empty() {
        return None;
    }
    let mut classifications = Vec::with_capacity(entries.len());
    let mut remaining_side_entries = side_entries.to_vec();
    for entry in entries {
        if let Some(position) = remaining_side_entries
            .iter()
            .position(|side| style_entries_equal(side, entry))
        {
            remaining_side_entries.remove(position);
            classifications.push(InlineStyleEntryKind::Side);
        } else {
            classifications.push(InlineStyleEntryKind::Pdb);
        }
    }
    if !remaining_side_entries.is_empty() {
        return None;
    }
    let first_pdb = classifications
        .iter()
        .position(|kind| *kind == InlineStyleEntryKind::Pdb)?;
    let last_pdb = classifications
        .iter()
        .rposition(|kind| *kind == InlineStyleEntryKind::Pdb)?;
    if classifications[first_pdb..=last_pdb].contains(&InlineStyleEntryKind::Side) {
        return None;
    }
    let mut parts = Vec::new();
    let before = serialize_css_style_entries(&entries[..first_pdb]);
    if !before.is_empty() {
        parts.push(before);
    }
    parts.push(pdb_text);
    let after = serialize_css_style_entries(&entries[last_pdb + 1..]);
    if !after.is_empty() {
        parts.push(after);
    }
    Some(parts.join(" "))
}

fn normalized_pdb_entries_for_style_entry(entry: &CssStyleEntry) -> Option<Vec<CssStyleEntry>> {
    let mut block = moli_css_parse::CssDeclarationBlock::default();
    let value = pdb_mutation_value_for_style_entry(entry);
    let projection = block.set_property_with_projection(&entry.name, &value, entry.priority);
    if projection.set_result == moli_css_parse::CssSetResult::ParseError {
        return None;
    }
    let normalized = projection
        .entries
        .into_iter()
        .map(CssStyleEntry::from)
        .collect::<Vec<_>>();
    (!normalized.is_empty()).then_some(normalized)
}

fn pdb_mutation_value_for_style_entry(entry: &CssStyleEntry) -> std::borrow::Cow<'_, str> {
    if entry.value.is_empty()
        && moli_css_parse::is_cssom_custom_property_name(&canonical_style_property_name(
            &entry.name,
        ))
    {
        return std::borrow::Cow::Borrowed(" ");
    }
    std::borrow::Cow::Borrowed(&entry.value)
}

fn serialize_ordered_css_style_entries_with_pdb_runs(
    entries: &[OrderedCssStyleEntry],
) -> Option<String> {
    let mut parts = Vec::new();
    let mut pdb_run = Vec::new();
    let mut side_run = Vec::new();
    for entry in entries {
        match entry {
            OrderedCssStyleEntry::Pdb(entry) => {
                flush_side_run(&mut parts, &mut side_run);
                pdb_run.push(entry.clone());
            }
            OrderedCssStyleEntry::Side(entry) => {
                flush_pdb_run(&mut parts, &mut pdb_run)?;
                side_run.push(entry.clone());
            }
        }
    }
    flush_pdb_run(&mut parts, &mut pdb_run)?;
    flush_side_run(&mut parts, &mut side_run);
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn flush_pdb_run(parts: &mut Vec<String>, entries: &mut Vec<CssStyleEntry>) -> Option<()> {
    if entries.is_empty() {
        return Some(());
    }
    let mut block = moli_css_parse::CssDeclarationBlock::default();
    for entry in entries.iter() {
        let value = pdb_mutation_value_for_style_entry(entry);
        let projection = block.set_property_with_projection(&entry.name, &value, entry.priority);
        if projection.set_result == moli_css_parse::CssSetResult::ParseError {
            entries.clear();
            return None;
        }
    }
    let canonical_entries = block
        .entries()
        .into_iter()
        .map(CssStyleEntry::from)
        .collect::<Vec<_>>();
    let css_text = if pdb_run_needs_renderer_shorthand_projection(&canonical_entries) {
        serialize_css_style_entries(&canonical_entries)
    } else {
        block.css_text()
    };
    entries.clear();
    if css_text.is_empty() {
        return None;
    }
    parts.push(css_text);
    Some(())
}

fn pdb_run_needs_renderer_shorthand_projection(entries: &[CssStyleEntry]) -> bool {
    pdb_run_needs_renderer_border_side_projection(entries)
        || pdb_run_needs_renderer_text_decoration_projection(entries)
}

fn pdb_run_needs_renderer_border_side_projection(entries: &[CssStyleEntry]) -> bool {
    ["border-top", "border-right", "border-bottom", "border-left"]
        .into_iter()
        .any(|side_name| {
            let Some((_, width)) = indexed_entry(entries, &format!("{side_name}-width")) else {
                return false;
            };
            let Some((_, style)) = indexed_entry(entries, &format!("{side_name}-style")) else {
                return false;
            };
            let Some((_, color)) = indexed_entry(entries, &format!("{side_name}-color")) else {
                return false;
            };
            width.priority == style.priority && width.priority == color.priority
        })
}

fn pdb_run_needs_renderer_text_decoration_projection(entries: &[CssStyleEntry]) -> bool {
    [
        "text-decoration-line",
        "text-decoration-thickness",
        "text-decoration-style",
        "text-decoration-color",
    ]
    .into_iter()
    .map(|name| indexed_entry(entries, name).map(|(_, entry)| entry.priority))
    .collect::<Option<Vec<_>>>()
    .is_some_and(|priorities| priorities.iter().all(|priority| priority == &priorities[0]))
}

fn flush_side_run(parts: &mut Vec<String>, entries: &mut Vec<CssStyleEntry>) {
    if entries.is_empty() {
        return;
    }
    let css_text = serialize_css_style_entries(entries);
    entries.clear();
    if !css_text.is_empty() {
        parts.push(css_text);
    }
}

#[derive(Clone)]
pub(crate) struct CssFontFaceEntry {
    pub(crate) family: String,
    pub(crate) source: String,
}

impl From<CssDeclaration> for CssStyleEntry {
    fn from(declaration: CssDeclaration) -> Self {
        let value = normalize_parsed_css_style_entry_value(&declaration.name, &declaration.value);
        Self {
            name: declaration.name,
            value,
            priority: declaration.important,
        }
    }
}

impl From<moli_css_parse::CssDeclarationEntry> for CssStyleEntry {
    fn from(entry: moli_css_parse::CssDeclarationEntry) -> Self {
        Self {
            name: canonical_style_property_name(&entry.name),
            value: entry.value,
            priority: entry.priority,
        }
    }
}

fn normalize_parsed_css_style_entry_value(name: &str, value: &str) -> String {
    normalize_cssom_declaration_value(name, value).unwrap_or_else(|| value.to_owned())
}

fn normalize_cssom_declaration_value(property: &str, value: &str) -> Option<String> {
    match canonical_style_property_name(property).as_str() {
        "width"
        | "height"
        | "margin"
        | "min-width"
        | "max-width"
        | "padding"
        | "inset-inline-end"
        | "inset-inline-start"
        | "left"
        | "right"
        | "top"
        | "bottom"
        | "scroll-margin-top"
        | "scroll-padding-bottom"
        | "column-width"
        | "column-rule-width"
        | "outline"
        | "shape-margin"
            if cssom_declaration_value_is_unitless_zero(value) =>
        {
            Some("0px".to_owned())
        }
        "flex" => normalize_cssom_flex_shorthand_value(value),
        "flex-basis" => normalize_cssom_flex_basis_value(value),
        _ => None,
    }
}

fn cssom_declaration_value_is_unitless_zero(value: &str) -> bool {
    moli_css_parse::normalize_cssom_component_value_serialization(value).as_deref() == Some("0")
}

pub(crate) fn normalize_cssom_flex_basis_value(value: &str) -> Option<String> {
    let serialized = moli_css_parse::normalize_cssom_component_value_serialization(value)?;
    if serialized == "0" {
        Some("0px".to_owned())
    } else {
        Some(serialized)
    }
}

pub(crate) fn normalize_cssom_flex_shorthand_value(value: &str) -> Option<String> {
    let serialized = moli_css_parse::normalize_cssom_component_value_serialization(value)?;
    if serialized == "0" {
        Some("0px".to_owned())
    } else {
        Some(serialized)
    }
}

/// Splits top-level component values for box-like CSSOM reconstruction. Validity
/// for the shorthand or its longhands must come from Stylo/PDB.
pub(crate) fn box_shorthand_value_components(value: &str) -> Option<Vec<String>> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let mut components = Vec::new();
    let mut component_start: Option<SourcePosition> = None;
    let mut component_end: Option<SourcePosition> = None;

    loop {
        let token_start = input.position();
        let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        if matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
            push_box_shorthand_component(
                &input,
                &mut components,
                &mut component_start,
                component_end,
            )?;
            component_end = None;
            continue;
        }

        component_start.get_or_insert(token_start);
        if css_component_token_starts_nested_block(&token) {
            let nested: Result<(), ParseError<'_, ()>> = input.parse_nested_block(|input| {
                consume_nested_css_component(input)
                    .ok_or_else(|| input.new_custom_error::<(), ()>(()))
            });
            nested.ok()?;
        }
        component_end = Some(input.position());
    }

    push_box_shorthand_component(&input, &mut components, &mut component_start, component_end)?;
    (!components.is_empty()).then_some(components)
}

/// Splits only top-level comma layers while preserving nested component text.
/// The caller is responsible for using this only after grammar validation.
pub(crate) fn top_level_comma_separated_component_values(value: &str) -> Option<Vec<String>> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let mut layers = Vec::new();
    let mut layer_start: Option<SourcePosition> = None;
    let mut layer_end: Option<SourcePosition> = None;

    loop {
        let token_start = input.position();
        let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => continue,
            Token::Comma => {
                push_top_level_comma_component(&input, &mut layers, &mut layer_start, layer_end)?;
                layer_end = None;
                continue;
            }
            _ => {}
        }

        layer_start.get_or_insert(token_start);
        if css_component_token_starts_nested_block(&token) {
            let nested: Result<(), ParseError<'_, ()>> = input.parse_nested_block(|input| {
                consume_nested_css_component(input)
                    .ok_or_else(|| input.new_custom_error::<(), ()>(()))
            });
            nested.ok()?;
        }
        layer_end = Some(input.position());
    }

    push_top_level_comma_component(&input, &mut layers, &mut layer_start, layer_end)?;
    Some(layers)
}

pub(crate) fn normalize_cssom_component_value_serialization_with_spaced_slash(
    value: &str,
) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let mut parts = Vec::new();
    let mut part_start: Option<SourcePosition> = None;
    let mut part_end: Option<SourcePosition> = None;
    let mut saw_top_level_slash = false;

    loop {
        let token_start = input.position();
        let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        if matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
            continue;
        }
        if matches!(token, Token::Delim('/')) {
            saw_top_level_slash = true;
            push_top_level_slash_component(&input, &mut parts, &mut part_start, part_end)?;
            part_end = None;
            continue;
        }

        part_start.get_or_insert(token_start);
        if css_component_token_starts_nested_block(&token) {
            let nested: Result<(), ParseError<'_, ()>> = input.parse_nested_block(|input| {
                consume_nested_css_component(input)
                    .ok_or_else(|| input.new_custom_error::<(), ()>(()))
            });
            nested.ok()?;
        }
        part_end = Some(input.position());
    }

    if !saw_top_level_slash {
        return moli_css_parse::normalize_cssom_component_value_serialization(value);
    }
    push_top_level_slash_component(&input, &mut parts, &mut part_start, part_end)?;
    Some(parts.join(" / ").trim().to_owned())
}

fn push_top_level_slash_component(
    input: &Parser<'_, '_>,
    parts: &mut Vec<String>,
    part_start: &mut Option<SourcePosition>,
    part_end: Option<SourcePosition>,
) -> Option<()> {
    let Some(start) = part_start.take() else {
        parts.push(String::new());
        return Some(());
    };
    let raw = input.slice(start..part_end?);
    parts.push(moli_css_parse::normalize_cssom_component_value_serialization(raw)?);
    Some(())
}

fn push_top_level_comma_component(
    input: &Parser<'_, '_>,
    layers: &mut Vec<String>,
    layer_start: &mut Option<SourcePosition>,
    layer_end: Option<SourcePosition>,
) -> Option<()> {
    let start = layer_start.take()?;
    let raw = input.slice(start..layer_end?);
    layers.push(moli_css_parse::normalize_cssom_component_value_serialization(raw)?);
    Some(())
}

fn push_box_shorthand_component(
    input: &Parser<'_, '_>,
    components: &mut Vec<String>,
    component_start: &mut Option<SourcePosition>,
    component_end: Option<SourcePosition>,
) -> Option<()> {
    let Some(start) = component_start.take() else {
        return Some(());
    };
    let raw = input.slice(start..component_end?);
    components.push(moli_css_parse::normalize_cssom_component_value_serialization(raw)?);
    Some(())
}

fn css_component_token_starts_nested_block(token: &Token<'_>) -> bool {
    matches!(
        token,
        Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock
    )
}

fn consume_nested_css_component(input: &mut Parser<'_, '_>) -> Option<()> {
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        if css_component_token_starts_nested_block(&token) {
            let nested: Result<(), ParseError<'_, ()>> = input.parse_nested_block(|input| {
                consume_nested_css_component(input)
                    .ok_or_else(|| input.new_custom_error::<(), ()>(()))
            });
            nested.ok()?;
        }
    }
    Some(())
}

pub(crate) fn background_shorthand_color(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::Ident(value) if ident_is_background_color_keyword(&value) => {
                return Some(value.to_string());
            }
            Token::Hash(value) | Token::IDHash(value) => {
                return Some(format!("#{value}"));
            }
            Token::Function(name) if function_is_color(&name) => {
                return shorthand_component_function_text(&mut input, &name);
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn border_shorthand_width(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::Ident(value)
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    "thin" | "medium" | "thick"
                ) =>
            {
                return Some(value.to_ascii_lowercase());
            }
            Token::Dimension { value, .. } if value >= 0.0 => {
                let mut css_text = String::new();
                token.to_css(&mut css_text).ok()?;
                return Some(css_text);
            }
            Token::Function(name)
                if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "calc" | "min" | "max" | "clamp"
                ) =>
            {
                return shorthand_component_function_text(&mut input, &name);
            }
            Token::Number { value: 0.0, .. } => {
                return Some("0px".to_owned());
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn border_shorthand_color(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::Ident(value)
                if ident_is_background_color_keyword(&value) || ident_is_system_color(&value) =>
            {
                return Some(value.to_string());
            }
            Token::Hash(value) | Token::IDHash(value) => {
                return Some(format!("#{value}"));
            }
            Token::Function(name) if function_is_color(&name) => {
                return shorthand_component_function_text(&mut input, &name);
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn border_shorthand_style(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::Ident(value)
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    "none"
                        | "hidden"
                        | "dotted"
                        | "dashed"
                        | "solid"
                        | "double"
                        | "groove"
                        | "ridge"
                        | "inset"
                        | "outset"
                ) =>
            {
                return Some(value.to_ascii_lowercase());
            }
            _ => {}
        }
    }
    None
}

fn ident_is_background_color_keyword(value: &str) -> bool {
    value.eq_ignore_ascii_case("transparent")
        || value.eq_ignore_ascii_case("currentcolor")
        || cssparser::color::parse_named_color(value).is_ok()
}

fn function_is_color(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklab" | "oklch" | "color"
    )
}

pub(crate) fn ident_is_system_color(value: &str) -> bool {
    moli_css_parse::css_system_color_srgb(value).is_some()
}

pub(crate) fn system_color_rgb(value: &str) -> Option<(u8, u8, u8)> {
    moli_css_parse::css_system_color_srgb(value)
}

fn shorthand_component_function_text(input: &mut Parser<'_, '_>, name: &str) -> Option<String> {
    let mut css_text = String::new();
    css_text.push_str(name);
    css_text.push('(');
    input
        .parse_nested_block(|nested| {
            serialize_shorthand_component_values(
                nested,
                &mut css_text,
                TokenSerializationType::Nothing,
            )
            .map(|_| ())
            .ok_or_else(|| nested.new_custom_error::<(), ()>(()))
        })
        .ok()?;
    css_text.push(')');
    Some(css_text)
}

fn serialize_shorthand_component_values<'i, 't>(
    input: &mut Parser<'i, 't>,
    css_text: &mut String,
    mut previous_token: TokenSerializationType,
) -> Option<TokenSerializationType> {
    let mut pending_whitespace = false;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        if matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
            pending_whitespace = true;
            continue;
        }

        let token_type = token.serialization_type();
        if pending_whitespace {
            if !css_text.ends_with([' ', '{', '(', '[']) {
                css_text.push(' ');
            }
        } else if previous_token.needs_separator_when_before(token_type) {
            css_text.push_str("/**/");
        }
        pending_whitespace = false;
        previous_token = token_type;
        token.to_css(css_text).ok()?;
        let closing_token = match token {
            Token::Function(_) | Token::ParenthesisBlock => Some(Token::CloseParenthesis),
            Token::SquareBracketBlock => Some(Token::CloseSquareBracket),
            Token::CurlyBracketBlock => Some(Token::CloseCurlyBracket),
            _ => None,
        };
        if let Some(closing_token) = closing_token {
            let nested: Result<TokenSerializationType, ParseError<'_, ()>> = input
                .parse_nested_block(|input| {
                    serialize_shorthand_component_values(input, css_text, previous_token)
                        .ok_or_else(|| input.new_custom_error::<(), ()>(()))
                });
            nested.ok()?;
            closing_token.to_css(css_text).ok()?;
            previous_token = closing_token.serialization_type();
        }
    }
    Some(previous_token)
}

impl From<CssFontFace> for CssFontFaceEntry {
    fn from(face: CssFontFace) -> Self {
        Self {
            family: face.family,
            source: face.source,
        }
    }
}

pub(crate) fn parse_css_declaration_list(style_text: &str) -> Vec<CssStyleEntry> {
    parse_declaration_list(
        style_text,
        DeclarationParseOptions {
            canonicalize_property_name: false,
            unescape_value_semicolons: true,
            preserve_empty_values: true,
        },
    )
    .into_iter()
    .map(CssStyleEntry::from)
    .collect()
}

/// Returns the winning declaration for one property inside a declaration
/// list. This is also usable for valid properties that the Servo flavor of
/// Stylo does not expose, such as `content-visibility`.
pub(crate) fn css_declaration_list_property_value(
    style_text: &str,
    property: &str,
) -> Option<String> {
    let property = canonical_style_property_name(property);
    let mut winner: Option<CssStyleEntry> = None;
    for entry in parse_css_declaration_list(style_text) {
        if canonical_style_property_name(&entry.name) != property {
            continue;
        }
        if winner
            .as_ref()
            .is_some_and(|current| current.priority && !entry.priority)
        {
            continue;
        }
        winner = Some(entry);
    }
    winner.map(|entry| entry.value)
}

pub(crate) fn resolve_css_url_function(value: &str, base_url: &url::Url) -> String {
    let value = value.trim();
    let Some(raw_url) = value
        .strip_prefix("url(")
        .and_then(|value| value.strip_suffix(')'))
        .map(str::trim)
    else {
        return value.to_owned();
    };
    let raw_url = raw_url
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            raw_url
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(raw_url);
    base_url
        .join(raw_url)
        .or_else(|_| url::Url::parse(raw_url))
        .map(|url| format!("url(\"{}\")", url.as_str()))
        .unwrap_or_else(|_| value.to_owned())
}

pub(crate) fn parse_css_font_faces(css_text: &str) -> Vec<CssFontFaceEntry> {
    parse_font_faces(css_text)
        .into_iter()
        .map(CssFontFaceEntry::from)
        .collect()
}

pub(crate) fn serialize_css_style_entries(entries: &[CssStyleEntry]) -> String {
    let entries = remove_overwritten_style_entries(entries);
    let entries = serialize_all_shorthand_entries(&entries);
    let entries = serialize_background_shorthand_entries(&entries);
    let entries = serialize_overflow_shorthand_entries(&entries);
    let entries = serialize_flex_shorthand_entries(&entries);
    let entries = serialize_border_shorthand_entries(&entries);
    let entries = serialize_text_decoration_shorthand_entries(&entries);
    let entries = serialize_outline_shorthand_entries(&entries);
    let entries = serialize_list_style_shorthand_entries(&entries);
    let entries = serialize_box_shorthand_entries(&entries);
    entries
        .iter()
        .map(|entry| {
            let name = serialize_style_property_name(&entry.name);
            let value = escape_top_level_semicolons(&entry.value);
            if entry.priority {
                format!("{name}: {value} !important;")
            } else {
                format!("{name}: {value};")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn all_shorthand_applies_to(property: &str) -> bool {
    !property.starts_with("--") && !matches!(property, "all" | "direction" | "unicode-bidi")
}

fn remove_overwritten_style_entries(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    entries
        .iter()
        .enumerate()
        .filter(|(index, entry)| {
            !entries
                .iter()
                .skip(index + 1)
                .any(|later| later.name == entry.name && (!entry.priority || later.priority))
        })
        .map(|(_, entry)| entry.clone())
        .collect()
}

fn serialize_all_shorthand_entries(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    let mut serialized = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        if all_shorthand_applies_to(&entry.name) {
            let overwritten_by_later = entries.iter().skip(index + 1).any(|later| {
                (later.name == "all" || later.name == entry.name)
                    && (!entry.priority || later.priority)
            });
            if overwritten_by_later {
                continue;
            }
            let redundant_after_all = entries[..index].iter().rev().any(|earlier| {
                earlier.name == "all"
                    && earlier.priority == entry.priority
                    && earlier.value == entry.value
            });
            if redundant_after_all {
                continue;
            }
        }
        if entry.name == "all"
            && entries
                .iter()
                .skip(index + 1)
                .any(|later| later.name == "all" && (!entry.priority || later.priority))
        {
            continue;
        }
        serialized.push(entry.clone());
    }
    serialized
}

fn serialize_background_shorthand_entries(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    let has_important_background = entries
        .iter()
        .any(|entry| entry.name == "background" && entry.priority);
    if !has_important_background {
        return entries.to_vec();
    }
    entries
        .iter()
        .filter(|entry| entry.priority || !entry.name.starts_with("background-"))
        .cloned()
        .collect()
}

fn serialize_overflow_shorthand_entries(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    let mut serialized = Vec::with_capacity(entries.len());
    let mut consumed = vec![false; entries.len()];
    for (index, entry) in entries.iter().enumerate() {
        if consumed[index] {
            continue;
        }
        if matches!(entry.name.as_str(), "overflow-x" | "overflow-y")
            && let Some((replacement, indexes)) = collect_overflow_shorthand_entry(entries, index)
        {
            for index in indexes {
                consumed[index] = true;
            }
            serialized.push(replacement);
            continue;
        }
        serialized.push(entry.clone());
    }
    serialized
}

fn serialize_flex_shorthand_entries(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    let mut serialized = Vec::with_capacity(entries.len());
    let mut consumed = vec![false; entries.len()];
    for (index, entry) in entries.iter().enumerate() {
        if consumed[index] {
            continue;
        }
        if entry.name == "flex"
            && let Some((replacement, indexes)) = collect_flex_shorthand_entries(entries, index)
        {
            for index in indexes {
                consumed[index] = true;
            }
            serialized.extend(replacement);
            continue;
        }
        serialized.push(entry.clone());
    }
    serialized
}

fn collect_flex_shorthand_entries(
    entries: &[CssStyleEntry],
    flex_index: usize,
) -> Option<(Vec<CssStyleEntry>, Vec<usize>)> {
    let flex = &entries[flex_index];
    let flex_longhands = flex_css_wide_longhands(flex)?;
    let later = overriding_later_flex_longhands(entries, flex_index, flex);
    if later.is_empty() {
        return None;
    }
    if later.iter().all(|(index, entry)| {
        flex_longhands
            .iter()
            .any(|longhand| longhand.name == entry.name && longhand.value == entry.value)
            && *index > flex_index
    }) {
        return Some((
            vec![flex.clone()],
            later.into_iter().map(|(index, _)| index).collect(),
        ));
    }

    let later_names = later
        .iter()
        .map(|(_, entry)| entry.name.as_str())
        .collect::<Vec<_>>();
    let replacement = flex_longhands
        .into_iter()
        .filter(|entry| !later_names.contains(&entry.name.as_str()))
        .collect::<Vec<_>>();
    Some((replacement, vec![flex_index]))
}

fn flex_css_wide_longhands(flex: &CssStyleEntry) -> Option<Vec<CssStyleEntry>> {
    let keyword = css_wide_keyword(&flex.value)?;
    Some(
        ["flex-grow", "flex-basis", "flex-shrink"]
            .into_iter()
            .map(|name| CssStyleEntry {
                name: name.to_owned(),
                value: keyword.clone(),
                priority: flex.priority,
            })
            .collect(),
    )
}

fn overriding_later_flex_longhands<'a>(
    entries: &'a [CssStyleEntry],
    flex_index: usize,
    flex: &CssStyleEntry,
) -> Vec<(usize, &'a CssStyleEntry)> {
    entries
        .iter()
        .enumerate()
        .skip(flex_index + 1)
        .filter(|(_, entry)| {
            matches!(
                entry.name.as_str(),
                "flex-grow" | "flex-basis" | "flex-shrink"
            ) && (!flex.priority || entry.priority)
        })
        .collect()
}

fn css_wide_keyword(value: &str) -> Option<String> {
    let lowered = value.trim().to_ascii_lowercase();
    matches!(
        lowered.as_str(),
        "initial" | "inherit" | "unset" | "revert" | "revert-layer" | "revert-rule"
    )
    .then_some(lowered)
}

fn collect_overflow_shorthand_entry(
    entries: &[CssStyleEntry],
    first_index: usize,
) -> Option<(CssStyleEntry, Vec<usize>)> {
    let (x_index, x_entry) = entries
        .iter()
        .enumerate()
        .find(|(_, entry)| entry.name == "overflow-x")?;
    let (y_index, y_entry) = entries
        .iter()
        .enumerate()
        .find(|(_, entry)| entry.name == "overflow-y")?;
    if x_index < first_index || y_index < first_index || x_entry.priority != y_entry.priority {
        return None;
    }
    let value = if x_entry.value == y_entry.value {
        x_entry.value.clone()
    } else {
        format!("{} {}", x_entry.value, y_entry.value)
    };
    Some((
        CssStyleEntry {
            name: "overflow".to_owned(),
            value,
            priority: x_entry.priority,
        },
        vec![x_index, y_index],
    ))
}

fn serialize_box_shorthand_entries(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    let entries = expand_var_box_shorthands_with_later_overrides(entries);
    let mut serialized = Vec::with_capacity(entries.len());
    let mut consumed = vec![false; entries.len()];
    for (index, entry) in entries.iter().enumerate() {
        if consumed[index] {
            continue;
        }
        if let Some((shorthand, longhands)) = box_shorthand_from_first_longhand(&entry.name)
            && let Some((replacement, indexes)) =
                collect_box_shorthand_entry(&entries, index, shorthand, longhands)
        {
            for index in indexes {
                consumed[index] = true;
            }
            serialized.push(replacement);
            continue;
        }
        serialized.push(entry.clone());
    }
    serialized
}

fn expand_var_box_shorthands_with_later_overrides(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    let mut expanded = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let Some((_, longhands)) = box_shorthand_from_first_longhand_for_shorthand(&entry.name)
        else {
            expanded.push(entry.clone());
            continue;
        };
        if !moli_css_parse::css_value_may_contain_var_function(&entry.value) {
            expanded.push(entry.clone());
            continue;
        }
        let overridden = later_overriding_box_longhands(entries, index, entry, longhands);
        if overridden.is_empty() {
            expanded.push(entry.clone());
            continue;
        }
        for longhand in longhands {
            if !overridden.contains(longhand) {
                expanded.push(CssStyleEntry {
                    name: (*longhand).to_owned(),
                    value: String::new(),
                    priority: entry.priority,
                });
            }
        }
    }
    expanded
}

fn box_shorthand_from_first_longhand_for_shorthand(
    property: &str,
) -> Option<(&'static str, &'static [&'static str])> {
    match property {
        "margin" => Some((
            "margin",
            &["margin-top", "margin-right", "margin-bottom", "margin-left"],
        )),
        "margin-inline" => Some((
            "margin-inline",
            &["margin-inline-start", "margin-inline-end"],
        )),
        "margin-block" => Some(("margin-block", &["margin-block-start", "margin-block-end"])),
        "padding" => Some((
            "padding",
            &[
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
            ],
        )),
        "border-width" => Some((
            "border-width",
            &[
                "border-top-width",
                "border-right-width",
                "border-bottom-width",
                "border-left-width",
            ],
        )),
        "border-style" => Some((
            "border-style",
            &[
                "border-top-style",
                "border-right-style",
                "border-bottom-style",
                "border-left-style",
            ],
        )),
        "border-color" => Some((
            "border-color",
            &[
                "border-top-color",
                "border-right-color",
                "border-bottom-color",
                "border-left-color",
            ],
        )),
        "overscroll-behavior" => Some((
            "overscroll-behavior",
            &["overscroll-behavior-x", "overscroll-behavior-y"],
        )),
        _ => None,
    }
}

fn later_overriding_box_longhands<'a>(
    entries: &'a [CssStyleEntry],
    shorthand_index: usize,
    _shorthand: &CssStyleEntry,
    longhands: &'static [&'static str],
) -> Vec<&'a str> {
    entries
        .iter()
        .skip(shorthand_index + 1)
        .filter(|entry| longhands.contains(&entry.name.as_str()))
        .map(|entry| entry.name.as_str())
        .collect()
}

fn box_shorthand_from_first_longhand(
    property: &str,
) -> Option<(&'static str, &'static [&'static str])> {
    match property {
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => Some((
            "margin",
            &["margin-top", "margin-right", "margin-bottom", "margin-left"],
        )),
        "margin-inline-start" | "margin-inline-end" => Some((
            "margin-inline",
            &["margin-inline-start", "margin-inline-end"],
        )),
        "margin-block-start" | "margin-block-end" => {
            Some(("margin-block", &["margin-block-start", "margin-block-end"]))
        }
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => Some((
            "padding",
            &[
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
            ],
        )),
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            Some((
                "border-width",
                &[
                    "border-top-width",
                    "border-right-width",
                    "border-bottom-width",
                    "border-left-width",
                ],
            ))
        }
        "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style" => {
            Some((
                "border-style",
                &[
                    "border-top-style",
                    "border-right-style",
                    "border-bottom-style",
                    "border-left-style",
                ],
            ))
        }
        "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            Some((
                "border-color",
                &[
                    "border-top-color",
                    "border-right-color",
                    "border-bottom-color",
                    "border-left-color",
                ],
            ))
        }
        "overscroll-behavior-x" | "overscroll-behavior-y" => Some((
            "overscroll-behavior",
            &["overscroll-behavior-x", "overscroll-behavior-y"],
        )),
        _ => None,
    }
}

fn collect_box_shorthand_entry(
    entries: &[CssStyleEntry],
    first_index: usize,
    shorthand: &str,
    longhands: &[&str],
) -> Option<(CssStyleEntry, Vec<usize>)> {
    let mut indexes = Vec::with_capacity(longhands.len());
    let mut values = Vec::with_capacity(longhands.len());
    let mut priority = None;
    for longhand in longhands {
        let (index, entry) = entries
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.name == *longhand)?;
        if index < first_index {
            return None;
        }
        if priority.is_some_and(|current| current != entry.priority) {
            return None;
        }
        priority = Some(entry.priority);
        indexes.push(index);
        values.push(entry.value.clone());
    }
    if values.iter().any(|value| value.is_empty()) {
        return None;
    }
    let min_index = indexes.iter().copied().min()?;
    let max_index = indexes.iter().copied().max()?;
    if entries[min_index..=max_index]
        .iter()
        .any(|entry| !longhands.contains(&entry.name.as_str()))
    {
        return None;
    }
    Some((
        CssStyleEntry {
            name: shorthand.to_owned(),
            value: compress_box_components(&values)?,
            priority: priority.unwrap_or(false),
        },
        indexes,
    ))
}

fn compress_box_components(values: &[String]) -> Option<String> {
    match values {
        [start, end] if start == end => Some(start.clone()),
        [start, end] => Some(format!("{start} {end}")),
        [top, right, bottom, left] if top == right && top == bottom && top == left => {
            Some(top.clone())
        }
        [top, right, bottom, left] if top == bottom && right == left => {
            Some(format!("{top} {right}"))
        }
        [top, right, bottom, left] if right == left => Some(format!("{top} {right} {bottom}")),
        [top, right, bottom, left] => Some(format!("{top} {right} {bottom} {left}")),
        _ => None,
    }
}

#[derive(Clone, PartialEq)]
struct BorderComponents {
    width: String,
    style: String,
    color: String,
}

fn serialize_border_shorthand_entries(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    if let Some(serialized) = serialize_border_with_later_override(entries) {
        return serialized;
    }
    if let Some(serialized) = serialize_border_mixed_component_entries(entries) {
        return serialized;
    }
    if let Some(serialized) = serialize_border_component_entries(entries) {
        return serialized;
    }
    if let Some(serialized) = serialize_border_single_side_component_entries(entries) {
        return serialized;
    }
    if let Some(serialized) = serialize_border_side_entries(entries) {
        return serialized;
    }
    if let Some(serialized) = serialize_border_edge_component_entries(entries) {
        return serialized;
    }
    entries.to_vec()
}

fn serialize_border_with_later_override(entries: &[CssStyleEntry]) -> Option<Vec<CssStyleEntry>> {
    let (border_index, border) = entries
        .iter()
        .enumerate()
        .find(|(_, entry)| entry.name == "border")?;
    let base = border_components(&border.value);
    if let Some((style_index, style)) = entries
        .iter()
        .enumerate()
        .skip(border_index + 1)
        .find(|(_, entry)| entry.name == "border-style" && entry.priority == border.priority)
    {
        return Some(replace_indexes(
            entries,
            &[(
                border_index,
                CssStyleEntry {
                    name: "border".to_owned(),
                    value: serialize_border_side_value(&BorderComponents {
                        width: base.width,
                        style: style.value.clone(),
                        color: base.color,
                    }),
                    priority: border.priority,
                },
            )],
            &[border_index, style_index],
        ));
    }
    let style_longhands = [
        "border-top-style",
        "border-right-style",
        "border-bottom-style",
        "border-left-style",
    ];
    let mut style_indexes = Vec::with_capacity(4);
    let mut style_values = Vec::with_capacity(4);
    for longhand in style_longhands {
        let Some((index, entry)) = entries
            .iter()
            .enumerate()
            .skip(border_index + 1)
            .find(|(_, entry)| entry.name == longhand && entry.priority == border.priority)
        else {
            style_indexes.clear();
            break;
        };
        style_indexes.push(index);
        style_values.push(entry.value.clone());
    }
    if style_indexes.len() == 4 && style_values.iter().all(|value| value == &style_values[0]) {
        return Some(replace_indexes(
            entries,
            &[(
                border_index,
                CssStyleEntry {
                    name: "border".to_owned(),
                    value: serialize_border_side_value(&BorderComponents {
                        width: base.width,
                        style: style_values[0].clone(),
                        color: base.color,
                    }),
                    priority: border.priority,
                },
            )],
            &std::iter::once(border_index)
                .chain(style_indexes)
                .collect::<Vec<_>>(),
        ));
    }
    if let Some((side_index, side)) = entries
        .iter()
        .enumerate()
        .skip(border_index + 1)
        .find(|(_, entry)| entry.name == "border-top")
    {
        let side_components = border_components(&side.value);
        if side.priority == border.priority && side_components == base {
            return Some(replace_indexes(entries, &[], &[side_index]));
        }
        if side.priority && !border.priority {
            return Some(border_with_important_top_override(
                entries,
                border_index,
                side_index,
                border,
                side,
            ));
        }
        if side.priority == border.priority {
            return Some(border_with_side_components(
                entries,
                border_index,
                side_index,
                border,
                [side_components, base.clone(), base.clone(), base],
                &[],
            ));
        }
    }
    if let Some((color_index, color)) = entries
        .iter()
        .enumerate()
        .skip(border_index + 1)
        .find(|(_, entry)| entry.name == "border-top-color" && entry.priority == border.priority)
    {
        let mut top = base.clone();
        top.color = color.value.clone();
        return Some(border_with_side_components(
            entries,
            border_index,
            color_index,
            border,
            [top, base.clone(), base.clone(), base],
            &[],
        ));
    }
    None
}

fn border_with_important_top_override(
    entries: &[CssStyleEntry],
    border_index: usize,
    side_index: usize,
    border: &CssStyleEntry,
    side: &CssStyleEntry,
) -> Vec<CssStyleEntry> {
    let side_value = serialize_border_side_value(&border_components(&border.value));
    let replacements = [
        (
            border_index,
            CssStyleEntry {
                name: "border-right".to_owned(),
                value: side_value.clone(),
                priority: border.priority,
            },
        ),
        (
            border_index,
            CssStyleEntry {
                name: "border-bottom".to_owned(),
                value: side_value.clone(),
                priority: border.priority,
            },
        ),
        (
            border_index,
            CssStyleEntry {
                name: "border-left".to_owned(),
                value: side_value,
                priority: border.priority,
            },
        ),
        (
            border_index,
            CssStyleEntry {
                name: "border-image".to_owned(),
                value: "none".to_owned(),
                priority: border.priority,
            },
        ),
    ];
    replace_indexes(entries, &replacements, &[border_index, side_index])
        .into_iter()
        .chain(std::iter::once(side.clone()))
        .collect()
}

fn border_with_side_components(
    entries: &[CssStyleEntry],
    border_index: usize,
    override_index: usize,
    border: &CssStyleEntry,
    sides: [BorderComponents; 4],
    extra_consumed: &[usize],
) -> Vec<CssStyleEntry> {
    let replacements = border_component_entries(&sides, border.priority)
        .into_iter()
        .chain(std::iter::once(CssStyleEntry {
            name: "border-image".to_owned(),
            value: "none".to_owned(),
            priority: border.priority,
        }))
        .map(|entry| (border_index, entry))
        .collect::<Vec<_>>();
    let consumed = std::iter::once(border_index)
        .chain(std::iter::once(override_index))
        .chain(extra_consumed.iter().copied())
        .collect::<Vec<_>>();
    replace_indexes(entries, &replacements, &consumed)
}

fn serialize_border_mixed_component_entries(
    entries: &[CssStyleEntry],
) -> Option<Vec<CssStyleEntry>> {
    let (width_index, width) = indexed_entry(entries, "border-width")?;
    let widths = box_components(&width.value)?;
    let side_names = ["border-top", "border-right", "border-bottom", "border-left"];
    let mut indexes = vec![width_index];
    let mut sides = Vec::with_capacity(4);
    let mut priority = Some(width.priority);
    for (side_name, width) in side_names.into_iter().zip(widths) {
        let (style_index, style) = indexed_entry(entries, &format!("{side_name}-style"))?;
        let (color_index, color) = indexed_entry(entries, &format!("{side_name}-color"))?;
        for entry in [style, color] {
            if priority.is_some_and(|current| current != entry.priority) {
                return None;
            }
            priority = Some(entry.priority);
        }
        indexes.extend([style_index, color_index]);
        sides.push(BorderComponents {
            width,
            style: style.value.clone(),
            color: color.value.clone(),
        });
    }
    serialize_border_components_from_indexes(entries, indexes, sides, priority)
}

fn serialize_border_component_entries(entries: &[CssStyleEntry]) -> Option<Vec<CssStyleEntry>> {
    let side_names = ["border-top", "border-right", "border-bottom", "border-left"];
    let mut indexes = Vec::with_capacity(12);
    let mut sides = Vec::with_capacity(4);
    let mut priority = None;
    for side_name in side_names {
        let (width_index, width) = indexed_entry(entries, &format!("{side_name}-width"))?;
        let (style_index, style) = indexed_entry(entries, &format!("{side_name}-style"))?;
        let (color_index, color) = indexed_entry(entries, &format!("{side_name}-color"))?;
        for entry in [width, style, color] {
            if priority.is_some_and(|current| current != entry.priority) {
                return None;
            }
            priority = Some(entry.priority);
        }
        indexes.extend([width_index, style_index, color_index]);
        sides.push(BorderComponents {
            width: width.value.clone(),
            style: style.value.clone(),
            color: color.value.clone(),
        });
    }
    serialize_border_components_from_indexes(entries, indexes, sides, priority)
}

fn serialize_border_single_side_component_entries(
    entries: &[CssStyleEntry],
) -> Option<Vec<CssStyleEntry>> {
    for side_name in ["border-top", "border-right", "border-bottom", "border-left"] {
        let width_name = format!("{side_name}-width");
        let style_name = format!("{side_name}-style");
        let color_name = format!("{side_name}-color");
        let Some((width_index, width)) = indexed_entry(entries, &width_name) else {
            continue;
        };
        let Some((style_index, style)) = indexed_entry(entries, &style_name) else {
            continue;
        };
        let Some((color_index, color)) = indexed_entry(entries, &color_name) else {
            continue;
        };
        if width.priority != style.priority || width.priority != color.priority {
            continue;
        }
        let indexes = [width_index, style_index, color_index];
        let min_index = indexes.iter().copied().min()?;
        let max_index = indexes.iter().copied().max()?;
        let longhands = [width_name, style_name, color_name];
        if entries[min_index..=max_index]
            .iter()
            .any(|entry| !longhands.iter().any(|longhand| longhand == &entry.name))
        {
            continue;
        }
        let replacement = CssStyleEntry {
            name: side_name.to_owned(),
            value: serialize_border_side_value(&BorderComponents {
                width: width.value.clone(),
                style: style.value.clone(),
                color: color.value.clone(),
            }),
            priority: width.priority,
        };
        return Some(replace_indexes(
            entries,
            &[(min_index, replacement)],
            &indexes,
        ));
    }
    None
}

fn serialize_border_components_from_indexes(
    entries: &[CssStyleEntry],
    indexes: Vec<usize>,
    sides: Vec<BorderComponents>,
    priority: Option<bool>,
) -> Option<Vec<CssStyleEntry>> {
    let min_index = indexes.iter().copied().min()?;
    let max_index = indexes.iter().copied().max()?;
    let border_image_index = entries
        .iter()
        .enumerate()
        .find(|(_, entry)| {
            entry.name == "border-image"
                && entry.value.eq_ignore_ascii_case("none")
                && Some(entry.priority) == priority
        })
        .map(|(index, _)| index);
    if entries[min_index..=max_index]
        .iter()
        .any(|entry| !border_component_name(&entry.name))
    {
        return None;
    }
    let priority = priority.unwrap_or(false);
    let sides: [BorderComponents; 4] = sides.try_into().ok()?;
    if sides.iter().all(|side| side == &sides[0])
        && border_image_index.is_some()
        && is_standalone_uniform_border_components(entries, &indexes, border_image_index)
    {
        return Some(replace_indexes(
            entries,
            &[(
                min_index,
                CssStyleEntry {
                    name: "border".to_owned(),
                    value: serialize_border_side_value(&sides[0]),
                    priority,
                },
            )],
            &indexes
                .into_iter()
                .chain(border_image_index)
                .collect::<Vec<_>>(),
        ));
    }
    let replacements = border_component_entries(&sides, priority)
        .into_iter()
        .chain(border_image_index.map(|_| CssStyleEntry {
            name: "border-image".to_owned(),
            value: "none".to_owned(),
            priority,
        }))
        .map(|entry| (min_index, entry))
        .collect::<Vec<_>>();
    Some(replace_indexes(
        entries,
        &replacements,
        &indexes
            .into_iter()
            .chain(border_image_index)
            .collect::<Vec<_>>(),
    ))
}

fn is_standalone_uniform_border_components(
    entries: &[CssStyleEntry],
    indexes: &[usize],
    border_image_index: Option<usize>,
) -> bool {
    let Some(border_image_index) = border_image_index else {
        return true;
    };
    let Some(min_index) = indexes.iter().copied().min() else {
        return false;
    };
    let max_index = indexes
        .iter()
        .copied()
        .chain(Some(border_image_index))
        .max()
        .unwrap_or(min_index);
    entries[min_index..=max_index]
        .iter()
        .all(|entry| border_component_name(&entry.name) || entry.name == "border-image")
}

fn box_components(value: &str) -> Option<[String; 4]> {
    let components = box_shorthand_value_components(value)?;
    match components.as_slice() {
        [single] => Some(std::array::from_fn(|_| single.clone())),
        [vertical, horizontal] => Some([
            vertical.clone(),
            horizontal.clone(),
            vertical.clone(),
            horizontal.clone(),
        ]),
        [top, horizontal, bottom] => Some([
            top.clone(),
            horizontal.clone(),
            bottom.clone(),
            horizontal.clone(),
        ]),
        [top, right, bottom, left] => {
            Some([top.clone(), right.clone(), bottom.clone(), left.clone()])
        }
        _ => None,
    }
}

fn indexed_entry<'a>(
    entries: &'a [CssStyleEntry],
    name: &str,
) -> Option<(usize, &'a CssStyleEntry)> {
    entries
        .iter()
        .enumerate()
        .find(|(_, entry)| entry.name == name)
}

fn border_component_name(name: &str) -> bool {
    matches!(
        name,
        "border-width"
            | "border-style"
            | "border-color"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
    )
}

fn serialize_border_side_entries(entries: &[CssStyleEntry]) -> Option<Vec<CssStyleEntry>> {
    let side_names = ["border-top", "border-right", "border-bottom", "border-left"];
    let mut indexes = Vec::with_capacity(4);
    let mut sides = Vec::with_capacity(4);
    let mut priority = None;
    for side_name in side_names {
        let (index, entry) = entries
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.name == side_name)?;
        if priority.is_some_and(|current| current != entry.priority) {
            return None;
        }
        priority = Some(entry.priority);
        indexes.push(index);
        sides.push(border_components(&entry.value));
    }
    let min_index = indexes.iter().copied().min()?;
    let max_index = indexes.iter().copied().max()?;
    let border_image_index = entries
        .iter()
        .enumerate()
        .find(|(_, entry)| {
            entry.name == "border-image"
                && entry.value.eq_ignore_ascii_case("none")
                && Some(entry.priority) == priority
        })
        .map(|(index, _)| index);
    if entries[min_index..=max_index]
        .iter()
        .any(|entry| !side_names.contains(&entry.name.as_str()))
    {
        return None;
    }
    let priority = priority.unwrap_or(false);
    let sides: [BorderComponents; 4] = sides.try_into().ok()?;
    if border_image_index.is_some() && sides.iter().all(|side| side == &sides[0]) {
        return Some(replace_indexes(
            entries,
            &[(
                min_index,
                CssStyleEntry {
                    name: "border".to_owned(),
                    value: serialize_border_side_value(&sides[0]),
                    priority,
                },
            )],
            &indexes
                .into_iter()
                .chain(border_image_index)
                .collect::<Vec<_>>(),
        ));
    }
    let replacements = border_component_entries(&sides, priority)
        .into_iter()
        .map(|entry| (min_index, entry))
        .collect::<Vec<_>>();
    Some(replace_indexes(entries, &replacements, &indexes))
}

fn border_component_entries(sides: &[BorderComponents; 4], priority: bool) -> Vec<CssStyleEntry> {
    let widths = sides
        .iter()
        .map(|side| side.width.clone())
        .collect::<Vec<_>>();
    let styles = sides
        .iter()
        .map(|side| side.style.clone())
        .collect::<Vec<_>>();
    let colors = sides
        .iter()
        .map(|side| side.color.clone())
        .collect::<Vec<_>>();
    [
        ("border-width", compress_box_components(&widths)),
        ("border-style", compress_box_components(&styles)),
        ("border-color", compress_box_components(&colors)),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value.map(|value| CssStyleEntry {
            name: name.to_owned(),
            value,
            priority,
        })
    })
    .collect()
}

fn border_components(value: &str) -> BorderComponents {
    BorderComponents {
        width: border_shorthand_width(value).unwrap_or_else(|| "medium".to_owned()),
        style: border_shorthand_style(value).unwrap_or_else(|| "none".to_owned()),
        color: border_shorthand_color(value).unwrap_or_else(|| "currentcolor".to_owned()),
    }
}

fn serialize_border_edge_component_entries(
    entries: &[CssStyleEntry],
) -> Option<Vec<CssStyleEntry>> {
    for (shorthand, longhands) in [
        (
            "border-width",
            [
                "border-top-width",
                "border-right-width",
                "border-bottom-width",
                "border-left-width",
            ],
        ),
        (
            "border-style",
            [
                "border-top-style",
                "border-right-style",
                "border-bottom-style",
                "border-left-style",
            ],
        ),
        (
            "border-color",
            [
                "border-top-color",
                "border-right-color",
                "border-bottom-color",
                "border-left-color",
            ],
        ),
    ] {
        if let Some(serialized) =
            serialize_border_edge_component_entry(entries, shorthand, longhands)
        {
            return Some(serialized);
        }
    }
    None
}

fn serialize_border_edge_component_entry(
    entries: &[CssStyleEntry],
    shorthand: &str,
    longhands: [&str; 4],
) -> Option<Vec<CssStyleEntry>> {
    let mut indexes = Vec::with_capacity(4);
    let mut values = Vec::with_capacity(4);
    let mut priority = None;
    for longhand in longhands {
        let (index, entry) = indexed_entry(entries, longhand)?;
        if priority.is_some_and(|current| current != entry.priority) {
            return None;
        }
        priority = Some(entry.priority);
        indexes.push(index);
        values.push(entry.value.clone());
    }
    let min_index = indexes.iter().copied().min()?;
    let max_index = indexes.iter().copied().max()?;
    if entries[min_index..=max_index]
        .iter()
        .any(|entry| !longhands.contains(&entry.name.as_str()))
    {
        return None;
    }
    let priority = priority.unwrap_or(false);
    let replacement = CssStyleEntry {
        name: shorthand.to_owned(),
        value: compress_box_components(&values)?,
        priority,
    };
    Some(replace_indexes(
        entries,
        &[(min_index, replacement)],
        &indexes,
    ))
}

fn serialize_border_side_value(side: &BorderComponents) -> String {
    let mut parts = Vec::new();
    if side.width != "medium" {
        parts.push(side.width.as_str());
    }
    if side.style != "none" {
        parts.push(side.style.as_str());
    }
    if side.color != "currentcolor" {
        parts.push(side.color.as_str());
    }
    parts.join(" ")
}

fn replace_indexes(
    entries: &[CssStyleEntry],
    replacements: &[(usize, CssStyleEntry)],
    consumed: &[usize],
) -> Vec<CssStyleEntry> {
    let mut serialized = Vec::with_capacity(entries.len() + replacements.len());
    for (index, entry) in entries.iter().enumerate() {
        for (_, replacement) in replacements
            .iter()
            .filter(|(replacement_index, _)| *replacement_index == index)
        {
            serialized.push(replacement.clone());
        }
        if !consumed.contains(&index) {
            serialized.push(entry.clone());
        }
    }
    serialized
}

fn serialize_outline_shorthand_entries(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    serialize_fixed_shorthand_entries(
        entries,
        "outline",
        &["outline-color", "outline-style", "outline-width"],
        |values| format!("{} {} {}", values[0], values[1], values[2]),
    )
}

fn serialize_text_decoration_shorthand_entries(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    serialize_fixed_shorthand_entries(
        entries,
        "text-decoration",
        &[
            "text-decoration-line",
            "text-decoration-thickness",
            "text-decoration-style",
            "text-decoration-color",
        ],
        |values| {
            serialize_text_decoration_shorthand_value(
                &values[0], &values[1], &values[2], &values[3],
            )
        },
    )
}

fn serialize_text_decoration_shorthand_value(
    line: &str,
    thickness: &str,
    style: &str,
    color: &str,
) -> String {
    let line = text_decoration_component_or_initial(line, "none");
    let thickness = text_decoration_component_or_initial(thickness, "auto");
    let style = text_decoration_component_or_initial(style, "solid");
    let color = text_decoration_component_or_initial(color, "currentcolor");
    let defaults =
        line == "none" && thickness == "auto" && style == "solid" && color == "currentcolor";

    let mut values = Vec::new();
    if defaults || line != "none" {
        values.push(line);
    }
    if thickness != "auto" {
        values.push(thickness);
    }
    if style != "solid" {
        values.push(style);
    }
    if !color.eq_ignore_ascii_case("currentcolor") {
        values.push(color);
    }
    values.join(" ")
}

fn text_decoration_component_or_initial<'a>(value: &'a str, initial: &'static str) -> &'a str {
    if value.is_empty() { initial } else { value }
}

fn serialize_list_style_shorthand_entries(entries: &[CssStyleEntry]) -> Vec<CssStyleEntry> {
    serialize_fixed_shorthand_entries(
        entries,
        "list-style",
        &["list-style-position", "list-style-type", "list-style-image"],
        |values| {
            let mut parts = Vec::new();
            if !values[0].eq_ignore_ascii_case("outside") {
                parts.push(values[0].as_str());
            }
            if !values[1].eq_ignore_ascii_case("disc") {
                parts.push(values[1].as_str());
            }
            if !values[2].eq_ignore_ascii_case("none") {
                parts.push(values[2].as_str());
            }
            if parts.is_empty() {
                "outside disc".to_owned()
            } else {
                parts.join(" ")
            }
        },
    )
}

fn serialize_fixed_shorthand_entries(
    entries: &[CssStyleEntry],
    shorthand: &str,
    longhands: &[&str],
    value: impl Fn(&[String]) -> String,
) -> Vec<CssStyleEntry> {
    let mut serialized = Vec::with_capacity(entries.len());
    let mut consumed = vec![false; entries.len()];
    for (index, entry) in entries.iter().enumerate() {
        if consumed[index] {
            continue;
        }
        if longhands.contains(&entry.name.as_str())
            && let Some((replacement, indexes)) =
                collect_fixed_shorthand_entry(entries, index, shorthand, longhands, &value)
        {
            for index in indexes {
                consumed[index] = true;
            }
            serialized.push(replacement);
            continue;
        }
        serialized.push(entry.clone());
    }
    serialized
}

fn collect_fixed_shorthand_entry(
    entries: &[CssStyleEntry],
    first_index: usize,
    shorthand: &str,
    longhands: &[&str],
    value: &impl Fn(&[String]) -> String,
) -> Option<(CssStyleEntry, Vec<usize>)> {
    let mut indexes = Vec::with_capacity(longhands.len());
    let mut values = Vec::with_capacity(longhands.len());
    let mut priority = None;
    for longhand in longhands {
        let (index, longhand_entry) = entries
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.name == *longhand)?;
        if index < first_index || priority.is_some_and(|current| current != longhand_entry.priority)
        {
            return None;
        }
        priority = Some(longhand_entry.priority);
        indexes.push(index);
        values.push(longhand_entry.value.clone());
    }
    let min_index = indexes.iter().copied().min()?;
    let max_index = indexes.iter().copied().max()?;
    if entries[min_index..=max_index]
        .iter()
        .any(|entry| !longhands.contains(&entry.name.as_str()))
    {
        return None;
    }
    let shorthand_value = if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let first = values.first()?;
        if !values.iter().all(|value| value == first) {
            return None;
        }
        first.clone()
    } else {
        value(&values)
    };
    Some((
        CssStyleEntry {
            name: shorthand.to_owned(),
            value: shorthand_value,
            priority: priority.unwrap_or(false),
        },
        indexes,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style_entry(name: &str, value: &str) -> CssStyleEntry {
        CssStyleEntry {
            name: name.to_owned(),
            value: value.to_owned(),
            priority: false,
        }
    }

    #[test]
    fn css_declaration_list_parser_preserves_nested_semicolons() {
        let entries = parse_css_declaration_list(
            r#"color: red; content: "a;b"; background-image: url("data:image/svg+xml;a=b");"#,
        );
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1].name, "content");
        assert_eq!(entries[1].value, r#""a;b""#);
        assert_eq!(entries[2].value, r#"url("data:image/svg+xml;a=b")"#);
    }

    #[test]
    fn css_style_serializer_escapes_set_property_semicolons_for_reparse() {
        let serialized = serialize_css_style_entries(&[CssStyleEntry {
            name: "cont".to_owned(),
            value: "Hello; world!".to_owned(),
            priority: false,
        }]);
        let entries = parse_css_declaration_list(&serialized);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "cont");
        assert_eq!(entries[0].value, "Hello; world!");
    }

    #[test]
    fn css_declaration_list_parser_handles_priority_and_invalid_blocks() {
        let entries = parse_css_declaration_list(
            "display: block !important; broken { color: red; } width: 10px;",
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "display");
        assert_eq!(entries[0].value, "block");
        assert!(entries[0].priority);
    }

    #[test]
    fn css_declaration_list_parser_normalizes_zero_lengths_for_cssom() {
        let entries = parse_css_declaration_list("width:0; height: 0; top: 0; left: 0;");
        assert_eq!(
            serialize_css_style_entries(&entries),
            "width: 0px; height: 0px; top: 0px; left: 0px;"
        );
    }

    #[test]
    fn css_declaration_value_normalization_stays_renderer_local() {
        assert_eq!(
            normalize_cssom_declaration_value("width", "0").as_deref(),
            Some("0px")
        );
        assert_eq!(
            normalize_cssom_declaration_value("top", " -0 ").as_deref(),
            Some("0px")
        );
        assert_eq!(
            normalize_cssom_declaration_value("flex", "0").as_deref(),
            Some("0px")
        );
        assert_eq!(normalize_cssom_declaration_value("color", "0"), None);
    }

    #[test]
    fn flex_cssom_value_normalization_stays_renderer_local() {
        assert_eq!(
            normalize_cssom_flex_shorthand_value("0").as_deref(),
            Some("0px")
        );
        assert_eq!(
            normalize_cssom_flex_shorthand_value("initial").as_deref(),
            Some("initial")
        );
        assert_eq!(
            normalize_cssom_flex_basis_value("0").as_deref(),
            Some("0px")
        );
    }

    #[test]
    fn box_shorthand_component_split_stays_renderer_local() {
        assert_eq!(
            box_shorthand_value_components("calc(10px + 5px) var(--gap, 1px 2px)"),
            Some(vec![
                "calc(10px + 5px)".to_owned(),
                "var(--gap, 1px 2px)".to_owned(),
            ])
        );
        assert_eq!(
            box_shorthand_value_components("1px 2px 3px 4px"),
            Some(vec![
                "1px".to_owned(),
                "2px".to_owned(),
                "3px".to_owned(),
                "4px".to_owned(),
            ])
        );
        assert_eq!(
            box_shorthand_value_components("url(http://localhost/) -0px"),
            Some(vec![
                r#"url("http://localhost/")"#.to_owned(),
                "0px".to_owned(),
            ])
        );
        assert_eq!(
            box_shorthand_value_components("env( test 0 1 , green) 1px"),
            Some(vec!["env(test 0 1, green)".to_owned(), "1px".to_owned()])
        );
        assert_eq!(box_shorthand_value_components("env(test -1, green)"), None);
    }

    #[test]
    fn css_style_serializer_preserves_unresolved_box_shorthand_projection() {
        let entries = [
            style_entry("margin", "var(--prop)"),
            style_entry("margin-top", "10px"),
        ];
        assert_eq!(
            serialize_css_style_entries(&entries),
            "margin-right: ; margin-bottom: ; margin-left: ; margin-top: 10px;"
        );

        let entries = [
            style_entry("border-width", "var(--width)"),
            style_entry("border-left-width", "3px"),
        ];
        assert_eq!(
            serialize_css_style_entries(&entries),
            "border-top-width: ; border-right-width: ; border-bottom-width: ; border-left-width: 3px;"
        );
    }

    #[test]
    fn comma_separated_component_split_stays_renderer_local() {
        assert_eq!(
            top_level_comma_separated_component_values(
                "1px 1px menutext, 2px 2px linktext, inset var(--shadow, 1px, 2px)"
            ),
            Some(vec![
                "1px 1px menutext".to_owned(),
                "2px 2px linktext".to_owned(),
                "inset var(--shadow, 1px, 2px)".to_owned(),
            ])
        );
        assert_eq!(
            top_level_comma_separated_component_values(
                " url(http://localhost/) , env( test 0 1 , green) "
            ),
            Some(vec![
                r#"url("http://localhost/")"#.to_owned(),
                "env(test 0 1, green)".to_owned(),
            ])
        );
        assert_eq!(top_level_comma_separated_component_values("1px,"), None);
        assert_eq!(
            top_level_comma_separated_component_values("env(test -1, green)"),
            None
        );
    }

    #[test]
    fn slash_spaced_component_serialization_stays_renderer_local() {
        assert_eq!(
            normalize_cssom_component_value_serialization_with_spaced_slash("10px/1 Ahem")
                .as_deref(),
            Some("10px / 1 Ahem")
        );
        assert_eq!(
            normalize_cssom_component_value_serialization_with_spaced_slash(
                "italic  small-caps  bold  16px /2 \"A B\", serif",
            )
            .as_deref(),
            Some("italic small-caps bold 16px / 2 \"A B\", serif")
        );
        assert_eq!(
            normalize_cssom_component_value_serialization_with_spaced_slash(
                "var(--font/size) / var(--line/height) Ahem",
            )
            .as_deref(),
            Some("var(--font/size) / var(--line/height) Ahem")
        );
        assert_eq!(
            normalize_cssom_component_value_serialization_with_spaced_slash(
                "env( test 0 1 , green)/url(http://localhost/)",
            )
            .as_deref(),
            Some(r#"env(test 0 1, green) / url("http://localhost/")"#)
        );
        assert_eq!(
            normalize_cssom_component_value_serialization_with_spaced_slash("env(test -1, green)"),
            None
        );
    }

    #[test]
    fn background_shorthand_color_projection_stays_renderer_local() {
        assert_eq!(
            background_shorthand_color("url(bg.png) no-repeat green").as_deref(),
            Some("green")
        );
        assert_eq!(
            background_shorthand_color("linear-gradient(red, blue) #0f0").as_deref(),
            Some("#0f0")
        );
        assert_eq!(
            background_shorthand_color("left / cover rgb(1, 2, 3)").as_deref(),
            Some("rgb(1, 2, 3)")
        );
        assert_eq!(background_shorthand_color("url(bg.png) no-repeat"), None);
    }

    #[test]
    fn border_shorthand_component_projection_stays_renderer_local() {
        assert_eq!(
            border_shorthand_width("5px solid red").as_deref(),
            Some("5px")
        );
        assert_eq!(
            border_shorthand_width("solid thick red").as_deref(),
            Some("thick")
        );
        assert_eq!(border_shorthand_width("solid red").as_deref(), None);

        assert_eq!(
            border_shorthand_color("1px solid Menu").as_deref(),
            Some("Menu")
        );
        assert_eq!(
            border_shorthand_color("solid rgb(1, 2, 3)").as_deref(),
            Some("rgb(1, 2, 3)")
        );

        assert_eq!(
            border_shorthand_style("1px solid red").as_deref(),
            Some("solid")
        );
        assert_eq!(border_shorthand_style("DOTTED").as_deref(), Some("dotted"));
        assert_eq!(border_shorthand_style("1px red").as_deref(), None);
    }

    #[test]
    fn system_color_approximation_uses_shared_headless_palette() {
        assert!(ident_is_system_color("MenuText"));
        assert!(ident_is_system_color("visitedtext"));
        assert!(!ident_is_system_color("NotMenuText"));
        assert_eq!(system_color_rgb("MenuText"), Some((0, 0, 0)));
        assert_eq!(system_color_rgb("ActiveBorder"), Some((169, 169, 169)));
        assert_eq!(system_color_rgb("NotMenuText"), None);
    }

    #[test]
    fn mixed_pdb_block_serialization_handles_interleaved_side_entries() {
        let entries = vec![
            style_entry("width", "0"),
            style_entry("-webkit-transform-origin", "20px 30px"),
            style_entry("height", "0"),
        ];
        let side_entries = vec![style_entry("-webkit-transform-origin", "20px 30px")];
        let block = moli_css_parse::parse_declaration_block("width: 0; height: 0;");

        assert_eq!(
            serialize_css_style_entries_with_pdb_block(&entries, &side_entries, &block).as_deref(),
            Some("width: 0px; -webkit-transform-origin: 20px 30px; height: 0px;")
        );
    }

    #[test]
    fn mixed_pdb_block_serialization_uses_stylo_active_entries_across_side_entries() {
        let entries = vec![
            style_entry("color", "red"),
            style_entry("-webkit-transform-origin", "20px 30px"),
            style_entry("color", "blue"),
        ];
        let side_entries = vec![style_entry("-webkit-transform-origin", "20px 30px")];
        let block = moli_css_parse::parse_declaration_block("color: red; color: blue;");

        assert_eq!(
            serialize_css_style_entries_with_pdb_block(&entries, &side_entries, &block).as_deref(),
            Some("-webkit-transform-origin: 20px 30px; color: blue;")
        );
    }

    #[test]
    fn mixed_pdb_block_serialization_uses_cssom_mutation_projection() {
        crate::style_engine::ensure_stylo_browser_compat_prefs();
        let mut block = moli_css_parse::CssDeclarationBlock::default();
        block.set_property_with_projection("link-parameters", "param(--a", false);
        block.set_property_with_projection("display", "block", false);
        let entries = vec![
            style_entry("link-parameters", "param(--a"),
            style_entry("-webkit-transform-origin", "20px 30px"),
            style_entry("display", "block"),
        ];
        let side_entries = vec![style_entry("-webkit-transform-origin", "20px 30px")];

        assert_eq!(
            serialize_css_style_entries_with_pdb_block(&entries, &side_entries, &block).as_deref(),
            Some(
                "link-parameters: param(--a); -webkit-transform-origin: 20px 30px; display: block;"
            )
        );
    }

    #[test]
    fn mixed_pdb_block_serialization_replays_pdb_runs_through_cssom_mutation() {
        let mut opacity = style_entry("opacity", "0.5");
        opacity.priority = true;
        let entries = vec![
            style_entry("width", "0"),
            style_entry("-webkit-transform-origin", "20px 30px"),
            opacity,
        ];
        let side_entries = vec![style_entry("-webkit-transform-origin", "20px 30px")];
        let block = moli_css_parse::parse_declaration_block("width: 0; opacity: 0.5 !important;");

        assert_eq!(
            serialize_css_style_entries_with_pdb_block(&entries, &side_entries, &block).as_deref(),
            Some("width: 0px; -webkit-transform-origin: 20px 30px; opacity: 0.5 !important;")
        );
    }

    #[test]
    fn mixed_pdb_block_serialization_projects_border_side_longhands() {
        let mut border_top_width = style_entry("border-top-width", "4px");
        border_top_width.priority = true;
        let mut border_top_style = style_entry("border-top-style", "dashed");
        border_top_style.priority = true;
        let mut border_top_color = style_entry("border-top-color", "green");
        border_top_color.priority = true;
        let entries = vec![
            style_entry("--token", "value"),
            style_entry("-webkit-transform-origin", "20px 30px"),
            border_top_width,
            border_top_style,
            border_top_color,
        ];
        let side_entries = vec![style_entry("-webkit-transform-origin", "20px 30px")];
        let mut block = moli_css_parse::CssDeclarationBlock::default();
        block.set_property_with_projection("--token", "value", false);
        block.set_property_with_projection("border-top", "4px dashed green", true);

        let css_text = serialize_css_style_entries_with_pdb_block(&entries, &side_entries, &block)
            .expect("mixed PDB state should serialize");
        assert!(css_text.contains("--token: value;"), "{css_text}");
        assert!(
            css_text.contains("-webkit-transform-origin: 20px 30px;"),
            "{css_text}"
        );
        assert!(
            css_text.contains("border-top: 4px dashed green !important;"),
            "{css_text}"
        );
    }

    #[test]
    fn mixed_inline_state_property_names_use_stored_pdb_active_entries() {
        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block("display: block; display: flex;"),
            entries: vec![
                style_entry("display", "block"),
                style_entry("-webkit-transform-origin", "20px 30px"),
                style_entry("display", "flex"),
            ],
            side_entries: vec![style_entry("-webkit-transform-origin", "20px 30px")],
        };

        assert_eq!(
            state
                .entries()
                .into_iter()
                .map(|entry| (entry.name, entry.value))
                .collect::<Vec<_>>(),
            [
                (
                    "-webkit-transform-origin".to_owned(),
                    "20px 30px".to_owned()
                ),
                ("display".to_owned(), "flex".to_owned())
            ]
        );
        assert_eq!(
            state.property_names(),
            ["-webkit-transform-origin", "display"]
        );
        assert_eq!(
            state.css_text(),
            "-webkit-transform-origin: 20px 30px; display: flex;"
        );
    }

    #[test]
    fn mixed_inline_state_property_names_preserve_side_entry_positions() {
        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block("width: 0; height: 0;"),
            entries: vec![
                style_entry("width", "0"),
                style_entry("-webkit-transform-origin", "20px 30px"),
                style_entry("height", "0"),
            ],
            side_entries: vec![style_entry("-webkit-transform-origin", "20px 30px")],
        };

        assert_eq!(
            state
                .entries()
                .into_iter()
                .map(|entry| (entry.name, entry.value))
                .collect::<Vec<_>>(),
            [
                ("width".to_owned(), "0px".to_owned()),
                (
                    "-webkit-transform-origin".to_owned(),
                    "20px 30px".to_owned()
                ),
                ("height".to_owned(), "0px".to_owned())
            ]
        );
        assert_eq!(
            state.property_names(),
            ["width", "-webkit-transform-origin", "height"]
        );
        assert_eq!(
            state.css_text(),
            "width: 0px; -webkit-transform-origin: 20px 30px; height: 0px;"
        );
    }

    #[test]
    fn mixed_inline_state_entries_include_pdb_block_after_custom_property_projection() {
        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(
                "transition: display 3s ease-in-out 1s allow-discrete, opacity !important; --token: value;",
            ),
            entries: vec![
                style_entry("--token", "value"),
                style_entry("-webkit-transform-origin", "20px 30px"),
            ],
            side_entries: vec![style_entry("-webkit-transform-origin", "20px 30px")],
        };

        let entries = state
            .entries()
            .into_iter()
            .map(|entry| (entry.name, entry.value, entry.priority))
            .collect::<Vec<_>>();
        assert!(
            entries
                .iter()
                .any(|(name, value, priority)| name == "transition-property"
                    && value == "display, opacity"
                    && *priority),
            "{entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|(name, value, priority)| name == "transition-duration"
                    && value == "3s, 0s"
                    && *priority),
            "{entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|(name, value, priority)| name == "--token" && value == "value" && !*priority),
            "{entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|(name, value, priority)| name == "-webkit-transform-origin"
                    && value == "20px 30px"
                    && !*priority),
            "{entries:?}"
        );
        assert_eq!(
            state.css_text(),
            "transition: display 3s ease-in-out 1s allow-discrete, opacity !important; --token: value; -webkit-transform-origin: 20px 30px;"
        );
    }

    #[test]
    fn inline_state_no_side_entries_are_pdb_views_not_cached_truth() {
        let mut state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block("display: block; opacity: 0.5;"),
            entries: vec![style_entry("display", "inline")],
            side_entries: Vec::new(),
        };

        state.refresh_pdb_entries();

        assert!(state.entries.is_empty());
        assert_eq!(state.property_names(), ["display", "opacity"]);
        assert_eq!(
            state
                .entries()
                .into_iter()
                .map(|entry| (entry.name, entry.value))
                .collect::<Vec<_>>(),
            [
                ("display".to_owned(), "block".to_owned()),
                ("opacity".to_owned(), "0.5".to_owned())
            ]
        );
        assert_eq!(state.css_text(), "display: block; opacity: 0.5;");
    }

    #[test]
    fn inline_state_all_keeps_single_cssom_adapter_entry() {
        let mut state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block("all: inherit;"),
            entries: vec![style_entry("all", "inherit")],
            side_entries: Vec::new(),
        };

        state.refresh_pdb_entries();

        assert_eq!(state.property_names(), ["all"]);
        assert_eq!(
            state
                .entries()
                .into_iter()
                .map(|entry| (entry.name, entry.value))
                .collect::<Vec<_>>(),
            [("all".to_owned(), "inherit".to_owned())]
        );
        assert_eq!(state.css_text(), "all: inherit;");
    }

    #[test]
    fn css_font_face_parser_uses_cssparser_rule_boundaries() {
        let entries = parse_css_font_faces(
            r#"
            .ignored { content: "@font-face { font-family: Bad; src: url(bad.woff2); }"; }
            @font-face {
                font-family: "A; B";
                src: url("data:font/woff2;base64;a;b");
            }
            @FONT-FACE {
                font-family: CaseFace;
                src: local("Case Face");
            }
            "#,
        );

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].family, "A; B");
        assert_eq!(entries[0].source, r#"url("data:font/woff2;base64;a;b")"#);
        assert_eq!(entries[1].family, "CaseFace");
        assert_eq!(entries[1].source, r#"local("Case Face")"#);
    }

    #[test]
    fn css_font_face_parser_filters_invalid_and_incomplete_faces() {
        let entries = parse_css_font_faces(
            r#"
            @font-face { font-family: serif; src: url(generic.woff2); }
            @font-face { font-family: MissingSource; }
            @font-face { src: url(missing-family.woff2); }
            @font-face { font-family: Valid; src: url(valid.woff2); }
            "#,
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].family, "Valid");
        assert_eq!(entries[0].source, r#"url("valid.woff2")"#);
    }

    #[test]
    fn css_style_serializer_respects_logical_margin_group_boundaries() {
        let entries = [
            "margin-top",
            "margin-right",
            "margin-bottom",
            "margin-left",
            "margin-inline-start",
            "margin-inline-end",
            "margin-block-start",
            "margin-block-end",
            "margin-inline-end",
            "margin-bottom",
        ]
        .into_iter()
        .map(|name| CssStyleEntry {
            name: name.to_owned(),
            value: "10px".to_owned(),
            priority: false,
        })
        .collect::<Vec<_>>();
        assert_eq!(
            serialize_css_style_entries(&entries),
            "margin-top: 10px; margin-right: 10px; margin-left: 10px; margin-inline-start: 10px; margin-block: 10px; margin-inline-end: 10px; margin-bottom: 10px;"
        );

        let entries = [
            "margin-top",
            "margin-left",
            "margin-right",
            "margin-bottom",
            "margin-inline-start",
            "margin-inline-end",
            "margin-block-start",
            "margin-block-end",
        ]
        .into_iter()
        .map(|name| CssStyleEntry {
            name: name.to_owned(),
            value: "10px".to_owned(),
            priority: false,
        })
        .collect::<Vec<_>>();
        assert_eq!(
            serialize_css_style_entries(&entries),
            "margin: 10px; margin-inline: 10px; margin-block: 10px;"
        );
    }

    #[test]
    fn css_style_serializer_combines_fixed_shorthand_longhands() {
        let entries = vec![
            CssStyleEntry {
                name: "outline-width".to_owned(),
                value: "2px".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "outline-style".to_owned(),
                value: "dotted".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "outline-color".to_owned(),
                value: "blue".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "list-style-type".to_owned(),
                value: "circle".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "list-style-position".to_owned(),
                value: "inside".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "list-style-image".to_owned(),
                value: "none".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "overscroll-behavior-x".to_owned(),
                value: "contain".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "overscroll-behavior-y".to_owned(),
                value: "contain".to_owned(),
                priority: false,
            },
        ];

        assert_eq!(
            serialize_css_style_entries(&entries),
            "outline: blue dotted 2px; list-style: inside circle; overscroll-behavior: contain;"
        );
    }

    #[test]
    fn css_style_serializer_combines_padding_longhands() {
        let entries = vec![
            CssStyleEntry {
                name: "padding-top".to_owned(),
                value: "1px".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "padding-right".to_owned(),
                value: "2px".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "padding-bottom".to_owned(),
                value: "3px".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "padding-left".to_owned(),
                value: "4px".to_owned(),
                priority: false,
            },
        ];

        assert_eq!(
            serialize_css_style_entries(&entries),
            "padding: 1px 2px 3px 4px;"
        );
    }

    #[test]
    fn css_style_serializer_canonicalizes_border_shorthand_cases() {
        let cases = [
            ("border: 1px; border-top: 1px;", "border: 1px;"),
            (
                "border-top: 1px; border-right: 1px; border-bottom: 1px; border-left: 1px; border-image: none;",
                "border: 1px;",
            ),
            (
                "border-top: 1px; border-right: 2px; border-bottom: 3px; border-left: 4px;",
                "border-width: 1px 2px 3px 4px; border-style: none; border-color: currentcolor;",
            ),
            (
                "border-top-width: 1px; border-top-style: none; border-top-color: currentcolor; border-right-width: 1px; border-right-style: none; border-right-color: currentcolor; border-bottom-width: 1px; border-bottom-style: none; border-bottom-color: currentcolor; border-left-width: 1px; border-left-style: none; border-left-color: currentcolor; border-image: none;",
                "border: 1px;",
            ),
            (
                "border-top-width: 1px; border-right-width: 1px; border-bottom-width: 1px; border-left-width: 1px; border-top-style: none; border-right-style: none; border-bottom-style: none; border-left-style: none; border-top-color: currentcolor; border-right-color: currentcolor; border-bottom-color: currentcolor; border-left-color: currentcolor; border-image: none;",
                "border: 1px;",
            ),
            (
                "border-width: 1px; border-top-style: none; border-right-style: none; border-bottom-style: none; border-left-style: none; border-top-color: currentcolor; border-right-color: currentcolor; border-bottom-color: currentcolor; border-left-color: currentcolor; border-image: none;",
                "border: 1px;",
            ),
            (
                "border-width: 1px; border-top-style: none; border-right-style: none; border-bottom-style: none; border-left-style: none; border-top-color: currentcolor; border-right-color: currentcolor; border-bottom-color: currentcolor; border-left-color: currentcolor;",
                "border-width: 1px; border-style: none; border-color: currentcolor;",
            ),
            (
                "border: 1px; border-top: 2px;",
                "border-width: 2px 1px 1px; border-style: none; border-color: currentcolor; border-image: none;",
            ),
            (
                "border-top-width: 1px; border-top-style: none; border-top-color: red; border-right-width: 1px; border-right-style: none; border-right-color: currentcolor; border-bottom-width: 1px; border-bottom-style: none; border-bottom-color: currentcolor; border-left-width: 1px; border-left-style: none; border-left-color: currentcolor;",
                "border-width: 1px; border-style: none; border-color: red currentcolor currentcolor;",
            ),
            (
                "border-width: 1px; border-top-style: none; border-right-style: none; border-bottom-style: none; border-left-style: none; border-top-color: red; border-right-color: currentcolor; border-bottom-color: currentcolor; border-left-color: currentcolor;",
                "border-width: 1px; border-style: none; border-color: red currentcolor currentcolor;",
            ),
            (
                "border: 1px; border-top: 1px !important;",
                "border-right: 1px; border-bottom: 1px; border-left: 1px; border-image: none; border-top: 1px !important;",
            ),
            (
                "border: 1px; border-top-color: red;",
                "border-width: 1px; border-style: none; border-color: red currentcolor currentcolor; border-image: none;",
            ),
            ("border: solid; border-style: dotted;", "border: dotted;"),
        ];

        for (input, expected) in cases {
            assert_eq!(
                serialize_css_style_entries(&parse_css_declaration_list(input)),
                expected
            );
        }
    }

    #[test]
    fn css_style_serializer_keeps_important_longhand_over_later_normal_longhand() {
        let entries = vec![
            CssStyleEntry {
                name: "padding-left".to_owned(),
                value: "10px".to_owned(),
                priority: true,
            },
            CssStyleEntry {
                name: "padding-left".to_owned(),
                value: "20px".to_owned(),
                priority: false,
            },
        ];

        assert_eq!(
            serialize_css_style_entries(&entries),
            "padding-left: 10px !important; padding-left: 20px;"
        );
    }

    #[test]
    fn css_style_serializer_expands_flex_css_wide_shorthand_when_longhands_override() {
        let entries = vec![
            CssStyleEntry {
                name: "flex".to_owned(),
                value: "initial".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "flex-basis".to_owned(),
                value: "0px".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "flex-shrink".to_owned(),
                value: "2".to_owned(),
                priority: false,
            },
        ];

        assert_eq!(
            serialize_css_style_entries(&entries),
            "flex-grow: initial; flex-basis: 0px; flex-shrink: 2;"
        );
    }

    #[test]
    fn css_style_serializer_drops_redundant_flex_css_wide_longhands() {
        let entries = vec![
            CssStyleEntry {
                name: "flex".to_owned(),
                value: "initial".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "flex-basis".to_owned(),
                value: "initial".to_owned(),
                priority: false,
            },
            CssStyleEntry {
                name: "flex-shrink".to_owned(),
                value: "initial".to_owned(),
                priority: false,
            },
        ];

        assert_eq!(serialize_css_style_entries(&entries), "flex: initial;");
    }
}
